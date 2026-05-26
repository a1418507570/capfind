//! Minimal `.capfind/config.toml` loader.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use capfind_search::ScorerConfig;

#[derive(Debug, Clone, Default)]
pub struct CapfindConfig {
    pub index: IndexConfig,
    pub search: ScorerConfig,
    pub ownership: OwnershipConfig,
    pub services: BTreeMap<String, ServiceConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexConfig {
    pub roots: Vec<String>,
    pub include: Vec<String>,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            roots: vec![".".to_string()],
            include: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnershipConfig {
    pub modules: BTreeMap<String, String>,
    pub packages: BTreeMap<String, String>,
    pub external: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceConfig {
    pub name: Option<String>,
    pub module: Option<String>,
    pub package: Option<String>,
    pub owner: Option<String>,
    pub tier: Option<String>,
    pub slo: Option<String>,
    pub runbook: Option<String>,
    pub tags: Vec<String>,
}

pub fn load(repo_root: &Path) -> Result<CapfindConfig> {
    let path = repo_root.join(".capfind/config.toml");
    if !path.exists() {
        return Ok(CapfindConfig::default());
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    parse(&content).with_context(|| format!("Invalid {}", path.display()))
}

pub(crate) fn parse(content: &str) -> Result<CapfindConfig> {
    let mut cfg = CapfindConfig::default();
    let mut section = String::new();

    for (line_no, raw) in content.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        if let Some(service_id) = service_section_key(&section) {
            let service = cfg.services.entry(service_id).or_default();
            match key {
                "name" => {
                    service.name = Some(parse_string_value("services.name", value, line_no + 1)?)
                }
                "module" => {
                    service.module =
                        Some(parse_string_value("services.module", value, line_no + 1)?)
                }
                "package" => {
                    service.package =
                        Some(parse_string_value("services.package", value, line_no + 1)?)
                }
                "owner" => {
                    service.owner = Some(parse_string_value("services.owner", value, line_no + 1)?)
                }
                "tier" => {
                    service.tier = Some(parse_string_value("services.tier", value, line_no + 1)?)
                }
                "slo" => {
                    service.slo = Some(parse_string_value("services.slo", value, line_no + 1)?)
                }
                "runbook" => {
                    service.runbook =
                        Some(parse_string_value("services.runbook", value, line_no + 1)?)
                }
                "tags" => service.tags = parse_string_array("services.tags", value, line_no + 1)?,
                _ => {}
            }
            continue;
        }

        match section.as_str() {
            "index" => match key {
                "roots" => cfg.index.roots = parse_string_array("index.roots", value, line_no + 1)?,
                "include" => {
                    cfg.index.include = parse_string_array("index.include", value, line_no + 1)?
                }
                _ => {}
            },
            "search" => match key {
                "k1" => cfg.search.k1 = parse_positive_f32("search.k1", value, line_no + 1)?,
                "b" => cfg.search.b = parse_b(value, line_no + 1)?,
                _ => {}
            },
            "ownership.modules" => {
                cfg.ownership.modules.insert(
                    parse_map_key(key),
                    parse_string_value("ownership.modules", value, line_no + 1)?,
                );
            }
            "ownership.packages" => {
                cfg.ownership.packages.insert(
                    parse_map_key(key),
                    parse_string_value("ownership.packages", value, line_no + 1)?,
                );
            }
            "ownership.external" => {
                cfg.ownership.external.insert(
                    parse_map_key(key),
                    parse_string_value("ownership.external", value, line_no + 1)?,
                );
            }
            _ => {}
        }
    }

    Ok(cfg)
}

fn service_section_key(section: &str) -> Option<String> {
    section.strip_prefix("services.").and_then(|key| {
        let key = parse_map_key(key);
        (!key.trim().is_empty()).then_some(key)
    })
}

fn parse_map_key(key: &str) -> String {
    let trimmed = key.trim();
    let Some(quote) = trimmed
        .chars()
        .next()
        .filter(|ch| (*ch == '"' || *ch == '\'') && trimmed.ends_with(*ch))
    else {
        return trimmed.to_string();
    };
    unescape_basic(
        &trimmed[quote.len_utf8()..trimmed.len() - quote.len_utf8()],
        quote,
    )
}

fn parse_string_value(name: &str, value: &str, line_no: usize) -> Result<String> {
    let value = value.trim();
    let quote = value
        .chars()
        .next()
        .filter(|ch| (*ch == '"' || *ch == '\'') && value.ends_with(*ch))
        .with_context(|| format!("{name} at line {line_no} must be a string"))?;
    Ok(unescape_basic(
        &value[quote.len_utf8()..value.len() - quote.len_utf8()],
        quote,
    ))
}

fn parse_string_array(name: &str, value: &str, line_no: usize) -> Result<Vec<String>> {
    let value = value.trim();
    if !value.starts_with('[') || !value.ends_with(']') {
        bail!("{name} at line {line_no} must be an array of strings");
    }

    let mut items = Vec::new();
    let mut rest = value[1..value.len() - 1].trim();
    while !rest.is_empty() {
        let quote = rest
            .chars()
            .next()
            .filter(|ch| *ch == '"' || *ch == '\'')
            .with_context(|| format!("{name} at line {line_no} must contain only strings"))?;

        let mut close_idx = None;
        let mut escaped = false;
        for (idx, ch) in rest.char_indices().skip(1) {
            if escaped {
                escaped = false;
                continue;
            }
            if quote == '"' && ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                close_idx = Some(idx);
                break;
            }
        }
        let close_idx = close_idx
            .with_context(|| format!("{name} at line {line_no} has an unterminated string"))?;
        let item = &rest[quote.len_utf8()..close_idx];
        items.push(unescape_basic(item, quote));

        rest = rest[close_idx + quote.len_utf8()..].trim_start();
        if rest.is_empty() {
            break;
        }
        if !rest.starts_with(',') {
            bail!("{name} at line {line_no} must separate strings with commas");
        }
        rest = rest[1..].trim_start();
    }

    Ok(items)
}

fn unescape_basic(input: &str, quote: char) -> String {
    if quote != '"' || !input.contains('\\') {
        return input.to_string();
    }

    let mut out = String::new();
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn parse_positive_f32(name: &str, value: &str, line_no: usize) -> Result<f32> {
    let parsed = value
        .parse::<f32>()
        .with_context(|| format!("{name} at line {line_no} must be a number"))?;
    if !parsed.is_finite() || parsed <= 0.0 {
        bail!("{name} at line {line_no} must be > 0");
    }
    Ok(parsed)
}

fn parse_b(value: &str, line_no: usize) -> Result<f32> {
    let parsed = value
        .parse::<f32>()
        .with_context(|| format!("search.b at line {line_no} must be a number"))?;
    if !parsed.is_finite() || !(0.0..=1.0).contains(&parsed) {
        bail!("search.b at line {line_no} must be between 0 and 1");
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_values_use_defaults() {
        let cfg = parse("version = 1\n[index]\n").unwrap();
        assert_eq!(cfg.index.roots, vec!["."]);
        assert!(cfg.index.include.is_empty());
        assert_eq!(cfg.search.k1, ScorerConfig::default().k1);
        assert_eq!(cfg.search.b, ScorerConfig::default().b);
    }

    #[test]
    fn parses_index_roots_and_includes() {
        let cfg = parse(
            r#"
version = 1

[index]
roots = ["services/api", 'libs/common']
include = ["**/*.java", "**/*.go"]
"#,
        )
        .unwrap();
        assert_eq!(cfg.index.roots, vec!["services/api", "libs/common"]);
        assert_eq!(cfg.index.include, vec!["**/*.java", "**/*.go"]);
    }

    #[test]
    fn parses_search_scoring_params() {
        let cfg = parse(
            r#"
version = 1

[search]
k1 = 1.7
b = 0.2
"#,
        )
        .unwrap();
        assert_eq!(cfg.search.k1, 1.7);
        assert_eq!(cfg.search.b, 0.2);
    }

    #[test]
    fn parses_ownership_maps() {
        let cfg = parse(
            r#"
[ownership.modules]
"apps/order-api" = "Order API Team"

[ownership.packages]
"com.acme.settlement" = "Settlement Team"

[ownership.external]
"com.acme.pricing" = "Pricing Platform"
"com.fasterxml.jackson.core" = "Open Source Runtime"
"#,
        )
        .unwrap();
        assert_eq!(
            cfg.ownership.modules.get("apps/order-api").unwrap(),
            "Order API Team"
        );
        assert_eq!(
            cfg.ownership.packages.get("com.acme.settlement").unwrap(),
            "Settlement Team"
        );
        assert_eq!(
            cfg.ownership.external.get("com.acme.pricing").unwrap(),
            "Pricing Platform"
        );
        assert_eq!(
            cfg.ownership
                .external
                .get("com.fasterxml.jackson.core")
                .unwrap(),
            "Open Source Runtime"
        );
    }

    #[test]
    fn parses_service_registry() {
        let cfg = parse(
            r#"
[services."order-api"]
name = "Order API"
module = "apps/order-api"
package = "com.acme.order"
owner = "Order Experience"
tier = "gold"
slo = "99.9%"
runbook = "docs/runbooks/order-api.md"
tags = ["customer-facing", "checkout"]

[services.payment-core]
module = "services/payment-core"
owner = "Payments Platform"
"#,
        )
        .unwrap();
        let order = cfg.services.get("order-api").unwrap();
        assert_eq!(order.name.as_deref(), Some("Order API"));
        assert_eq!(order.module.as_deref(), Some("apps/order-api"));
        assert_eq!(order.package.as_deref(), Some("com.acme.order"));
        assert_eq!(order.owner.as_deref(), Some("Order Experience"));
        assert_eq!(order.tier.as_deref(), Some("gold"));
        assert_eq!(order.slo.as_deref(), Some("99.9%"));
        assert_eq!(order.runbook.as_deref(), Some("docs/runbooks/order-api.md"));
        assert_eq!(order.tags, vec!["customer-facing", "checkout"]);
        assert_eq!(
            cfg.services.get("payment-core").unwrap().owner.as_deref(),
            Some("Payments Platform")
        );
    }

    #[test]
    fn rejects_invalid_search_params() {
        assert!(parse("[search]\nk1 = 0\n").is_err());
        assert!(parse("[search]\nb = 1.5\n").is_err());
    }
}
