//! Minimal `.capfind/config.toml` loader.

use std::path::Path;

use anyhow::{bail, Context, Result};
use capfind_search::ScorerConfig;

#[derive(Debug, Clone)]
pub struct CapfindConfig {
    pub search: ScorerConfig,
}

impl Default for CapfindConfig {
    fn default() -> Self {
        Self {
            search: ScorerConfig::default(),
        }
    }
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
        let value = value.trim().trim_matches('"');

        if section == "search" {
            match key {
                "k1" => cfg.search.k1 = parse_positive_f32("search.k1", value, line_no + 1)?,
                "b" => cfg.search.b = parse_b(value, line_no + 1)?,
                _ => {}
            }
        }
    }

    Ok(cfg)
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
        assert_eq!(cfg.search.k1, ScorerConfig::default().k1);
        assert_eq!(cfg.search.b, ScorerConfig::default().b);
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
    fn rejects_invalid_search_params() {
        assert!(parse("[search]\nk1 = 0\n").is_err());
        assert!(parse("[search]\nb = 1.5\n").is_err());
    }
}
