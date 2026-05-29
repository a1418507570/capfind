//! CLI subcommand implementations.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use capfind_core::{load_index, write_index, Capability, IndexBody, IndexHeader, Kind, Lang};
use capfind_search::{search, synonyms, tokenize as tokenize_text, Hit};

use crate::adoption;
use crate::asset_map as code_map;
use crate::config;
use crate::context as ai_context;
use crate::eval as quality_eval;
use crate::file_diagnosis;
use crate::fmt as output;
use crate::indexer;
use crate::{
    AdoptionStage, AgentArgs, ContextArgs, DashboardArgs, DetectAdoptionArgs, DiagnoseFileArgs,
    DiagnoseQueryArgs, DoctorArgs, EvalArgs, FindArgs, InitArgs, KindFilter, LangFilter, MapArgs,
    McpArgs, RecordAdoptionArgs,
};

const DASHBOARD_HISTORY_FILE: &str = ".capfind/dashboard-history.jsonl";
const DASHBOARD_HISTORY_SCHEMA_VERSION: &str = "capfind.dashboard_history.v1";
const DASHBOARD_HISTORY_LIMIT: usize = 20;
const QUERY_HISTORY_FILE: &str = ".capfind/query-history.jsonl";
const QUERY_EVENT_SCHEMA_VERSION: &str = "capfind.query_event.v1";
const QUERY_HISTORY_SCHEMA_VERSION: &str = "capfind.query_history.v1";
const QUERY_HISTORY_LIMIT: usize = 20;
const EVAL_HISTORY_FILE: &str = ".capfind/eval-history.jsonl";
const EVAL_HISTORY_SCHEMA_VERSION: &str = "capfind.eval_history.v1";
const EVAL_HISTORY_LIMIT: usize = 20;

/// `capfind init` — create .capfind/ directory with config.
pub fn init(repo_root: &Path, args: &InitArgs) -> Result<()> {
    let dir = repo_root.join(".capfind");
    std::fs::create_dir_all(&dir)?;

    let config_path = dir.join("config.toml");
    if !config_path.exists() {
        std::fs::write(&config_path, DEFAULT_CONFIG)?;
        println!("Created {}", config_path.display());
    }

    let capfindignore_path = repo_root.join(".capfindignore");
    if !capfindignore_path.exists() {
        std::fs::write(&capfindignore_path, DEFAULT_CAPFINDIGNORE)?;
        println!("Created {}", capfindignore_path.display());
    }

    // Append to .gitignore if not already there.
    let gitignore = repo_root.join(".gitignore");
    let entry = ".capfind/";
    let already = gitignore.exists()
        && std::fs::read_to_string(&gitignore)
            .map(|c| c.contains(entry))
            .unwrap_or(false);
    if !already {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&gitignore)?;
        writeln!(
            f,
            "\n# capfind index (local cache, do not commit)\n{}",
            entry
        )?;
        println!("Appended {} to .gitignore", entry);
    }

    if args.product_config {
        generate_product_configs(repo_root, args.force)?;
    }

    println!("Initialized .capfind/ at {}", repo_root.display());
    Ok(())
}

fn generate_product_configs(repo_root: &Path, force: bool) -> Result<()> {
    let inventory = product_config_inventory(repo_root)?;
    let generated = vec![
        (
            repo_root.join(".capfind/config.suggested.toml"),
            suggested_config_toml(&inventory),
        ),
        (
            repo_root.join(".capfind/integrations/mcp.generic.json"),
            generic_mcp_config(repo_root)?,
        ),
        (
            repo_root.join(".capfind/integrations/agent-rules.md"),
            agent_rules_template(),
        ),
        (
            repo_root.join(".capfind/integrations/mcp-client.md"),
            mcp_client_template(repo_root),
        ),
    ];

    for (path, content) in generated {
        let wrote = write_generated_file(&path, &content, force)?;
        if wrote {
            println!("Created {}", path.display());
        } else {
            println!(
                "Kept existing {} (use --force to overwrite)",
                path.display()
            );
        }
    }
    println!(
        "Product config scan: {} supported files{}",
        inventory.supported_files,
        if inventory.truncated {
            " (truncated)"
        } else {
            ""
        }
    );
    Ok(())
}

#[derive(Default)]
struct ProductConfigInventory {
    supported_files: u64,
    truncated: bool,
    has_java: bool,
    has_go: bool,
    has_proto: bool,
    has_xml: bool,
    has_maven: bool,
    has_gradle: bool,
    has_jar: bool,
    modules: BTreeMap<String, u64>,
    packages: BTreeMap<String, u64>,
    external_prefixes: BTreeMap<String, u64>,
}

fn product_config_inventory(repo_root: &Path) -> Result<ProductConfigInventory> {
    const PRODUCT_CONFIG_SCAN_LIMIT: u64 = 12_000;

    let mut inventory = ProductConfigInventory::default();
    let capfind_ignore = indexer::load_capfindignore(repo_root)?;
    let mut builder = ignore::WalkBuilder::new(repo_root);
    let root_for_filter = repo_root.to_path_buf();
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .filter_entry(move |entry| {
            let path = entry.path();
            if path == root_for_filter {
                return true;
            }
            let is_dir = entry
                .file_type()
                .is_some_and(|file_type| file_type.is_dir());
            !is_product_config_skip_dir(path)
                && !indexer::is_capfind_ignored(capfind_ignore.as_ref(), path, is_dir)
        });
    let walker = builder.build();

    for entry in walker {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        if !indexer::is_supported_index_input(path) || indexer::is_skipped_source_path(path) {
            continue;
        }

        inventory.supported_files += 1;
        if inventory.supported_files > PRODUCT_CONFIG_SCAN_LIMIT {
            inventory.truncated = true;
            break;
        }

        let rel_path = path
            .strip_prefix(repo_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if let Some(module) = suggested_module(&rel_path) {
            *inventory.modules.entry(module).or_default() += 1;
        }

        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        match (file_name, ext) {
            (_, "java") => {
                inventory.has_java = true;
                if let Some(package) = read_java_package(path)? {
                    if let Some(prefix) = package_prefix(&package) {
                        *inventory.packages.entry(prefix).or_default() += 1;
                    }
                }
            }
            (_, "go") => inventory.has_go = true,
            (_, "proto") => inventory.has_proto = true,
            (_, "xml") => inventory.has_xml = true,
            (_, "jar") => inventory.has_jar = true,
            ("pom.xml", _) => {
                inventory.has_maven = true;
                collect_external_prefixes(path, &mut inventory.external_prefixes)?;
            }
            ("build.gradle" | "build.gradle.kts", _) => {
                inventory.has_gradle = true;
                collect_external_prefixes(path, &mut inventory.external_prefixes)?;
            }
            _ => {}
        }
    }

    Ok(inventory)
}

fn is_product_config_skip_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                ".capfind"
                    | "target"
                    | "build"
                    | "node_modules"
                    | "vendor"
                    | ".gradle"
                    | "generated"
            )
        })
}

fn suggested_module(rel_path: &str) -> Option<String> {
    let parts = rel_path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() >= 3 {
        Some(format!("{}/{}", parts[0], parts[1]))
    } else if parts.len() >= 2 {
        Some(parts[0].to_string())
    } else {
        None
    }
}

fn read_java_package(path: &Path) -> Result<Option<String>> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    Ok(content.lines().take(20).find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix("package ")
            .map(|package| package.trim_end_matches(';').trim().to_string())
            .filter(|package| !package.is_empty())
    }))
}

fn package_prefix(package: &str) -> Option<String> {
    let parts = package
        .split('.')
        .filter(|part| !part.is_empty())
        .take(3)
        .collect::<Vec<_>>();
    if parts.len() >= 2 {
        Some(parts.join("."))
    } else {
        None
    }
}

fn collect_external_prefixes(path: &Path, prefixes: &mut BTreeMap<String, u64>) -> Result<()> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    for line in content.lines() {
        if let Some(group) = xml_value(line, "groupId") {
            record_external_prefix(prefixes, group);
        }
        if let Some(group) = quoted_dependency_group(line) {
            record_external_prefix(prefixes, group);
        }
    }
    Ok(())
}

fn xml_value<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let start_tag = format!("<{tag}>");
    let end_tag = format!("</{tag}>");
    let start = line.find(&start_tag)? + start_tag.len();
    let end = line[start..].find(&end_tag)? + start;
    Some(line[start..end].trim()).filter(|value| !value.is_empty())
}

fn quoted_dependency_group(line: &str) -> Option<&str> {
    let quote_idx = line.find(['"', '\''])?;
    let quote = line.as_bytes()[quote_idx] as char;
    let rest = &line[quote_idx + 1..];
    let end = rest.find(quote)?;
    let coordinate = &rest[..end];
    let (group, tail) = coordinate.split_once(':')?;
    if tail.contains(':') {
        Some(group.trim()).filter(|value| !value.is_empty())
    } else {
        None
    }
}

fn record_external_prefix(prefixes: &mut BTreeMap<String, u64>, value: &str) {
    let value = value.trim();
    if value.is_empty() || value.starts_with("${") {
        return;
    }
    *prefixes.entry(value.to_string()).or_default() += 1;
}

fn suggested_config_toml(inventory: &ProductConfigInventory) -> String {
    let includes = suggested_includes(inventory);
    let modules = top_keys(&inventory.modules, 12);
    let packages = top_keys(&inventory.packages, 12);
    let external_prefixes = top_keys(&inventory.external_prefixes, 12);

    let mut out = String::new();
    out.push_str(
        "# Suggested capfind product config generated by `capfind init --product-config`.\n",
    );
    out.push_str("# Review and copy the sections you want into .capfind/config.toml.\n");
    out.push_str("# This file is not loaded automatically.\n\n");
    out.push_str("version = 1\n\n[index]\n");
    out.push_str("# Safe default keeps full-repo coverage. Narrow roots after reviewing dashboard coverage.\n");
    out.push_str("roots = [\".\"]\n");
    out.push_str(&format!("include = [{}]\n\n", quoted_list(&includes)));
    out.push_str("[search]\n# BM25 defaults. Tune only after eval data shows ranking drift.\n");
    out.push_str("k1 = 1.2\nb = 0.4\n\n");

    out.push_str("[ownership.modules]\n");
    if modules.is_empty() {
        out.push_str("# \"services/order-api\" = \"Order Platform\"\n");
    } else {
        for module in modules {
            out.push_str(&format!(
                "\"{}\" = \"TODO owner for {}\"\n",
                toml_escape(&module),
                toml_escape(&module)
            ));
        }
    }
    out.push_str("\n[ownership.packages]\n");
    if packages.is_empty() {
        out.push_str("# \"com.example.billing\" = \"Billing Team\"\n");
    } else {
        for package in packages {
            out.push_str(&format!(
                "\"{}\" = \"TODO owner for {}\"\n",
                toml_escape(&package),
                toml_escape(&package)
            ));
        }
    }
    out.push_str("\n[ownership.external]\n");
    if external_prefixes.is_empty() {
        out.push_str("# \"com.fasterxml.jackson.core\" = \"Runtime Platform\"\n");
    } else {
        for prefix in external_prefixes {
            out.push_str(&format!(
                "\"{}\" = \"TODO owner for {}\"\n",
                toml_escape(&prefix),
                toml_escape(&prefix)
            ));
        }
    }
    out.push_str(
        "\n\n# Optional service registry for service-map ownership, tier, and SLO metadata.\n",
    );
    if top_keys(&inventory.modules, 6).is_empty() {
        out.push_str("[services.\"order-api\"]\n");
        out.push_str("name = \"Order API\"\n");
        out.push_str("module = \"services/order-api\"\n");
        out.push_str("owner = \"Order Platform\"\n");
        out.push_str("tier = \"gold\"\n");
        out.push_str("slo = \"99.9%\"\n");
        out.push_str("runbook = \"docs/runbooks/order-api.md\"\n");
        out.push_str("tags = [\"customer-facing\"]\n");
    } else {
        for module in top_keys(&inventory.modules, 6) {
            let service_id = service_id_from_module(&module);
            out.push_str(&format!("\n[services.\"{}\"]\n", toml_escape(&service_id)));
            out.push_str(&format!("name = \"{}\"\n", toml_escape(&service_id)));
            out.push_str(&format!("module = \"{}\"\n", toml_escape(&module)));
            out.push_str(&format!(
                "owner = \"TODO owner for {}\"\n",
                toml_escape(&module)
            ));
            out.push_str("tier = \"standard\"\n");
            out.push_str("slo = \"TODO\"\n");
            out.push_str("runbook = \"TODO\"\n");
            out.push_str("tags = []\n");
        }
    }

    out
}

fn suggested_includes(inventory: &ProductConfigInventory) -> Vec<String> {
    let mut includes = Vec::new();
    if inventory.has_java {
        includes.push("**/*.java".to_string());
    }
    if inventory.has_go {
        includes.push("**/*.go".to_string());
    }
    if inventory.has_proto {
        includes.push("**/*.proto".to_string());
    }
    if inventory.has_xml {
        includes.push("**/*.xml".to_string());
    }
    if inventory.has_maven {
        includes.push("pom.xml".to_string());
        includes.push("**/pom.xml".to_string());
    }
    if inventory.has_gradle {
        includes.push("build.gradle".to_string());
        includes.push("build.gradle.kts".to_string());
        includes.push("**/build.gradle".to_string());
        includes.push("**/build.gradle.kts".to_string());
    }
    if inventory.has_jar {
        includes.push("**/*.jar".to_string());
    }
    if includes.is_empty() {
        includes.extend([
            "**/*.java".to_string(),
            "**/*.go".to_string(),
            "**/*.proto".to_string(),
        ]);
    }
    includes
}

fn top_keys(values: &BTreeMap<String, u64>, limit: usize) -> Vec<String> {
    let mut items = values.iter().collect::<Vec<_>>();
    items.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    items
        .into_iter()
        .take(limit)
        .map(|(key, _)| key.clone())
        .collect()
}

fn service_id_from_module(module: &str) -> String {
    module
        .trim_matches('/')
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(module)
        .replace(['.', ' '], "-")
}

fn quoted_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("\"{}\"", toml_escape(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn generic_mcp_config(repo_root: &Path) -> Result<String> {
    let value = serde_json::json!({
        "mcpServers": {
            "capfind": {
                "command": "capfind",
                "args": ["mcp", "--stdio"],
                "cwd": repo_root.canonicalize().unwrap_or_else(|_| repo_root.to_path_buf()),
            }
        }
    });
    Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
}

fn agent_rules_template() -> String {
    r#"When the user intent is to find an existing interface, capability, call chain,
or reusable implementation, use capfind before text search. If this repository
has .capfind/ or the capfind command is available, run capfind_doctor first,
then call capfind_context with a short task description and limit 5. Use
rg/git grep after capfind only to verify cited candidates, inspect call sites,
or supplement empty/surprising results.

Before implementing a backend endpoint, service method, DAO/repository method,
RPC, integration call, or external-library wrapper, call capfind_context with a
short task description and limit 5.

If capfind returns callable candidates, inspect the cited file:line or
capfind_show output before writing code. Prefer calling the candidate directly,
extending it, or using the external API it points to.

If results are empty or surprising, call capfind_diagnose_query. If a known
file is missing, call capfind_diagnose_file. When ownership or dependencies
matter, call capfind_map and inspect graph edges, diagnostics, and ownership.

After code generation, call capfind_detect_adoption with the task and candidate
ids so the dashboard can measure final-code adoption.
"#
    .to_string()
}

fn mcp_client_template(repo_root: &Path) -> String {
    format!(
        r#"# capfind MCP client setup

Use the same process contract as other MCP clients:

```json
{{
  "mcpServers": {{
    "capfind": {{
      "command": "capfind",
      "args": ["mcp", "--stdio"],
      "cwd": "{}"
    }}
  }}
}}
```

If your client uses command registration instead of JSON, register:

```bash
capfind mcp --stdio
```

Run it from the repository root above.

Suggested agent policy: for requests to find an existing interface, capability,
call chain, or reusable implementation, call capfind_doctor and capfind_context
before rg/git grep. Use text search afterward to verify file:line citations,
inspect call sites, or supplement empty/surprising capfind results."#,
        repo_root
            .canonicalize()
            .unwrap_or_else(|_| repo_root.to_path_buf())
            .display()
    )
}

fn write_generated_file(path: &Path, content: &str, force: bool) -> Result<bool> {
    if path.exists() && !force {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(true)
}

/// `capfind index` — scan + build index.
pub fn index(repo_root: &Path, rehash: bool) -> Result<()> {
    build_index_file(repo_root, rehash, true)
}

fn build_index_file(repo_root: &Path, rehash: bool, verbose: bool) -> Result<()> {
    let start = Instant::now();
    let cfg = config::load(repo_root)?;

    if verbose {
        println!(
            "Scanning {}...",
            scan_roots_display(repo_root, &cfg.index.roots)
        );
    }
    let build_start = Instant::now();
    let result = indexer::index_repo(repo_root, &cfg.index, rehash)?;
    let body = result.body;
    let build_elapsed = build_start.elapsed();
    if verbose {
        println!(
            "  Found {} capabilities in {:.2}s",
            body.capabilities.len(),
            start.elapsed().as_secs_f64()
        );
        println!(
            "  Parsed {} files, reused {} unchanged files",
            result.parsed_files, result.reused_files
        );
        println!(
            "  Built index ({} vocab terms) in {:.2}s",
            body.vocab.len(),
            build_elapsed.as_secs_f64()
        );
    }

    let index_path = repo_root.join(".capfind/index.cfi");
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    write_index(&index_path, &body, created, [0u8; 32])?;

    if verbose {
        let size = std::fs::metadata(&index_path).map(|m| m.len()).unwrap_or(0);
        println!(
            "  Wrote {} ({:.1} KB)",
            index_path.display(),
            size as f64 / 1024.0
        );
        println!("  Total: {:.2}s", start.elapsed().as_secs_f64());
    }
    Ok(())
}

fn scan_roots_display(repo_root: &Path, roots: &[String]) -> String {
    let roots = if roots.is_empty() {
        vec![".".to_string()]
    } else {
        roots.to_vec()
    };
    let paths = roots
        .into_iter()
        .filter(|root| !root.trim().is_empty())
        .map(|root| {
            let path = Path::new(root.as_str());
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                repo_root.join(path)
            }
        })
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        repo_root.display().to_string()
    } else {
        paths.join(", ")
    }
}

/// `capfind find` — search the index.
pub fn find(repo_root: &Path, args: &FindArgs, no_color: bool) -> Result<()> {
    let index_path = repo_root.join(".capfind/index.cfi");
    if !index_path.exists() {
        bail!("No index found. Run `capfind index` first.");
    }

    let start = Instant::now();
    let (_header, body) = load_index(&index_path).context("Failed to load index")?;

    let query = args.query.join(" ");
    if query.is_empty() {
        bail!("Please provide query terms, e.g.: capfind find mdm query");
    }

    let cfg = config::load(repo_root)?;
    let has_filters = args.lang.is_some() || args.kind.is_some() || args.path.is_some();
    let search_limit = if has_filters {
        body.capabilities.len().max(args.limit)
    } else {
        args.limit
    };

    let hits = search(
        &query,
        &body.capabilities,
        &body.postings,
        &body.vocab,
        body.avgdl,
        &cfg.search,
        search_limit,
        args.explain,
    );

    let elapsed = start.elapsed();

    // Filter by lang/kind/path before applying the user-visible limit.
    let mut filtered: Vec<_> = hits
        .iter()
        .filter(|h| {
            let cap = &body.capabilities[h.cap_id as usize];
            if let Some(ref lf) = args.lang {
                let lang_ok = match lf {
                    crate::LangFilter::Java => cap.lang == Lang::Java,
                    crate::LangFilter::Go => cap.lang == Lang::Go,
                    crate::LangFilter::Proto => cap.lang == Lang::Proto,
                };
                if !lang_ok {
                    return false;
                }
            }
            if let Some(ref kf) = args.kind {
                let kind_ok = match kf {
                    crate::KindFilter::Endpoint => cap.kind == Kind::HttpEndpoint,
                    crate::KindFilter::Rpc => cap.kind == Kind::RpcMethod,
                    crate::KindFilter::Service => cap.kind == Kind::ServiceMethod,
                    crate::KindFilter::Dao => cap.kind == Kind::DaoMethod,
                };
                if !kind_ok {
                    return false;
                }
            }
            if let Some(ref prefix) = args.path {
                if !cap.file.starts_with(prefix.as_str()) {
                    return false;
                }
            }
            true
        })
        .collect();
    filtered.truncate(args.limit);

    record_query_history_event(
        repo_root,
        QueryHistoryEvent {
            source: "cli",
            tool: "find",
            query: &query,
            limit: args.limit,
            raw_result_count: hits.len(),
            result_count: filtered.len(),
            took_ms: duration_ms(elapsed),
            filters: query_filters_json(&args.lang, &args.kind, args.path.as_deref()),
            top_candidate_ids: candidate_ids_from_hits(
                filtered.iter().copied(),
                &body.capabilities,
            ),
            diagnosis: None,
        },
    );

    if args.json {
        output::print_json(&query, &filtered, &body.capabilities, elapsed)?;
    } else {
        output::print_compact(&filtered, &body.capabilities, no_color);
        eprintln!(
            "\n{} results in {:.0}ms",
            filtered.len(),
            elapsed.as_millis()
        );
    }

    Ok(())
}

/// `capfind agent` — JSON preflight check for AI coding agents.
pub fn agent(repo_root: &Path, args: &AgentArgs) -> Result<()> {
    let index_path = repo_root.join(".capfind/index.cfi");
    let auto_index = effective_auto_index(args.auto_index, args.no_auto_index);
    if !index_path.exists() {
        if auto_index {
            build_index_file(repo_root, false, false)
                .context("Failed to auto-build capfind index")?;
        } else {
            bail!("No index found. Run `capfind index` first, or pass `--auto-index`.");
        }
    }

    let start = Instant::now();
    let (_header, body) = load_index(&index_path).context("Failed to load index")?;

    let query = args.task.join(" ");
    if query.is_empty() {
        bail!("Please provide a task, e.g.: capfind agent add mdm query endpoint");
    }

    let cfg = config::load(repo_root)?;
    let has_filters = args.lang.is_some() || args.kind.is_some() || args.path.is_some();
    let search_limit = if has_filters {
        body.capabilities.len().max(args.limit)
    } else {
        args.limit
    };

    let hits = search(
        &query,
        &body.capabilities,
        &body.postings,
        &body.vocab,
        body.avgdl,
        &cfg.search,
        search_limit,
        false,
    );

    let mut filtered: Vec<_> = hits
        .iter()
        .filter(|h| {
            let cap = &body.capabilities[h.cap_id as usize];
            if let Some(ref lf) = args.lang {
                let lang_ok = match lf {
                    crate::LangFilter::Java => cap.lang == Lang::Java,
                    crate::LangFilter::Go => cap.lang == Lang::Go,
                    crate::LangFilter::Proto => cap.lang == Lang::Proto,
                };
                if !lang_ok {
                    return false;
                }
            }
            if let Some(ref kf) = args.kind {
                let kind_ok = match kf {
                    crate::KindFilter::Endpoint => cap.kind == Kind::HttpEndpoint,
                    crate::KindFilter::Rpc => cap.kind == Kind::RpcMethod,
                    crate::KindFilter::Service => cap.kind == Kind::ServiceMethod,
                    crate::KindFilter::Dao => cap.kind == Kind::DaoMethod,
                };
                if !kind_ok {
                    return false;
                }
            }
            if let Some(ref prefix) = args.path {
                if !cap.file.starts_with(prefix.as_str()) {
                    return false;
                }
            }
            true
        })
        .collect();
    filtered.truncate(args.limit);

    let elapsed = start.elapsed();
    record_query_history_event(
        repo_root,
        QueryHistoryEvent {
            source: "cli",
            tool: "agent",
            query: &query,
            limit: args.limit,
            raw_result_count: hits.len(),
            result_count: filtered.len(),
            took_ms: duration_ms(elapsed),
            filters: query_filters_json(&args.lang, &args.kind, args.path.as_deref()),
            top_candidate_ids: candidate_ids_from_hits(
                filtered.iter().copied(),
                &body.capabilities,
            ),
            diagnosis: None,
        },
    );

    let has_candidates = !filtered.is_empty();
    output::print_agent_json(
        &query,
        &filtered,
        &body.capabilities,
        elapsed,
        args.fail_on_candidates,
    )?;
    if args.fail_on_candidates && has_candidates {
        use std::io::Write;
        std::io::stdout().flush()?;
        std::process::exit(2);
    }
    Ok(())
}

/// `capfind context` — low-token AI context for planning and implementation.
pub fn context(repo_root: &Path, args: &ContextArgs) -> Result<()> {
    let task = args.task.join(" ");
    if task.is_empty() {
        bail!("Please provide a task, e.g.: capfind context add mdm query endpoint");
    }

    let index_path = repo_root.join(".capfind/index.cfi");
    let start = Instant::now();
    let (_header, body) = load_index_for_tool(
        repo_root,
        effective_auto_index(args.auto_index, args.no_auto_index),
        &index_path,
    )?;
    let cfg = config::load(repo_root)?;
    let has_filters = args.lang.is_some() || args.kind.is_some() || args.path.is_some();
    let search_limit = if has_filters {
        body.capabilities.len().max(args.limit)
    } else {
        args.limit
    };
    let hits = search(
        &task,
        &body.capabilities,
        &body.postings,
        &body.vocab,
        body.avgdl,
        &cfg.search,
        search_limit,
        false,
    );
    let filtered = filter_hits(
        &hits,
        &body.capabilities,
        &args.lang,
        &args.kind,
        &args.path,
        args.limit,
    );
    let adoption_summary = adoption::summary(repo_root);
    let filtered =
        apply_adoption_quality_ranking(filtered, &adoption_summary, &body.capabilities, args.limit);
    let elapsed = start.elapsed();
    record_query_history_event(
        repo_root,
        QueryHistoryEvent {
            source: "cli",
            tool: "context",
            query: &task,
            limit: args.limit,
            raw_result_count: hits.len(),
            result_count: filtered.len(),
            took_ms: duration_ms(elapsed),
            filters: query_filters_json(&args.lang, &args.kind, args.path.as_deref()),
            top_candidate_ids: candidate_ids_from_hits(
                filtered.iter().copied(),
                &body.capabilities,
            ),
            diagnosis: None,
        },
    );
    let mut response = ai_context::context_json(&task, &filtered, &body.capabilities, elapsed);
    annotate_context_with_adoption_quality(&mut response, &adoption_summary);
    if args.record_shown {
        let recorded = record_hits_stage(
            repo_root,
            "cli",
            &task,
            &filtered,
            &body.capabilities,
            "shown",
            args.session_id.as_deref(),
        )?;
        response["adoption_recording"] = serde_json::json!({
            "recorded": recorded,
            "stage": "shown",
            "source": "cli",
            "session_id": args.session_id.as_deref(),
        });
    }
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn record_hits_stage(
    repo_root: &Path,
    source: &str,
    task: &str,
    hits: &[&Hit],
    caps: &[Capability],
    stage: &str,
    session_id: Option<&str>,
) -> Result<usize> {
    let files = Vec::<String>::new();
    let note = if source == "mcp" {
        "capfind_context returned candidate"
    } else {
        "capfind context returned candidate"
    };
    for hit in hits {
        let cap = &caps[hit.cap_id as usize];
        adoption::record_event_with_source(
            repo_root,
            adoption::EventRecord {
                source,
                task,
                candidate: cap,
                stage,
                files: &files,
                note: Some(note),
                rejected_reason: None,
                session_id,
            },
        )?;
    }
    Ok(hits.len())
}

fn apply_adoption_quality_ranking<'a>(
    hits: Vec<&'a Hit>,
    adoption_summary: &serde_json::Value,
    caps: &[Capability],
    limit: usize,
) -> Vec<&'a Hit> {
    let quality = candidate_quality_index(adoption_summary);
    if quality.is_empty() {
        return hits;
    }
    let mut ranked = hits.into_iter().enumerate().collect::<Vec<_>>();
    ranked.sort_by(|(left_idx, left), (right_idx, right)| {
        let left_cap = &caps[left.cap_id as usize];
        let right_cap = &caps[right.cap_id as usize];
        let left_rank = quality
            .get(&left_cap.id)
            .map(quality_rank)
            .unwrap_or_default();
        let right_rank = quality
            .get(&right_cap.id)
            .map(quality_rank)
            .unwrap_or_default();
        left_rank
            .cmp(&right_rank)
            .then_with(|| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then(left_idx.cmp(right_idx))
    });
    ranked.into_iter().take(limit).map(|(_, hit)| hit).collect()
}

fn annotate_context_with_adoption_quality(
    response: &mut serde_json::Value,
    adoption_summary: &serde_json::Value,
) {
    let quality = candidate_quality_index(adoption_summary);
    response["recommendation_policy"] = serde_json::json!({
        "schema_version": "capfind.recommendation_policy.v1",
        "ranking": "bm25_then_adoption_quality",
        "signals": {
            "high_quality_candidates": value_u64(&adoption_summary["quality"]["summary"]["high_quality_candidates"]),
            "noise_risk_candidates": value_u64(&adoption_summary["quality"]["summary"]["noise_risk_candidates"]),
            "low_confidence_candidates": value_u64(&adoption_summary["quality"]["summary"]["low_confidence_candidates"]),
        }
    });
    let Some(candidates) = response["candidates"].as_array_mut() else {
        return;
    };
    for candidate in candidates {
        let Some(id) = candidate.get("id").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        if let Some(signal) = quality.get(&(id as u32)) {
            candidate["quality_signal"] = serde_json::json!({
                "schema_version": "capfind.candidate_quality_signal.v1",
                "label": signal["quality_label"].as_str().unwrap_or("unknown"),
                "recommended_action": signal["recommended_action"].as_str().unwrap_or("watch_more_sessions"),
                "adoption_probability": value_f64(&signal["adoption_probability"]),
                "rejection_probability": value_f64(&signal["rejection_probability"]),
                "confidence": value_f64(&signal["confidence"]),
                "reason": signal["reason"].as_str().unwrap_or_default(),
            });
        }
    }
}

fn candidate_quality_index(
    adoption_summary: &serde_json::Value,
) -> BTreeMap<u32, serde_json::Value> {
    adoption_summary["quality"]["candidate_quality"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| Some((row["candidate_id"].as_u64()? as u32, row.clone())))
                .collect()
        })
        .unwrap_or_default()
}

fn quality_rank(signal: &serde_json::Value) -> i32 {
    let confidence = value_f64(&signal["confidence"]);
    let label = signal["quality_label"].as_str().unwrap_or_default();
    match label {
        "strong_reuse_signal" if confidence >= 0.5 => -3,
        "likely_reusable" if confidence >= 0.35 => -2,
        "noise_risk" if confidence >= 0.35 => 3,
        "low_confidence" => 1,
        "mixed" => 1,
        _ => 0,
    }
}

struct QueryHistoryEvent<'a> {
    source: &'a str,
    tool: &'a str,
    query: &'a str,
    limit: usize,
    raw_result_count: usize,
    result_count: usize,
    took_ms: u64,
    filters: serde_json::Value,
    top_candidate_ids: Vec<u32>,
    diagnosis: Option<&'a str>,
}

fn duration_ms(duration: std::time::Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

fn record_query_history_event(repo_root: &Path, event: QueryHistoryEvent<'_>) {
    let _ = append_query_history_event(repo_root, event);
}

fn append_query_history_event(repo_root: &Path, event: QueryHistoryEvent<'_>) -> Result<()> {
    use std::io::Write;

    let path = repo_root.join(QUERY_HISTORY_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(
        file,
        "{}",
        serde_json::json!({
            "schema_version": QUERY_EVENT_SCHEMA_VERSION,
            "created_unix": now_unix(),
            "source": event.source,
            "tool": event.tool,
            "query": event.query,
            "limit": event.limit,
            "raw_result_count": event.raw_result_count,
            "result_count": event.result_count,
            "zero_results": event.result_count == 0,
            "took_ms": event.took_ms,
            "filters": event.filters,
            "top_candidate_ids": event.top_candidate_ids,
            "diagnosis": event.diagnosis,
        })
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_query_history_from_diagnosis(
    repo_root: &Path,
    source: &str,
    tool: &str,
    query: &str,
    limit: usize,
    lang: &Option<LangFilter>,
    kind: &Option<KindFilter>,
    path: Option<&str>,
    response: &serde_json::Value,
) {
    let top_candidate_ids = response["filtered_top"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["id"].as_u64().map(|id| id as u32))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    record_query_history_event(
        repo_root,
        QueryHistoryEvent {
            source,
            tool,
            query,
            limit,
            raw_result_count: value_u64(&response["result_counts"]["raw_hits"]) as usize,
            result_count: value_u64(&response["result_counts"]["filtered_hits"]) as usize,
            took_ms: value_u64(&response["took_ms"]),
            filters: query_filters_json(lang, kind, path),
            top_candidate_ids,
            diagnosis: response["diagnosis"].as_str(),
        },
    );
}

fn query_filters_json(
    lang: &Option<LangFilter>,
    kind: &Option<KindFilter>,
    path: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "lang": lang.as_ref().map(lang_filter_name),
        "kind": kind.as_ref().map(kind_filter_name),
        "path": path,
    })
}

fn candidate_ids_from_hits<'a>(
    hits: impl IntoIterator<Item = &'a Hit>,
    caps: &[Capability],
) -> Vec<u32> {
    hits.into_iter()
        .take(10)
        .filter_map(|hit| caps.get(hit.cap_id as usize).map(|cap| cap.id))
        .collect()
}

fn filter_hits<'a>(
    hits: &'a [Hit],
    caps: &[Capability],
    lang: &Option<LangFilter>,
    kind: &Option<KindFilter>,
    path: &Option<String>,
    limit: usize,
) -> Vec<&'a Hit> {
    let mut filtered = hits
        .iter()
        .filter(|hit| {
            let cap = &caps[hit.cap_id as usize];
            if let Some(lf) = lang {
                let lang_ok = match lf {
                    LangFilter::Java => cap.lang == Lang::Java,
                    LangFilter::Go => cap.lang == Lang::Go,
                    LangFilter::Proto => cap.lang == Lang::Proto,
                };
                if !lang_ok {
                    return false;
                }
            }
            if let Some(kf) = kind {
                let kind_ok = match kf {
                    KindFilter::Endpoint => cap.kind == Kind::HttpEndpoint,
                    KindFilter::Rpc => cap.kind == Kind::RpcMethod,
                    KindFilter::Service => cap.kind == Kind::ServiceMethod,
                    KindFilter::Dao => cap.kind == Kind::DaoMethod,
                };
                if !kind_ok {
                    return false;
                }
            }
            if let Some(prefix) = path {
                if !cap.file.starts_with(prefix.as_str()) {
                    return false;
                }
            }
            true
        })
        .collect::<Vec<_>>();
    filtered.truncate(limit);
    filtered
}

#[allow(clippy::too_many_arguments)]
fn diagnose_query_json(
    repo_root: &Path,
    query: &str,
    limit: usize,
    lang: &Option<LangFilter>,
    kind: &Option<KindFilter>,
    path: &Option<String>,
    auto_index: bool,
) -> Result<serde_json::Value> {
    use serde_json::json;

    let start = Instant::now();
    let index_path = repo_root.join(".capfind/index.cfi");
    let index_present_before = index_path.exists();
    let (_header, body) = load_index_for_tool(repo_root, auto_index, &index_path)?;
    let cfg = config::load(repo_root)?;
    let search_limit = body.capabilities.len().max(limit).max(50);
    let hits = search(
        query,
        &body.capabilities,
        &body.postings,
        &body.vocab,
        body.avgdl,
        &cfg.search,
        search_limit,
        true,
    );
    let filtered = filter_hits(&hits, &body.capabilities, lang, kind, path, limit);
    let vocab_set = body
        .vocab
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let query_tokens = tokenize_text(query);
    let token_diagnostics = query_tokens
        .iter()
        .map(|token| {
            let synonyms = synonyms::expand(token)
                .iter()
                .map(|synonym| {
                    json!({
                        "term": synonym,
                        "in_vocab": vocab_set.contains(synonym),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "term": token,
                "in_vocab": vocab_set.contains(token.as_str()),
                "synonyms": synonyms,
            })
        })
        .collect::<Vec<_>>();
    let phrase_expansions = synonyms::expand_phrases(query)
        .iter()
        .map(|expansion| {
            let terms = expansion
                .terms
                .iter()
                .map(|term| {
                    json!({
                        "term": term.term,
                        "weight": term.weight,
                        "in_vocab": vocab_set.contains(term.term),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "phrase": expansion.phrase,
                "terms": terms,
            })
        })
        .collect::<Vec<_>>();
    let configured_synonyms = cfg
        .search
        .synonym_rules
        .iter()
        .filter(|rule| rule.matches_query(query, &query_tokens))
        .map(|rule| {
            let terms = rule
                .terms
                .iter()
                .map(|term| {
                    json!({
                        "term": term,
                        "weight": rule.weight,
                        "in_vocab": vocab_set.contains(term.as_str()),
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "trigger": rule.trigger,
                "terms": terms,
            })
        })
        .collect::<Vec<_>>();
    let has_query_vocab_signal = query_tokens
        .iter()
        .any(|token| vocab_set.contains(token.as_str()))
        || query_tokens.iter().any(|token| {
            synonyms::expand(token)
                .iter()
                .any(|synonym| vocab_set.contains(synonym))
        })
        || synonyms::expand_phrases(query).iter().any(|expansion| {
            expansion
                .terms
                .iter()
                .any(|term| vocab_set.contains(term.term))
        })
        || cfg.search.synonym_rules.iter().any(|rule| {
            rule.matches_query(query, &query_tokens)
                && rule
                    .terms
                    .iter()
                    .any(|term| vocab_set.contains(term.as_str()))
        });

    let raw_top = hits
        .iter()
        .take(limit)
        .map(|hit| diagnostic_hit_json(hit, &body.capabilities, lang, kind, path))
        .collect::<Vec<_>>();
    let filtered_hits = filtered
        .iter()
        .map(|hit| diagnostic_hit_json(hit, &body.capabilities, lang, kind, path))
        .collect::<Vec<_>>();
    let filter_loss = hits
        .iter()
        .take(search_limit)
        .filter(|hit| !matches_filters(&body.capabilities[hit.cap_id as usize], lang, kind, path))
        .count();
    let diagnosis = query_diagnosis(
        &body,
        &hits,
        &filtered,
        &query_tokens,
        has_query_vocab_signal,
        filter_loss,
        lang,
        kind,
        path,
    );
    let next_actions = query_next_actions(diagnosis);

    Ok(json!({
        "schema_version": "capfind.query_diagnosis.v1",
        "query": query,
        "took_ms": duration_ms(start.elapsed()),
        "index": {
            "path": index_path.display().to_string(),
            "present_before": index_present_before,
            "auto_index_used": !index_present_before && auto_index,
            "capabilities": body.capabilities.len(),
            "indexed_files": body.file_stats.len(),
            "vocab": body.vocab.len(),
        },
        "filters": {
            "lang": lang.as_ref().map(lang_filter_name),
            "kind": kind.as_ref().map(kind_filter_name),
            "path": path,
        },
        "tokens": token_diagnostics,
        "phrase_expansions": phrase_expansions,
        "configured_synonyms": configured_synonyms,
        "result_counts": {
            "raw_hits": hits.len(),
            "filtered_hits": filtered.len(),
            "filter_loss": filter_loss,
        },
        "diagnosis": diagnosis,
        "next_actions": next_actions,
        "raw_top": raw_top,
        "filtered_top": filtered_hits,
    }))
}

fn diagnostic_hit_json(
    hit: &Hit,
    caps: &[Capability],
    lang: &Option<LangFilter>,
    kind: &Option<KindFilter>,
    path: &Option<String>,
) -> serde_json::Value {
    use serde_json::json;
    let cap = &caps[hit.cap_id as usize];
    let mut obj = capability_json(cap, hit.score);
    obj["type"] = json!(ai_context::candidate_type(cap));
    obj["matches_filters"] = json!(matches_filters(cap, lang, kind, path));
    if let Some(ref explain) = hit.explain {
        obj["explain"] = json!({
            "bm25_raw": explain.bm25_raw,
            "final_score": explain.final_score,
            "boosts": explain.boosts.iter().map(|(name, factor)| json!({
                "name": name,
                "factor": factor,
            })).collect::<Vec<_>>(),
            "term_hits": explain.term_hits.iter().take(8).map(|term| json!({
                "term": term.term,
                "field": format!("{:?}", term.field),
                "tf": term.tf,
                "weight": term.weight,
                "is_synonym": term.is_synonym,
            })).collect::<Vec<_>>(),
        });
    }
    obj
}

fn matches_filters(
    cap: &Capability,
    lang: &Option<LangFilter>,
    kind: &Option<KindFilter>,
    path: &Option<String>,
) -> bool {
    if let Some(lang) = lang {
        let lang_ok = match lang {
            LangFilter::Java => cap.lang == Lang::Java,
            LangFilter::Go => cap.lang == Lang::Go,
            LangFilter::Proto => cap.lang == Lang::Proto,
        };
        if !lang_ok {
            return false;
        }
    }
    if let Some(kind) = kind {
        let kind_ok = match kind {
            KindFilter::Endpoint => cap.kind == Kind::HttpEndpoint,
            KindFilter::Rpc => cap.kind == Kind::RpcMethod,
            KindFilter::Service => cap.kind == Kind::ServiceMethod,
            KindFilter::Dao => cap.kind == Kind::DaoMethod,
        };
        if !kind_ok {
            return false;
        }
    }
    if let Some(prefix) = path {
        if !cap.file.starts_with(prefix.as_str()) {
            return false;
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn query_diagnosis(
    body: &IndexBody,
    hits: &[Hit],
    filtered: &[&Hit],
    query_tokens: &[String],
    has_query_vocab_signal: bool,
    filter_loss: usize,
    lang: &Option<LangFilter>,
    kind: &Option<KindFilter>,
    path: &Option<String>,
) -> &'static str {
    if body.capabilities.is_empty() {
        "empty_index"
    } else if query_tokens.is_empty() {
        "query_has_no_indexable_tokens"
    } else if !has_query_vocab_signal {
        "query_terms_not_in_vocab"
    } else if hits.is_empty() {
        "terms_exist_but_no_postings_scored"
    } else if filtered.is_empty() && filter_loss > 0 {
        "filters_removed_all_hits"
    } else if filtered.is_empty() && (lang.is_some() || kind.is_some() || path.is_some()) {
        "filters_match_no_candidates"
    } else if filtered.is_empty() {
        "no_ranked_candidates"
    } else {
        "ranked_candidates_available"
    }
}

fn query_next_actions(diagnosis: &str) -> Vec<&'static str> {
    match diagnosis {
        "empty_index" => vec!["run_capfind_index", "check_roots_and_include"],
        "query_has_no_indexable_tokens" => vec!["rewrite_query_with_domain_terms"],
        "query_terms_not_in_vocab" => vec![
            "check_whether_target_files_are_indexed",
            "try_known_class_method_or_endpoint_terms",
            "run_capfind_doctor",
        ],
        "filters_removed_all_hits" | "filters_match_no_candidates" => {
            vec!["relax_lang_kind_path_filters", "inspect_raw_top_hits"]
        }
        "terms_exist_but_no_postings_scored" | "no_ranked_candidates" => {
            vec![
                "run_capfind_map",
                "try_broader_query",
                "check_parser_coverage",
            ]
        }
        _ => vec!["inspect_filtered_top", "open_candidate_file_lines"],
    }
}

fn lang_filter_name(lang: &LangFilter) -> &'static str {
    match lang {
        LangFilter::Java => "java",
        LangFilter::Go => "go",
        LangFilter::Proto => "proto",
    }
}

fn kind_filter_name(kind: &KindFilter) -> &'static str {
    match kind {
        KindFilter::Endpoint => "endpoint",
        KindFilter::Rpc => "rpc",
        KindFilter::Service => "service",
        KindFilter::Dao => "dao",
    }
}

/// `capfind mcp` — MCP-compatible tool catalog and local tool-call shim.
pub fn mcp(repo_root: &Path, args: &McpArgs) -> Result<()> {
    use serde_json::{json, Value};
    let auto_index = effective_auto_index(args.auto_index, args.no_auto_index);

    if args.stdio {
        return mcp_stdio(repo_root, auto_index);
    }

    if args.list_tools {
        println!("{}", serde_json::to_string_pretty(&mcp_tools_json())?);
        return Ok(());
    }

    let Some(tool_name) = args.call.as_deref() else {
        println!("{}", serde_json::to_string_pretty(&mcp_tools_json())?);
        return Ok(());
    };
    let input: Value = serde_json::from_str(&args.args).context("Invalid --args JSON")?;
    let output = mcp_call_tool(repo_root, tool_name, &input, auto_index)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "tool": tool_name,
            "result": output,
        }))?
    );
    Ok(())
}

fn mcp_stdio(repo_root: &Path, auto_index: bool) -> Result<()> {
    use serde_json::{json, Value};
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                writeln!(
                    stdout,
                    "{}",
                    json_rpc_error(Value::Null, -32700, err.to_string())
                )?;
                stdout.flush()?;
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let response = match method {
            "initialize" => Some(json_rpc_result(
                id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "capfind", "version": env!("CARGO_PKG_VERSION")}
                }),
            )),
            "tools/list" => Some(json_rpc_result(
                id,
                json!({
                    "tools": mcp_tools_json()["tools"].clone()
                }),
            )),
            "tools/call" => Some(handle_mcp_tools_call(repo_root, id, &request, auto_index)),
            method if method.starts_with("notifications/") => None,
            _ => Some(json_rpc_error(
                id,
                -32601,
                format!("Unknown method: {method}"),
            )),
        };
        if let Some(response) = response {
            writeln!(stdout, "{}", response)?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn handle_mcp_tools_call(
    repo_root: &Path,
    id: serde_json::Value,
    request: &serde_json::Value,
    auto_index: bool,
) -> serde_json::Value {
    use serde_json::json;
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let Some(tool_name) = params.get("name").and_then(|value| value.as_str()) else {
        return json_rpc_error(id, -32602, "tools/call requires params.name".to_string());
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match mcp_call_tool(repo_root, tool_name, &arguments, auto_index) {
        Ok(result) => json_rpc_result(
            id,
            json!({
                "content": [{"type": "text", "text": result.to_string()}],
                "structuredContent": result,
                "isError": false
            }),
        ),
        Err(err) => json_rpc_result(
            id,
            json!({
                "content": [{"type": "text", "text": err.to_string()}],
                "isError": true
            }),
        ),
    }
}

fn json_rpc_result(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    use serde_json::json;
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn json_rpc_error(id: serde_json::Value, code: i64, message: String) -> serde_json::Value {
    use serde_json::json;
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn effective_auto_index(_auto_index: bool, no_auto_index: bool) -> bool {
    !no_auto_index
}

fn mcp_call_tool(
    repo_root: &Path,
    tool_name: &str,
    input: &serde_json::Value,
    auto_index: bool,
) -> Result<serde_json::Value> {
    match tool_name {
        "capfind_context" => mcp_context(repo_root, input, auto_index),
        "capfind_search" => mcp_search(repo_root, input, auto_index),
        "capfind_show" => mcp_show(repo_root, input, auto_index),
        "capfind_agent_preflight" => mcp_agent_preflight(repo_root, input, auto_index),
        "capfind_doctor" => dashboard_summary(repo_root),
        "capfind_diagnose_query" => mcp_diagnose_query(repo_root, input, auto_index),
        "capfind_diagnose_file" => mcp_diagnose_file(repo_root, input, auto_index),
        "capfind_map" => mcp_map(repo_root, input, auto_index),
        "capfind_record_adoption" => mcp_record_adoption(repo_root, input),
        "capfind_detect_adoption" => mcp_detect_adoption(repo_root, input, auto_index),
        other => bail!("Unknown MCP tool: {other}"),
    }
}

fn load_index_for_tool(
    repo_root: &Path,
    auto_index: bool,
    index_path: &Path,
) -> Result<(IndexHeader, IndexBody)> {
    if !index_path.exists() {
        if auto_index {
            build_index_file(repo_root, false, false)
                .context("Failed to auto-build capfind index")?;
        } else {
            bail!(
                "No capfind index found at {}. Run `capfind index` first, or start MCP with `capfind mcp --stdio --auto-index`.",
                index_path.display()
            );
        }
    }
    let mut loaded = load_index(index_path).context("Failed to load index")?;
    if auto_index && index_requires_refresh(repo_root, index_path, &loaded.1)? {
        build_index_file(repo_root, false, false)
            .context("Failed to refresh stale capfind index")?;
        loaded = load_index(index_path).context("Failed to load refreshed index")?;
    }
    Ok(loaded)
}

fn index_requires_refresh(repo_root: &Path, index_path: &Path, body: &IndexBody) -> Result<bool> {
    if path_modified_after(index_path, &repo_root.join(".capfind/config.toml"))?
        || path_modified_after(index_path, &repo_root.join(".capfindignore"))?
    {
        return Ok(true);
    }
    let cfg = config::load(repo_root)?;
    let roots = indexer::resolve_scan_roots(repo_root, &cfg.index.roots)
        .unwrap_or_else(|_| vec![repo_root.to_path_buf()]);
    if body.file_stats.iter().any(|(rel_path, stat)| {
        let path = indexed_file_path(repo_root, &roots, rel_path);
        metadata_changed(&path, stat)
    }) {
        return Ok(true);
    }

    has_new_indexable_file(repo_root, index_path, &roots, &cfg.index.include, body)
}

fn has_new_indexable_file(
    repo_root: &Path,
    index_path: &Path,
    roots: &[std::path::PathBuf],
    include_patterns: &[String],
    body: &IndexBody,
) -> Result<bool> {
    use ignore::WalkBuilder;

    let index_modified = std::fs::metadata(index_path)?.modified()?;
    let include_set = indexer::build_include_set(include_patterns)?;
    let capfind_ignore = std::sync::Arc::new(indexer::load_capfindignore(repo_root)?);

    for root in roots {
        let capfind_ignore = std::sync::Arc::clone(&capfind_ignore);
        let mut builder = WalkBuilder::new(root);
        builder
            .hidden(true)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(true)
            .filter_entry(move |entry| {
                should_visit_refresh_entry(capfind_ignore.as_ref().as_ref(), entry)
            });
        let walker = builder.build();

        for entry in walker {
            let Ok(entry) = entry else {
                continue;
            };
            if !entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
            {
                continue;
            }
            let path = entry.path();
            let rel_path_buf = path.strip_prefix(repo_root).unwrap_or(path).to_path_buf();
            if !indexer::matches_include(include_set.as_ref(), &rel_path_buf)
                || !indexer::is_supported_index_input(path)
                || indexer::is_skipped_source_path(path)
                || exceeds_indexable_size(path)
            {
                continue;
            }

            let rel_path = rel_path_buf.to_string_lossy().to_string();
            if !body.file_stats.contains_key(&rel_path)
                && (metadata_modified_after(path, index_modified)?
                    || path
                        .parent()
                        .map(|parent| metadata_modified_after(parent, index_modified))
                        .transpose()?
                        .unwrap_or(false))
            {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn should_visit_refresh_entry(
    capfind_ignore: Option<&ignore::gitignore::Gitignore>,
    entry: &ignore::DirEntry,
) -> bool {
    let path = entry.path();
    let is_dir = entry
        .file_type()
        .is_some_and(|file_type| file_type.is_dir());
    if indexer::is_capfind_ignored(capfind_ignore, path, is_dir) {
        return false;
    }
    !(is_dir && is_skipped_refresh_dir(path))
}

fn is_skipped_refresh_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                "target" | "build" | "node_modules" | "vendor" | ".gradle" | "generated"
            )
        })
}

fn exceeds_indexable_size(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return true;
    };
    let limit = if path.extension().and_then(|ext| ext.to_str()) == Some("jar") {
        indexer::MAX_JAR_BYTES
    } else {
        indexer::MAX_SOURCE_BYTES
    };
    metadata.len() > limit
}

fn path_modified_after(index_path: &Path, path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let index_modified = std::fs::metadata(index_path)?.modified()?;
    metadata_modified_after(path, index_modified)
}

fn metadata_modified_after(path: &Path, index_modified: SystemTime) -> Result<bool> {
    let path_modified = std::fs::metadata(path)?.modified()?;
    Ok(path_modified > index_modified)
}

fn indexed_file_path(repo_root: &Path, roots: &[std::path::PathBuf], rel_path: &str) -> PathBuf {
    let direct = repo_root.join(rel_path);
    if direct.exists() {
        return direct;
    }
    roots
        .iter()
        .map(|root| root.join(rel_path))
        .find(|path| path.exists())
        .unwrap_or(direct)
}

fn metadata_changed(path: &Path, old: &capfind_core::FileStat) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return true;
    };
    metadata.len() != old.size || metadata_mtime_ns(&metadata) != old.mtime_ns
}

fn metadata_mtime_ns(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn mcp_tools_json() -> serde_json::Value {
    use serde_json::json;
    json!({
        "server": "capfind",
        "schema_version": "capfind.mcp.v1",
        "tools": [
            {
                "name": "capfind_context",
                "description": "Primary low-token AI context packet for blind capability/API discovery. Call before rg/git grep when the user asks for existing interfaces, call chains, reusable implementations, or available APIs.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "task": {"type": "string"},
                        "limit": {"type": "integer", "default": 5},
                        "lang": {"type": "string"},
                        "kind": {"type": "string"},
                        "path": {"type": "string"},
                        "record_shown": {"type": "boolean", "default": false},
                        "session_id": {"type": "string"}
                    },
                    "required": ["task"]
                }
            },
            {
                "name": "capfind_search",
                "description": "Search indexed capabilities and referenced external APIs; use before text search for capability lookup, then verify with rg/git grep when needed.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "limit": {"type": "integer", "default": 10}
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "capfind_show",
                "description": "Show one capability by numeric id.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "integer"},
                        "task": {"type": "string"},
                        "record_inspected": {"type": "boolean", "default": false},
                        "session_id": {"type": "string"}
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "capfind_agent_preflight",
                "description": "Preflight duplicate/reuse check before creating a new capability.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "task": {"type": "string"},
                        "limit": {"type": "integer", "default": 5}
                    },
                    "required": ["task"]
                }
            },
            {
                "name": "capfind_doctor",
                "description": "Return index/config/coverage/adoption diagnostics. Use as preflight when .capfind exists or capfind is available before capability discovery.",
                "input_schema": {"type": "object", "properties": {}}
            },
            {
                "name": "capfind_diagnose_query",
                "description": "Explain why a query did or did not return expected candidates.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "limit": {"type": "integer", "default": 10},
                        "lang": {"type": "string"},
                        "kind": {"type": "string"},
                        "path": {"type": "string"}
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "capfind_diagnose_file",
                "description": "Diagnose file-level index/parser coverage and why a file did not produce capabilities.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "file": {"type": "string"}
                    },
                    "required": ["file"]
                }
            },
            {
                "name": "capfind_map",
                "description": "Return a machine-readable code asset/service map with module/package/class/capability nodes.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "module": {"type": "string"},
                        "limit": {"type": "integer", "default": 200}
                    }
                }
            },
            {
                "name": "capfind_record_adoption",
                "description": "Record whether final generated code adopted a candidate.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "task": {"type": "string"},
                        "candidate_id": {"type": "integer"},
                        "adopted": {"type": "boolean", "default": true},
                        "stage": {"type": "string", "enum": ["shown", "inspected", "adopted", "rejected"]},
                        "rejected_reason": {"type": "string", "enum": ["wrong_ownership_boundary", "not_callable", "missing_behavior", "unsafe_abstraction", "external_api_required", "low_confidence", "stale_index", "user_requested_new", "other", "unspecified"]},
                        "files": {"type": "array", "items": {"type": "string"}},
                        "note": {"type": "string"},
                        "session_id": {"type": "string"}
                    },
                    "required": ["task", "candidate_id"]
                }
            },
            {
                "name": "capfind_detect_adoption",
                "description": "Detect final-code adoption from git diff and record matched candidates.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "since": {"type": "string", "default": "HEAD"},
                        "task": {"type": "string"},
                        "candidate_id": {"type": "integer"},
                        "candidate_ids": {"type": "array", "items": {"type": "integer"}},
                        "dry_run": {"type": "boolean", "default": false},
                        "session_id": {"type": "string"}
                    }
                }
            }
        ]
    })
}

fn mcp_context(
    repo_root: &Path,
    input: &serde_json::Value,
    auto_index: bool,
) -> Result<serde_json::Value> {
    let task = input
        .get("task")
        .or_else(|| input.get("query"))
        .and_then(|value| value.as_str())
        .context("capfind_context requires string field `task`")?;
    let limit = input
        .get("limit")
        .and_then(|value| value.as_u64())
        .unwrap_or(5) as usize;
    let lang = input
        .get("lang")
        .and_then(|value| value.as_str())
        .and_then(parse_lang_filter);
    let kind = input
        .get("kind")
        .and_then(|value| value.as_str())
        .and_then(parse_kind_filter);
    let path = input
        .get("path")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let record_shown = input
        .get("record_shown")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let session_id = input.get("session_id").and_then(|value| value.as_str());

    let start = Instant::now();
    let index_path = repo_root.join(".capfind/index.cfi");
    let (_header, body) = load_index_for_tool(repo_root, auto_index, &index_path)?;
    let cfg = config::load(repo_root)?;
    let has_filters = lang.is_some() || kind.is_some() || path.is_some();
    let search_limit = if has_filters {
        body.capabilities.len().max(limit)
    } else {
        limit
    };
    let hits = search(
        task,
        &body.capabilities,
        &body.postings,
        &body.vocab,
        body.avgdl,
        &cfg.search,
        search_limit,
        false,
    );
    let filtered = filter_hits(&hits, &body.capabilities, &lang, &kind, &path, limit);
    let adoption_summary = adoption::summary(repo_root);
    let filtered =
        apply_adoption_quality_ranking(filtered, &adoption_summary, &body.capabilities, limit);
    let elapsed = start.elapsed();
    record_query_history_event(
        repo_root,
        QueryHistoryEvent {
            source: "mcp",
            tool: "capfind_context",
            query: task,
            limit,
            raw_result_count: hits.len(),
            result_count: filtered.len(),
            took_ms: duration_ms(elapsed),
            filters: query_filters_json(&lang, &kind, path.as_deref()),
            top_candidate_ids: candidate_ids_from_hits(
                filtered.iter().copied(),
                &body.capabilities,
            ),
            diagnosis: None,
        },
    );
    let mut response = ai_context::context_json(task, &filtered, &body.capabilities, elapsed);
    annotate_context_with_adoption_quality(&mut response, &adoption_summary);
    if record_shown {
        let recorded = record_hits_stage(
            repo_root,
            "mcp",
            task,
            &filtered,
            &body.capabilities,
            "shown",
            session_id,
        )?;
        response["adoption_recording"] = serde_json::json!({
            "recorded": recorded,
            "stage": "shown",
            "source": "mcp",
            "session_id": session_id,
        });
    }
    Ok(response)
}

fn mcp_search(
    repo_root: &Path,
    input: &serde_json::Value,
    auto_index: bool,
) -> Result<serde_json::Value> {
    use serde_json::json;
    let query = input
        .get("query")
        .and_then(|value| value.as_str())
        .context("capfind_search requires string field `query`")?;
    let limit = input
        .get("limit")
        .and_then(|value| value.as_u64())
        .unwrap_or(10) as usize;
    let start = Instant::now();
    let index_path = repo_root.join(".capfind/index.cfi");
    let (_header, body) = load_index_for_tool(repo_root, auto_index, &index_path)?;
    let cfg = config::load(repo_root)?;
    let hits = search(
        query,
        &body.capabilities,
        &body.postings,
        &body.vocab,
        body.avgdl,
        &cfg.search,
        limit,
        false,
    );
    let results = hits
        .iter()
        .map(|hit| capability_json(&body.capabilities[hit.cap_id as usize], hit.score))
        .collect::<Vec<_>>();
    let elapsed = start.elapsed();
    record_query_history_event(
        repo_root,
        QueryHistoryEvent {
            source: "mcp",
            tool: "capfind_search",
            query,
            limit,
            raw_result_count: hits.len(),
            result_count: results.len(),
            took_ms: duration_ms(elapsed),
            filters: query_filters_json(&None, &None, None),
            top_candidate_ids: candidate_ids_from_hits(hits.iter(), &body.capabilities),
            diagnosis: None,
        },
    );
    Ok(json!({
        "query": query,
        "took_ms": duration_ms(elapsed),
        "results": results,
    }))
}

fn parse_lang_filter(value: &str) -> Option<LangFilter> {
    match value.to_ascii_lowercase().as_str() {
        "java" => Some(LangFilter::Java),
        "go" => Some(LangFilter::Go),
        "proto" => Some(LangFilter::Proto),
        _ => None,
    }
}

fn parse_kind_filter(value: &str) -> Option<KindFilter> {
    match value.to_ascii_lowercase().as_str() {
        "endpoint" | "http" | "httpendpoint" => Some(KindFilter::Endpoint),
        "rpc" | "rpcmethod" => Some(KindFilter::Rpc),
        "service" | "servicemethod" => Some(KindFilter::Service),
        "dao" | "daomethod" => Some(KindFilter::Dao),
        _ => None,
    }
}

fn mcp_show(
    repo_root: &Path,
    input: &serde_json::Value,
    auto_index: bool,
) -> Result<serde_json::Value> {
    let id = input
        .get("id")
        .and_then(|value| value.as_u64())
        .context("capfind_show requires integer field `id`")? as usize;
    let index_path = repo_root.join(".capfind/index.cfi");
    let (_header, body) = load_index_for_tool(repo_root, auto_index, &index_path)?;
    let cap = body
        .capabilities
        .get(id)
        .context(format!("No capability with id={id}"))?;
    let mut result = capability_json(cap, 0.0);
    let record_inspected = input
        .get("record_inspected")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let session_id = input.get("session_id").and_then(|value| value.as_str());
    if record_inspected {
        let task = input
            .get("task")
            .and_then(|value| value.as_str())
            .context("capfind_show with record_inspected requires string field `task`")?;
        let files = Vec::<String>::new();
        adoption::record_event_with_source(
            repo_root,
            adoption::EventRecord {
                source: "mcp",
                task,
                candidate: cap,
                stage: "inspected",
                files: &files,
                note: Some("capfind_show returned candidate detail"),
                rejected_reason: None,
                session_id,
            },
        )?;
        result["adoption_recording"] = serde_json::json!({
            "recorded": 1,
            "stage": "inspected",
            "source": "mcp",
            "session_id": session_id,
        });
    }
    Ok(result)
}

fn mcp_agent_preflight(
    repo_root: &Path,
    input: &serde_json::Value,
    auto_index: bool,
) -> Result<serde_json::Value> {
    use serde_json::json;
    let task = input
        .get("task")
        .and_then(|value| value.as_str())
        .context("capfind_agent_preflight requires string field `task`")?;
    let limit = input
        .get("limit")
        .and_then(|value| value.as_u64())
        .unwrap_or(5) as usize;
    let search_result = mcp_search(
        repo_root,
        &json!({"query": task, "limit": limit}),
        auto_index,
    )?;
    let has_candidates = search_result
        .get("results")
        .and_then(|value| value.as_array())
        .map(|results| !results.is_empty())
        .unwrap_or(false);
    Ok(json!({
        "schema_version": output::AGENT_SCHEMA_VERSION,
        "task": task,
        "has_candidates": has_candidates,
        "recommendation": if has_candidates {
            "review_existing_capability_before_implementing"
        } else {
            "no_similar_capability_found"
        },
        "next_actions": if has_candidates {
            json!(["open_candidate_file_lines", "prefer_reuse_or_extension"])
        } else {
            json!(["verify_domain_context", "continue_implementation_if_no_conflict"])
        },
        "candidates": search_result["results"].clone(),
    }))
}

fn mcp_diagnose_query(
    repo_root: &Path,
    input: &serde_json::Value,
    auto_index: bool,
) -> Result<serde_json::Value> {
    let query = input
        .get("query")
        .or_else(|| input.get("task"))
        .and_then(|value| value.as_str())
        .context("capfind_diagnose_query requires string field `query`")?;
    let limit = input
        .get("limit")
        .and_then(|value| value.as_u64())
        .unwrap_or(10) as usize;
    let lang = input
        .get("lang")
        .and_then(|value| value.as_str())
        .and_then(parse_lang_filter);
    let kind = input
        .get("kind")
        .and_then(|value| value.as_str())
        .and_then(parse_kind_filter);
    let path = input
        .get("path")
        .and_then(|value| value.as_str())
        .map(str::to_string);

    let response = diagnose_query_json(repo_root, query, limit, &lang, &kind, &path, auto_index)?;
    record_query_history_from_diagnosis(
        repo_root,
        "mcp",
        "capfind_diagnose_query",
        query,
        limit,
        &lang,
        &kind,
        path.as_deref(),
        &response,
    );
    Ok(response)
}

fn mcp_diagnose_file(
    repo_root: &Path,
    input: &serde_json::Value,
    auto_index: bool,
) -> Result<serde_json::Value> {
    let file = input
        .get("file")
        .or_else(|| input.get("path"))
        .and_then(|value| value.as_str())
        .context("capfind_diagnose_file requires string field `file`")?;
    diagnose_file_json_for_tool(repo_root, Path::new(file), auto_index)
}

fn mcp_map(
    repo_root: &Path,
    input: &serde_json::Value,
    auto_index: bool,
) -> Result<serde_json::Value> {
    let module = input.get("module").and_then(|value| value.as_str());
    let limit = input
        .get("limit")
        .and_then(|value| value.as_u64())
        .unwrap_or(200) as usize;
    let index_path = repo_root.join(".capfind/index.cfi");
    let (_header, body) = load_index_for_tool(repo_root, auto_index, &index_path)?;
    Ok(code_map::asset_map_json_with_repo(
        &body, repo_root, module, limit,
    ))
}

fn mcp_record_adoption(repo_root: &Path, input: &serde_json::Value) -> Result<serde_json::Value> {
    use serde_json::json;
    let task = input
        .get("task")
        .and_then(|value| value.as_str())
        .context("capfind_record_adoption requires string field `task`")?;
    let candidate_id = input
        .get("candidate_id")
        .and_then(|value| value.as_u64())
        .context("capfind_record_adoption requires integer field `candidate_id`")?
        as usize;
    let adopted = input
        .get("adopted")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    let files = input
        .get("files")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let note = input.get("note").and_then(|value| value.as_str());
    let session_id = input.get("session_id").and_then(|value| value.as_str());
    let rejected_reason = input
        .get("rejected_reason")
        .or_else(|| input.get("reason"))
        .and_then(|value| value.as_str());
    let stage = input
        .get("stage")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if adopted {
                "adopted".to_string()
            } else {
                "rejected".to_string()
            }
        });

    let index_path = repo_root.join(".capfind/index.cfi");
    let (_header, body) = load_index(&index_path).context("Failed to load index")?;
    let candidate = body
        .capabilities
        .get(candidate_id)
        .context(format!("No capability with id={candidate_id}"))?;
    adoption::record_event_with_source(
        repo_root,
        adoption::EventRecord {
            source: "mcp",
            task,
            candidate,
            stage: &stage,
            files: &files,
            note,
            rejected_reason,
            session_id,
        },
    )?;
    let normalized_stage = adoption::normalize_stage(&stage).unwrap_or("unknown");
    Ok(json!({
        "recorded": true,
        "candidate_id": candidate.id,
        "stage": normalized_stage,
        "session_id": session_id,
        "adopted": adoption::adopted_for_stage(normalized_stage),
        "rejected_reason": if normalized_stage == "rejected" {
            json!(adoption::normalize_rejection_reason(rejected_reason))
        } else {
            serde_json::Value::Null
        },
        "adoption": adoption::summary(repo_root),
    }))
}

fn mcp_detect_adoption(
    repo_root: &Path,
    input: &serde_json::Value,
    auto_index: bool,
) -> Result<serde_json::Value> {
    let since = input
        .get("since")
        .and_then(|value| value.as_str())
        .unwrap_or("HEAD");
    let task = input.get("task").and_then(|value| value.as_str());
    let candidate_ids = mcp_candidate_ids(input);
    let dry_run = input
        .get("dry_run")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let session_id = input.get("session_id").and_then(|value| value.as_str());

    let index_path = repo_root.join(".capfind/index.cfi");
    let (_header, body) = load_index_for_tool(repo_root, auto_index, &index_path)?;
    adoption::detect_from_git_diff(
        repo_root,
        &body.capabilities,
        since,
        task,
        &candidate_ids,
        session_id,
        dry_run,
    )
}

fn mcp_candidate_ids(input: &serde_json::Value) -> Vec<u32> {
    if let Some(values) = input
        .get("candidate_ids")
        .and_then(|value| value.as_array())
    {
        return values
            .iter()
            .filter_map(|value| value.as_u64().map(|id| id as u32))
            .collect();
    }
    input
        .get("candidate_id")
        .and_then(|value| value.as_u64())
        .map(|id| vec![id as u32])
        .unwrap_or_default()
}

fn capability_json(cap: &Capability, score: f32) -> serde_json::Value {
    use serde_json::json;
    let mut obj = json!({
        "id": cap.id,
        "score": (score * 10.0).round() / 10.0,
        "kind": cap.kind.as_str(),
        "lang": cap.lang.as_str(),
        "class": cap.class,
        "method": cap.method,
        "signature": cap.signature,
        "annotations": cap.annotations,
        "tags": cap.tags,
        "doc": cap.doc,
        "is_reference": cap.tags.iter().any(|tag| tag == "external"),
        "file": cap.file,
        "line": cap.line,
    });
    if let Some(ref http) = cap.http {
        obj["http"] = json!({"method": http.method, "path": http.path});
    }
    if let Some(ref rpc) = cap.rpc {
        obj["rpc"] = json!({
            "service": rpc.service,
            "rpc": rpc.rpc,
            "req": rpc.req,
            "rsp": rpc.rsp,
            "proto_file": rpc.proto_file,
        });
    }
    obj
}

/// `capfind show` — display full capability details.
pub fn show(
    repo_root: &Path,
    id: u32,
    _no_color: bool,
    task: Option<&str>,
    record_inspected: bool,
    session_id: Option<&str>,
) -> Result<()> {
    if record_inspected && task.is_none() {
        bail!("--record-inspected requires --task");
    }
    let index_path = repo_root.join(".capfind/index.cfi");
    let (_header, body) = load_index(&index_path).context("Failed to load index")?;

    let cap = body
        .capabilities
        .get(id as usize)
        .context(format!("No capability with id={id}"))?;

    println!("ID:         {}", cap.id);
    println!("Kind:       {}", cap.kind.as_str());
    println!("Language:   {}", cap.lang.as_str());
    println!("Module:     {}", cap.module);
    println!("Package:    {}", cap.package);
    if let Some(ref c) = cap.class {
        println!("Class:      {}", c);
    }
    println!("Method:     {}", cap.method);
    println!("Signature:  {}", cap.signature);
    if let Some(ref http) = cap.http {
        println!("HTTP:       {} {}", http.method, http.path);
    }
    if let Some(ref rpc) = cap.rpc {
        println!("RPC:        {}.{}", rpc.service, rpc.rpc);
        println!("RPC Types:  {} -> {}", rpc.req, rpc.rsp);
    }
    if let Some(ref doc) = cap.doc {
        println!("Doc:        {}", doc);
    }
    if !cap.annotations.is_empty() {
        println!("Annotations: {:?}", cap.annotations);
    }
    println!("File:       {}:{}", cap.file, cap.line);
    if record_inspected {
        let files = Vec::<String>::new();
        adoption::record_event_with_source(
            repo_root,
            adoption::EventRecord {
                source: "cli",
                task: task.unwrap_or_default(),
                candidate: cap,
                stage: "inspected",
                files: &files,
                note: Some("capfind show displayed candidate detail"),
                rejected_reason: None,
                session_id,
            },
        )?;
        println!("Adoption:  recorded inspected event");
    }
    Ok(())
}

/// `capfind record-adoption` — append one generated-code outcome event.
pub fn record_adoption(repo_root: &Path, args: &RecordAdoptionArgs) -> Result<()> {
    let index_path = repo_root.join(".capfind/index.cfi");
    let (_header, body) = load_index(&index_path).context("Failed to load index")?;
    let candidate = body
        .capabilities
        .get(args.candidate_id as usize)
        .context(format!("No capability with id={}", args.candidate_id))?;
    if args.rejected
        && args
            .stage
            .is_some_and(|stage| stage != AdoptionStage::Rejected)
    {
        bail!("--rejected can only be combined with --stage rejected");
    }
    let stage = args
        .stage
        .map(AdoptionStage::as_str)
        .unwrap_or(if args.rejected { "rejected" } else { "adopted" });
    adoption::record_event_with_source(
        repo_root,
        adoption::EventRecord {
            source: "manual",
            task: &args.task,
            candidate,
            stage,
            files: &args.file,
            note: args.note.as_deref(),
            rejected_reason: args.rejected_reason.as_deref(),
            session_id: args.session_id.as_deref(),
        },
    )?;
    let output = serde_json::json!({
        "recorded": true,
        "candidate_id": candidate.id,
        "stage": stage,
        "session_id": args.session_id.as_deref(),
        "adopted": adoption::adopted_for_stage(stage),
        "rejected_reason": if stage == "rejected" {
            serde_json::json!(adoption::normalize_rejection_reason(args.rejected_reason.as_deref()))
        } else {
            serde_json::Value::Null
        },
        "adoption": adoption::summary(repo_root),
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// `capfind detect-adoption` — detect generated-code adoption from git diff.
pub fn detect_adoption(repo_root: &Path, args: &DetectAdoptionArgs) -> Result<()> {
    let index_path = repo_root.join(".capfind/index.cfi");
    let (_header, body) = load_index_for_tool(
        repo_root,
        effective_auto_index(args.auto_index, args.no_auto_index),
        &index_path,
    )?;
    let output = adoption::detect_from_git_diff(
        repo_root,
        &body.capabilities,
        &args.since,
        args.task.as_deref(),
        &args.candidate_id,
        args.session_id.as_deref(),
        args.dry_run,
    )?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// `capfind stats` — print index statistics.
pub fn stats(repo_root: &Path) -> Result<()> {
    let index_path = repo_root.join(".capfind/index.cfi");
    let (header, body) = load_index(&index_path).context("Failed to load index")?;

    let size = std::fs::metadata(&index_path).map(|m| m.len()).unwrap_or(0);
    let n = body.capabilities.len();

    println!("capfind index stats");
    println!("  Path:          {}", index_path.display());
    println!("  Version:       {}", header.version);
    println!("  Created:       {}", header.created_unix);
    println!("  Capabilities:  {}", n);
    println!("  Vocab size:    {}", body.vocab.len());
    println!("  Avg doc len:   {:.1}", body.avgdl);
    println!("  File size:     {:.1} KB", size as f64 / 1024.0);

    // Breakdown by kind.
    let mut by_kind = std::collections::HashMap::new();
    let mut by_lang = std::collections::HashMap::new();
    for cap in &body.capabilities {
        *by_kind.entry(cap.kind.as_str()).or_insert(0u32) += 1;
        *by_lang.entry(cap.lang.as_str()).or_insert(0u32) += 1;
    }
    println!("  By kind:");
    for (k, v) in &by_kind {
        println!("    {:<16} {}", k, v);
    }
    println!("  By language:");
    for (k, v) in &by_lang {
        println!("    {:<8} {}", k, v);
    }

    Ok(())
}

/// `capfind doctor` — explain scan/index health.
pub fn doctor(repo_root: &Path, args: &DoctorArgs) -> Result<()> {
    let summary = dashboard_summary(repo_root)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }

    println!("capfind doctor");
    println!("  Repo:        {}", repo_root.display());
    println!(
        "  Index:       {}",
        if summary["index"]["present"].as_bool().unwrap_or(false) {
            "present"
        } else {
            "missing"
        }
    );
    println!("  Roots:       {}", summary["config"]["roots"]);
    println!("  Include:     {}", summary["config"]["include"]);
    println!("  Files:       {}", summary["index"]["indexed_files"]);
    println!("  Assets:      {}", summary["index"]["capabilities"]);
    println!("  External:    {}", summary["coverage"]["external_assets"]);
    println!("  Adoption:    {}", summary["adoption"]["adoption_rate"]);
    Ok(())
}

/// `capfind diagnose-query` — explain search health for one task/query.
pub fn diagnose_query(repo_root: &Path, args: &DiagnoseQueryArgs) -> Result<()> {
    let query = args.query.join(" ");
    if query.is_empty() {
        bail!("Please provide a query, e.g.: capfind diagnose-query mdm query");
    }
    let response = diagnose_query_json(
        repo_root,
        &query,
        args.limit,
        &args.lang,
        &args.kind,
        &args.path,
        effective_auto_index(args.auto_index, args.no_auto_index),
    )?;
    record_query_history_from_diagnosis(
        repo_root,
        "cli",
        "diagnose_query",
        &query,
        args.limit,
        &args.lang,
        &args.kind,
        args.path.as_deref(),
        &response,
    );
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

/// `capfind diagnose-file` — explain parser/index coverage for one file.
pub fn diagnose_file(repo_root: &Path, args: &DiagnoseFileArgs) -> Result<()> {
    let response = diagnose_file_json_for_tool(
        repo_root,
        &args.file,
        effective_auto_index(args.auto_index, args.no_auto_index),
    )?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn diagnose_file_json_for_tool(
    repo_root: &Path,
    file: &Path,
    auto_index: bool,
) -> Result<serde_json::Value> {
    let cfg = config::load(repo_root)?;
    let index_path = repo_root.join(".capfind/index.cfi");
    let (index_present, index_body, index_error) =
        load_index_for_diagnosis(repo_root, auto_index, &index_path);
    file_diagnosis::diagnose_file_json(
        repo_root,
        file,
        &cfg,
        index_body.as_ref(),
        index_present,
        index_error.as_deref(),
    )
}

fn load_index_for_diagnosis(
    repo_root: &Path,
    auto_index: bool,
    index_path: &Path,
) -> (bool, Option<IndexBody>, Option<String>) {
    let mut index_present = index_path.exists();
    let mut index_error = None;
    if !index_present && auto_index {
        if let Err(err) = build_index_file(repo_root, false, false) {
            index_error = Some(format!("auto-index failed: {err}"));
        }
        index_present = index_path.exists();
    }

    let body = if index_present {
        match load_index(index_path) {
            Ok((_header, body)) => Some(body),
            Err(err) => {
                let message = format!("{err:#}");
                index_error = Some(match index_error {
                    Some(previous) => format!("{previous}; load failed: {message}"),
                    None => message,
                });
                None
            }
        }
    } else {
        None
    };
    (index_present, body, index_error)
}

/// `capfind dashboard` — generate a local HTML dashboard for repository capability review.
pub fn dashboard(repo_root: &Path, args: &DashboardArgs) -> Result<()> {
    let view = DashboardViewOptions::from_args(args);
    let summary = dashboard_summary_with_options(
        repo_root,
        args.record_history || !args.no_record_history,
        &view,
    )?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }

    let output_path = args
        .output
        .clone()
        .unwrap_or_else(|| repo_root.join(".capfind/dashboard.html"));
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output_path, dashboard_html(&summary))?;
    println!("Wrote {}", output_path.display());
    Ok(())
}

/// `capfind map` — emit a machine-readable asset/service map.
pub fn asset_map(repo_root: &Path, args: &MapArgs) -> Result<()> {
    let index_path = repo_root.join(".capfind/index.cfi");
    let (_header, body) = load_index_for_tool(
        repo_root,
        effective_auto_index(args.auto_index, args.no_auto_index),
        &index_path,
    )?;
    let map =
        code_map::asset_map_json_with_repo(&body, repo_root, args.module.as_deref(), args.limit);
    println!("{}", serde_json::to_string_pretty(&map)?);
    Ok(())
}

/// `capfind eval` — run golden query/file fixtures and emit quality metrics.
pub fn eval(repo_root: &Path, args: &EvalArgs) -> Result<()> {
    let cfg = config::load(repo_root)?;
    let index_path = repo_root.join(".capfind/index.cfi");
    let index_present_before = index_path.exists();
    let (_header, body) = load_index_for_tool(
        repo_root,
        effective_auto_index(args.auto_index, args.no_auto_index),
        &index_path,
    )?;
    let mut output = quality_eval::eval_json(repo_root, &body, &cfg, args, index_present_before)?;
    let suite = eval_suite_name(repo_root, args);
    output["suite"] = serde_json::json!(suite);
    if args.record_history || !args.no_record_history {
        append_eval_history(repo_root, &output, &suite)?;
    }
    output["history"] = eval_history_summary(repo_root, Some(&suite))?;
    println!("{}", serde_json::to_string_pretty(&output)?);

    if eval_threshold_failed(&output, args) {
        use std::io::Write;
        std::io::stdout().flush()?;
        std::process::exit(2);
    }
    Ok(())
}

fn eval_suite_name(repo_root: &Path, args: &EvalArgs) -> String {
    args.suite
        .as_deref()
        .map(str::trim)
        .filter(|suite| !suite.is_empty())
        .map(str::to_string)
        .or_else(|| {
            repo_root
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "default".to_string())
}

fn append_eval_history(repo_root: &Path, output: &serde_json::Value, suite: &str) -> Result<()> {
    use std::io::Write;

    let path = repo_root.join(EVAL_HISTORY_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let snapshot = eval_history_snapshot(output, suite);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{}", snapshot)?;
    Ok(())
}

fn eval_history_summary(repo_root: &Path, suite: Option<&str>) -> Result<serde_json::Value> {
    let path = repo_root.join(EVAL_HISTORY_FILE);
    let entries = read_jsonl_values(&path)?;
    let valid_entries = entries
        .into_iter()
        .filter(|entry| entry["schema_version"] == EVAL_HISTORY_SCHEMA_VERSION)
        .collect::<Vec<_>>();
    let scoped_entries = valid_entries
        .iter()
        .filter(|entry| match suite {
            Some(suite) => entry["suite"].as_str() == Some(suite),
            None => true,
        })
        .cloned()
        .collect::<Vec<_>>();
    let total_entries = scoped_entries.len();
    let start = total_entries.saturating_sub(EVAL_HISTORY_LIMIT);
    Ok(serde_json::json!({
        "schema_version": EVAL_HISTORY_SCHEMA_VERSION,
        "path": path.display().to_string(),
        "entries": valid_entries.len(),
        "suite": suite,
        "suite_entries": scoped_entries.len(),
        "latest": scoped_entries.last().cloned().unwrap_or(serde_json::Value::Null),
        "recent": scoped_entries[start..].to_vec(),
        "trend": eval_history_trend(&scoped_entries),
    }))
}

fn eval_history_snapshot(output: &serde_json::Value, suite: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": EVAL_HISTORY_SCHEMA_VERSION,
        "created_unix": now_unix(),
        "suite": suite,
        "repo_root": output["repo_root"].clone(),
        "index": {
            "capabilities": value_u64(&output["index"]["capabilities"]),
            "indexed_files": value_u64(&output["index"]["indexed_files"]),
            "present_before": output["index"]["present_before"].as_bool().unwrap_or(false),
        },
        "fixtures": output["fixtures"].clone(),
        "metrics": output["metrics"].clone(),
        "failed": {
            "query": output["metrics"]["query"]["failed_case_ids"].clone(),
            "file": output["metrics"]["file"]["failed_case_ids"].clone(),
            "edge": output["metrics"]["edge"]["failed_case_ids"].clone(),
            "diagnostic": output["metrics"]["diagnostic"]["failed_case_ids"].clone(),
            "ownership": output["metrics"]["ownership"]["failed_case_ids"].clone(),
        },
    })
}

fn eval_history_trend(entries: &[serde_json::Value]) -> serde_json::Value {
    let Some(first) = entries.first() else {
        return serde_json::json!({
            "samples": 0,
            "recall_at_k_delta": 0.0,
            "mrr_delta": 0.0,
            "known_precision_at_k_delta": 0.0,
            "file_pass_delta": 0.0,
            "edge_pass_delta": 0.0,
            "diagnostic_pass_delta": 0.0,
            "ownership_pass_delta": 0.0,
        });
    };
    let last = entries.last().unwrap_or(first);
    serde_json::json!({
        "samples": entries.len(),
        "recall_at_k_delta": delta_f64(&last["metrics"]["query"]["recall_at_k"], &first["metrics"]["query"]["recall_at_k"]),
        "mrr_delta": delta_f64(&last["metrics"]["query"]["mrr"], &first["metrics"]["query"]["mrr"]),
        "known_precision_at_k_delta": delta_f64(&last["metrics"]["query"]["known_precision_at_k"], &first["metrics"]["query"]["known_precision_at_k"]),
        "file_pass_delta": delta_f64(&last["metrics"]["file"]["diagnosis_pass_rate"], &first["metrics"]["file"]["diagnosis_pass_rate"]),
        "edge_pass_delta": delta_f64(&last["metrics"]["edge"]["edge_pass_rate"], &first["metrics"]["edge"]["edge_pass_rate"]),
        "diagnostic_pass_delta": delta_f64(&last["metrics"]["diagnostic"]["diagnostic_pass_rate"], &first["metrics"]["diagnostic"]["diagnostic_pass_rate"]),
        "ownership_pass_delta": delta_f64(&last["metrics"]["ownership"]["ownership_pass_rate"], &first["metrics"]["ownership"]["ownership_pass_rate"]),
    })
}

fn eval_threshold_failed(output: &serde_json::Value, args: &EvalArgs) -> bool {
    let recall_failed = args.fail_under_recall.is_some_and(|threshold| {
        metric_below(&output["metrics"]["query"]["recall_at_k"], threshold)
    });
    let file_failed = args.fail_under_file_pass.is_some_and(|threshold| {
        metric_below(&output["metrics"]["file"]["diagnosis_pass_rate"], threshold)
    });
    let edge_failed = args.fail_under_edge_pass.is_some_and(|threshold| {
        metric_below(&output["metrics"]["edge"]["edge_pass_rate"], threshold)
    });
    let diagnostic_failed = args.fail_under_diagnostic_pass.is_some_and(|threshold| {
        metric_below(
            &output["metrics"]["diagnostic"]["diagnostic_pass_rate"],
            threshold,
        )
    });
    let ownership_failed = args.fail_under_ownership_pass.is_some_and(|threshold| {
        metric_below(
            &output["metrics"]["ownership"]["ownership_pass_rate"],
            threshold,
        )
    });
    recall_failed || file_failed || edge_failed || diagnostic_failed || ownership_failed
}

fn metric_below(value: &serde_json::Value, threshold: f64) -> bool {
    value
        .as_f64()
        .is_some_and(|metric| metric + f64::EPSILON < threshold)
}

fn dashboard_summary(repo_root: &Path) -> Result<serde_json::Value> {
    dashboard_summary_with_history(repo_root, false)
}

fn dashboard_summary_with_history(
    repo_root: &Path,
    record_history: bool,
) -> Result<serde_json::Value> {
    dashboard_summary_with_options(repo_root, record_history, &DashboardViewOptions::default())
}

#[derive(Default)]
struct DashboardViewOptions {
    module: Option<String>,
    relationships: Vec<String>,
    graph_limit: usize,
}

impl DashboardViewOptions {
    fn from_args(args: &DashboardArgs) -> Self {
        Self {
            module: args
                .module
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            relationships: normalized_relationship_filters(&args.relationships),
            graph_limit: args.graph_limit.clamp(1, 80),
        }
    }

    fn graph_limit(&self) -> usize {
        if self.graph_limit == 0 {
            16
        } else {
            self.graph_limit
        }
    }
}

fn normalized_relationship_filters(values: &[String]) -> Vec<String> {
    let mut filters = values
        .iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    filters.sort();
    filters.dedup();
    filters
}

fn dashboard_summary_with_options(
    repo_root: &Path,
    record_history: bool,
    view: &DashboardViewOptions,
) -> Result<serde_json::Value> {
    use serde_json::json;

    let cfg = config::load(repo_root)?;
    let index_path = repo_root.join(".capfind/index.cfi");
    let mut index = json!({
        "present": false,
        "path": index_path.display().to_string(),
        "capabilities": 0,
        "indexed_files": 0,
        "vocab": 0,
        "file_size_kb": 0.0,
    });
    let mut coverage = json!({
        "by_kind": {},
        "by_lang": {},
        "by_type": {},
        "top_modules": [],
        "top_packages": [],
        "external_assets": 0,
    });
    let mut asset_map = json!({
        "schema_version": code_map::ASSET_MAP_SCHEMA_VERSION,
        "filters": {"module": view.module.as_deref(), "limit": 80},
        "totals": {
            "capabilities": 0,
            "modules": 0,
            "packages": 0,
            "external_assets": 0,
            "indexed_files": 0,
        },
        "modules": [],
        "external": {"dependencies": [], "api_usages": [], "jar_methods": []},
        "services": {"configured": 0, "items": []},
        "graph": {"node_count": 0, "edge_count": 0, "nodes": [], "edges": [], "diagnostics": []},
    });

    if index_path.exists() {
        let (header, body) = load_index(&index_path).context("Failed to load index")?;
        let size = std::fs::metadata(&index_path).map(|m| m.len()).unwrap_or(0);
        let by_kind = count_by(body.capabilities.iter().map(|cap| cap.kind.as_str()));
        let by_lang = count_by(body.capabilities.iter().map(|cap| cap.lang.as_str()));
        let by_type = count_by(body.capabilities.iter().map(ai_context::candidate_type));
        let external_assets = body
            .capabilities
            .iter()
            .filter(|cap| ai_context::is_external(cap))
            .count();
        index = json!({
            "present": true,
            "path": index_path.display().to_string(),
            "version": header.version,
            "created_unix": header.created_unix,
            "capabilities": body.capabilities.len(),
            "indexed_files": body.file_stats.len(),
            "vocab": body.vocab.len(),
            "file_size_kb": (size as f64 / 102.4).round() / 10.0,
        });
        coverage = json!({
            "by_kind": by_kind,
            "by_lang": by_lang,
            "by_type": by_type,
            "top_modules": top_counts(body.capabilities.iter().map(|cap| cap.module.as_str())),
            "top_packages": top_counts(body.capabilities.iter().map(|cap| cap.package.as_str())),
            "external_assets": external_assets,
        });
        let graph_limit = view.graph_limit().saturating_mul(6).max(80);
        asset_map = code_map::asset_map_json_with_repo(
            &body,
            repo_root,
            view.module.as_deref(),
            graph_limit,
        );
    }

    let adoption = adoption::summary(repo_root);
    let queries = query_history_summary(repo_root)?;
    let evals = eval_history_summary(repo_root, None)?;
    let adopted = adoption
        .get("adopted")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    let mut summary = json!({
        "schema_version": "capfind.dashboard.v1",
        "repo_root": repo_root.display().to_string(),
        "config": {
            "roots": cfg.index.roots,
            "include": cfg.index.include,
        },
        "view": {
            "filters": {
                "module": view.module.as_deref(),
                "relationships": &view.relationships,
                "graph_limit": view.graph_limit(),
            }
        },
        "index": index,
        "coverage": coverage,
        "asset_map": asset_map,
        "architecture": architecture_summary(&asset_map),
        "topology": topology_summary(&asset_map, view),
        "adoption": adoption,
        "queries": queries,
        "evals": evals,
        "impact": {
            "estimated_tokens_saved": adopted * 1500,
            "north_star": "final_code_called_candidate"
        }
    });
    if record_history {
        append_dashboard_history(repo_root, &summary)?;
    }
    summary["history"] = dashboard_history(repo_root)?;
    Ok(summary)
}

fn append_dashboard_history(repo_root: &Path, summary: &serde_json::Value) -> Result<()> {
    use std::io::Write;

    let path = repo_root.join(DASHBOARD_HISTORY_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let snapshot = dashboard_history_snapshot(summary);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{}", snapshot)?;
    Ok(())
}

fn dashboard_history(repo_root: &Path) -> Result<serde_json::Value> {
    let path = repo_root.join(DASHBOARD_HISTORY_FILE);
    let entries = read_dashboard_history(&path)?;
    let total_entries = entries.len();
    let start = total_entries.saturating_sub(DASHBOARD_HISTORY_LIMIT);
    let recent = entries[start..].to_vec();
    Ok(serde_json::json!({
        "path": path.display().to_string(),
        "entries": total_entries,
        "recent": recent,
        "trend": dashboard_history_trend(&entries),
    }))
}

fn read_dashboard_history(path: &Path) -> Result<Vec<serde_json::Value>> {
    read_jsonl_values(path)
}

#[derive(Default)]
struct QueryRollup {
    events: u64,
    zero_result_events: u64,
    total_results: u64,
    latest_event_unix: u64,
    sources: BTreeMap<String, u64>,
    tools: BTreeMap<String, u64>,
}

#[derive(Default)]
struct QueryBucket {
    events: u64,
    zero_result_events: u64,
    total_results: u64,
    queries: BTreeSet<String>,
}

fn query_history_summary(repo_root: &Path) -> Result<serde_json::Value> {
    let path = repo_root.join(QUERY_HISTORY_FILE);
    let entries = read_jsonl_values(&path)?;
    let mut valid_entries = Vec::new();
    let mut events = 0u64;
    let mut zero_result_events = 0u64;
    let mut by_source = BTreeMap::<String, u64>::new();
    let mut by_tool = BTreeMap::<String, u64>::new();
    let mut by_query = BTreeMap::<String, QueryRollup>::new();
    let mut timeline = BTreeMap::<u64, QueryBucket>::new();

    for value in entries {
        if value["schema_version"] != QUERY_EVENT_SCHEMA_VERSION {
            continue;
        }
        let Some(query) = value.get("query").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let query = query.trim();
        if query.is_empty() {
            continue;
        }
        let source = value
            .get("source")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let tool = value
            .get("tool")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let result_count = value_u64(&value["result_count"]);
        let is_zero = result_count == 0
            || value
                .get("zero_results")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
        let created_unix = value
            .get("created_unix")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default()
            .max(0) as u64;

        events += 1;
        if is_zero {
            zero_result_events += 1;
        }
        *by_source.entry(source.to_string()).or_default() += 1;
        *by_tool.entry(tool.to_string()).or_default() += 1;

        let rollup = by_query.entry(query.to_string()).or_default();
        rollup.events += 1;
        rollup.total_results += result_count;
        if is_zero {
            rollup.zero_result_events += 1;
        }
        rollup.latest_event_unix = rollup.latest_event_unix.max(created_unix);
        *rollup.sources.entry(source.to_string()).or_default() += 1;
        *rollup.tools.entry(tool.to_string()).or_default() += 1;

        let unix_day = created_unix / 86_400;
        let bucket = timeline.entry(unix_day).or_default();
        bucket.events += 1;
        bucket.total_results += result_count;
        if is_zero {
            bucket.zero_result_events += 1;
        }
        bucket.queries.insert(query.to_string());

        valid_entries.push(value);
    }

    let total_entries = valid_entries.len();
    let recent_start = total_entries.saturating_sub(QUERY_HISTORY_LIMIT);
    let timeline_rows = query_timeline_rows(&timeline);
    Ok(serde_json::json!({
        "schema_version": QUERY_HISTORY_SCHEMA_VERSION,
        "path": path.display().to_string(),
        "events": events,
        "unique_queries": by_query.len(),
        "zero_result_events": zero_result_events,
        "zero_result_rate": if events == 0 { 0.0 } else { round_float(zero_result_events as f64 / events as f64) },
        "by_source": by_source,
        "by_tool": by_tool,
        "top_queries": query_rollup_rows(&by_query, false),
        "zero_result_queries": query_rollup_rows(&by_query, true),
        "recent": valid_entries[recent_start..].to_vec(),
        "timeline": timeline_rows,
    }))
}

fn query_rollup_rows(
    rollups: &BTreeMap<String, QueryRollup>,
    zero_only: bool,
) -> Vec<serde_json::Value> {
    let mut rows = rollups
        .iter()
        .filter(|(_, rollup)| !zero_only || rollup.zero_result_events > 0)
        .map(|(query, rollup)| {
            serde_json::json!({
                "query": query,
                "events": rollup.events,
                "zero_result_events": rollup.zero_result_events,
                "zero_result_rate": if rollup.events == 0 { 0.0 } else { round_float(rollup.zero_result_events as f64 / rollup.events as f64) },
                "average_results": if rollup.events == 0 { 0.0 } else { round_float(rollup.total_results as f64 / rollup.events as f64) },
                "latest_event_unix": rollup.latest_event_unix,
                "sources": rollup.sources,
                "tools": rollup.tools,
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        let primary = if zero_only {
            b["zero_result_events"]
                .as_u64()
                .cmp(&a["zero_result_events"].as_u64())
        } else {
            b["events"].as_u64().cmp(&a["events"].as_u64())
        };
        primary
            .then(
                b["latest_event_unix"]
                    .as_u64()
                    .cmp(&a["latest_event_unix"].as_u64()),
            )
            .then(a["query"].as_str().cmp(&b["query"].as_str()))
    });
    rows.truncate(20);
    rows
}

fn query_timeline_rows(buckets: &BTreeMap<u64, QueryBucket>) -> Vec<serde_json::Value> {
    let mut rows = buckets
        .iter()
        .map(|(unix_day, bucket)| {
            serde_json::json!({
                "unix_day": unix_day,
                "events": bucket.events,
                "unique_queries": bucket.queries.len(),
                "zero_result_events": bucket.zero_result_events,
                "average_results": if bucket.events == 0 { 0.0 } else { round_float(bucket.total_results as f64 / bucket.events as f64) },
            })
        })
        .collect::<Vec<_>>();
    let start = rows.len().saturating_sub(30);
    rows.split_off(start)
}

fn read_jsonl_values(path: &Path) -> Result<Vec<serde_json::Value>> {
    use std::io::BufRead;

    let Ok(file) = std::fs::File::open(path) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for line in std::io::BufReader::new(file)
        .lines()
        .map_while(|line| line.ok())
    {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        entries.push(value);
    }
    Ok(entries)
}

fn dashboard_history_snapshot(summary: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema_version": DASHBOARD_HISTORY_SCHEMA_VERSION,
        "created_unix": now_unix(),
        "index": {
            "capabilities": value_u64(&summary["index"]["capabilities"]),
            "indexed_files": value_u64(&summary["index"]["indexed_files"]),
            "file_size_kb": value_f64(&summary["index"]["file_size_kb"]),
        },
        "coverage": {
            "external_assets": value_u64(&summary["coverage"]["external_assets"]),
            "top_modules": summary["coverage"]["top_modules"].clone(),
            "by_type": summary["coverage"]["by_type"].clone(),
        },
        "architecture": {
            "modules": value_u64(&summary["asset_map"]["totals"]["modules"]),
            "edge_count": value_u64(&summary["asset_map"]["graph"]["edge_count"]),
            "edge_counts": summary["architecture"]["edge_counts"].clone(),
        },
        "modules": dashboard_history_modules(summary),
        "adoption": {
            "events": value_u64(&summary["adoption"]["events"]),
            "final_events": value_u64(&summary["adoption"]["final_events"]),
            "adopted": value_u64(&summary["adoption"]["adopted"]),
            "rejected": value_u64(&summary["adoption"]["rejected"]),
            "adoption_rate": value_f64(&summary["adoption"]["adoption_rate"]),
        },
        "queries": {
            "events": value_u64(&summary["queries"]["events"]),
            "unique_queries": value_u64(&summary["queries"]["unique_queries"]),
            "zero_result_events": value_u64(&summary["queries"]["zero_result_events"]),
            "zero_result_rate": value_f64(&summary["queries"]["zero_result_rate"]),
        },
        "evals": {
            "entries": value_u64(&summary["evals"]["entries"]),
            "latest_recall_at_k": value_f64(&summary["evals"]["latest"]["metrics"]["query"]["recall_at_k"]),
            "latest_edge_pass_rate": value_f64(&summary["evals"]["latest"]["metrics"]["edge"]["edge_pass_rate"]),
            "latest_diagnostic_pass_rate": value_f64(&summary["evals"]["latest"]["metrics"]["diagnostic"]["diagnostic_pass_rate"]),
            "latest_ownership_pass_rate": value_f64(&summary["evals"]["latest"]["metrics"]["ownership"]["ownership_pass_rate"]),
        },
        "impact": {
            "estimated_tokens_saved": value_u64(&summary["impact"]["estimated_tokens_saved"]),
        },
    })
}

fn dashboard_history_modules(summary: &serde_json::Value) -> Vec<serde_json::Value> {
    summary["architecture"]["module_health"]
        .as_array()
        .map(|modules| {
            modules
                .iter()
                .take(20)
                .map(|module| {
                    serde_json::json!({
                        "module": module["module"].as_str().unwrap_or_default(),
                        "owner": module["owner"].as_str().unwrap_or_default(),
                        "service": module["service"].as_str().unwrap_or_default(),
                        "assets": value_u64(&module["assets"]),
                        "entrypoints": value_u64(&module["entrypoints"]),
                        "external_assets": value_u64(&module["external_assets"]),
                        "health": module["health"].as_str().unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn dashboard_history_trend(entries: &[serde_json::Value]) -> serde_json::Value {
    let Some(first) = entries.first() else {
        return serde_json::json!({
            "samples": 0,
            "capabilities_delta": 0,
            "indexed_files_delta": 0,
            "edge_count_delta": 0,
            "final_events_delta": 0,
            "adoption_rate_delta": 0.0,
            "query_events_delta": 0,
            "zero_result_events_delta": 0,
            "eval_entries_delta": 0,
            "estimated_tokens_saved_delta": 0,
        });
    };
    let last = entries.last().unwrap_or(first);
    serde_json::json!({
        "samples": entries.len(),
        "capabilities_delta": delta_u64(&last["index"]["capabilities"], &first["index"]["capabilities"]),
        "indexed_files_delta": delta_u64(&last["index"]["indexed_files"], &first["index"]["indexed_files"]),
        "edge_count_delta": delta_u64(&last["architecture"]["edge_count"], &first["architecture"]["edge_count"]),
        "final_events_delta": delta_u64(&last["adoption"]["final_events"], &first["adoption"]["final_events"]),
        "adoption_rate_delta": round_float(value_f64(&last["adoption"]["adoption_rate"]) - value_f64(&first["adoption"]["adoption_rate"])),
        "query_events_delta": delta_u64(&last["queries"]["events"], &first["queries"]["events"]),
        "zero_result_events_delta": delta_u64(&last["queries"]["zero_result_events"], &first["queries"]["zero_result_events"]),
        "eval_entries_delta": delta_u64(&last["evals"]["entries"], &first["evals"]["entries"]),
        "estimated_tokens_saved_delta": delta_u64(&last["impact"]["estimated_tokens_saved"], &first["impact"]["estimated_tokens_saved"]),
    })
}

fn value_u64(value: &serde_json::Value) -> u64 {
    value.as_u64().unwrap_or_default()
}

fn value_f64(value: &serde_json::Value) -> f64 {
    value.as_f64().unwrap_or_default()
}

fn delta_u64(last: &serde_json::Value, first: &serde_json::Value) -> i64 {
    value_u64(last) as i64 - value_u64(first) as i64
}

fn delta_f64(last: &serde_json::Value, first: &serde_json::Value) -> f64 {
    round_float(value_f64(last) - value_f64(first))
}

fn round_float(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn architecture_summary(asset_map: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "module_health": module_health_rows(asset_map),
        "edge_counts": count_rows(&asset_map["graph"]["relationship_counts"]),
        "diagnostics": service_map_diagnostics(&asset_map["graph"]["diagnostics"]),
    })
}

fn service_map_diagnostics(value: &serde_json::Value) -> serde_json::Value {
    let diagnostics = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(20)
                .map(|item| {
                    serde_json::json!({
                        "kind": item["kind"].as_str().unwrap_or_default(),
                        "reason": item["reason"].as_str().unwrap_or_default(),
                        "file": item["file"].as_str().unwrap_or_default(),
                        "line": value_u64(&item["line"]),
                        "symbol": item["call"].as_str().or_else(|| item["dependency_name"].as_str()).unwrap_or_default(),
                        "candidate_count": item["candidates"].as_array().map(|candidates| candidates.len()).unwrap_or_default(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({
        "total": value.as_array().map(|items| items.len()).unwrap_or_default(),
        "by_kind": count_by(diagnostics.iter().filter_map(|item| item["kind"].as_str())),
        "by_reason": count_by(diagnostics.iter().filter_map(|item| item["reason"].as_str())),
        "items": diagnostics,
    })
}

fn topology_summary(
    asset_map: &serde_json::Value,
    view: &DashboardViewOptions,
) -> serde_json::Value {
    use serde_json::json;

    let nodes = asset_map["graph"]["nodes"]
        .as_array()
        .map(|nodes| nodes.as_slice())
        .unwrap_or(&[]);
    let edges = asset_map["graph"]["edges"]
        .as_array()
        .map(|edges| edges.as_slice())
        .unwrap_or(&[]);

    let mut capability_nodes = BTreeMap::<String, TopologyNode>::new();
    let mut module_stats = BTreeMap::<String, ModuleTopologyStats>::new();
    for node in nodes {
        if node["kind"].as_str() != Some("capability") {
            continue;
        }
        let id = node["id"].as_str().unwrap_or_default().to_string();
        let module = node["module"].as_str().unwrap_or_default().to_string();
        let entrypoint = node["entrypoint"].is_object();
        let external = node["external"].as_bool().unwrap_or_default();
        let type_name = node["type"].as_str().unwrap_or_default().to_string();
        let framework = node["framework"].as_str().map(str::to_string);
        capability_nodes.insert(
            id,
            TopologyNode {
                module: module.clone(),
                name: node["name"].as_str().unwrap_or_default().to_string(),
                type_name: type_name.clone(),
                framework,
            },
        );
        let stat = module_stats.entry(module).or_default();
        stat.capabilities += 1;
        if entrypoint {
            stat.entrypoints += 1;
        }
        if external {
            stat.external_assets += 1;
        }
        *stat.by_type.entry(type_name).or_insert(0) += 1;
    }

    let mut module_links = BTreeMap::<(String, String, String), ModuleLinkStat>::new();
    let mut sample_edges = Vec::new();
    for edge in edges {
        let Some(relationship) = edge["relationship"].as_str() else {
            continue;
        };
        if matches!(
            relationship,
            "contains_package" | "contains_class" | "declares_capability"
        ) {
            continue;
        }
        let Some(from_id) = edge["from"].as_str() else {
            continue;
        };
        let Some(to_id) = edge["to"].as_str() else {
            continue;
        };
        let Some(from_node) = capability_nodes.get(from_id) else {
            continue;
        };
        let Some(to_node) = capability_nodes.get(to_id) else {
            continue;
        };

        let from_key = from_node.module.clone();
        let to_key = to_node.module.clone();
        let link = module_links
            .entry((from_key.clone(), to_key.clone(), relationship.to_string()))
            .or_default();
        link.count += 1;
        if link.sample_calls.len() < 3 {
            if let Some(call) = edge["evidence"]["call"].as_str() {
                link.sample_calls.push(call.to_string());
            } else {
                link.sample_calls.push(
                    edge["evidence"]["kind"]
                        .as_str()
                        .unwrap_or(relationship)
                        .to_string(),
                );
            }
        }

        let from_stat = module_stats.entry(from_key.clone()).or_default();
        from_stat.outbound_calls += 1;
        *from_stat
            .outbound_by_module
            .entry(to_key.clone())
            .or_insert(0) += 1;
        *from_stat
            .outbound_by_relationship
            .entry(relationship.to_string())
            .or_insert(0) += 1;

        let to_stat = module_stats.entry(to_key.clone()).or_default();
        to_stat.inbound_calls += 1;
        *to_stat.inbound_by_module.entry(from_key).or_insert(0) += 1;
        *to_stat
            .inbound_by_relationship
            .entry(relationship.to_string())
            .or_insert(0) += 1;

        if sample_edges.len() < 40 {
            sample_edges.push(json!({
                "from_module": from_node.module.as_str(),
                "from": from_node.name.as_str(),
                "from_type": from_node.type_name.as_str(),
                "from_framework": from_node.framework.as_deref(),
                "to_module": to_node.module.as_str(),
                "to": to_node.name.as_str(),
                "to_type": to_node.type_name.as_str(),
                "to_framework": to_node.framework.as_deref(),
                "relationship": relationship,
                "confidence": edge["confidence"].as_str().unwrap_or_default(),
                "call": edge["evidence"]["call"].as_str(),
            }));
        }
    }

    let mut module_links_rows = module_links
        .into_iter()
        .map(|((from_module, to_module, relationship), stat)| {
            json!({
                "from_module": from_module,
                "to_module": to_module,
                "relationship": relationship,
                "count": stat.count,
                "sample_calls": stat.sample_calls,
            })
        })
        .collect::<Vec<_>>();
    module_links_rows.sort_by(|a, b| {
        b["count"]
            .as_u64()
            .cmp(&a["count"].as_u64())
            .then(a["from_module"].as_str().cmp(&b["from_module"].as_str()))
            .then(a["to_module"].as_str().cmp(&b["to_module"].as_str()))
            .then(a["relationship"].as_str().cmp(&b["relationship"].as_str()))
    });

    let module_ownership = asset_map["modules"]
        .as_array()
        .map(|modules| {
            modules
                .iter()
                .filter_map(|module| Some((module["name"].as_str()?, module["ownership"].clone())))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();

    let mut module_drilldowns = module_stats
        .into_iter()
        .map(|(module, stat)| {
            let ownership = module_ownership
                .get(module.as_str())
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let service = asset_map["modules"]
                .as_array()
                .and_then(|modules| {
                    modules
                        .iter()
                        .find(|item| item["name"].as_str() == Some(module.as_str()))
                })
                .map(|item| item["service"].clone())
                .unwrap_or(serde_json::Value::Null);
            json!({
                "module": module,
                "ownership": ownership,
                "service": service,
                "capabilities": stat.capabilities,
                "entrypoints": stat.entrypoints,
                "external_assets": stat.external_assets,
                "outbound_calls": stat.outbound_calls,
                "inbound_calls": stat.inbound_calls,
                "primary_type": top_name(&stat.by_type),
                "outbound_by_module": top_counts_map(&stat.outbound_by_module, 5),
                "inbound_by_module": top_counts_map(&stat.inbound_by_module, 5),
                "outbound_by_relationship": top_counts_map(&stat.outbound_by_relationship, 5),
                "inbound_by_relationship": top_counts_map(&stat.inbound_by_relationship, 5),
            })
        })
        .collect::<Vec<_>>();
    module_drilldowns.sort_by(|a, b| {
        b["outbound_calls"]
            .as_u64()
            .cmp(&a["outbound_calls"].as_u64())
            .then(
                b["inbound_calls"]
                    .as_u64()
                    .cmp(&a["inbound_calls"].as_u64()),
            )
            .then(a["module"].as_str().cmp(&b["module"].as_str()))
    });

    let graph_view = topology_graph_view(&module_links_rows, &module_drilldowns, view);
    let ownership_drilldowns = ownership_drilldowns(asset_map, &module_drilldowns);

    serde_json::json!({
        "module_links": module_links_rows,
        "module_drilldowns": module_drilldowns,
        "graph_view": graph_view,
        "ownership_drilldowns": ownership_drilldowns,
        "sample_edges": sample_edges,
    })
}

fn topology_graph_view(
    module_links: &[serde_json::Value],
    module_drilldowns: &[serde_json::Value],
    view: &DashboardViewOptions,
) -> serde_json::Value {
    let selected_relationships = view
        .relationships
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let module_filter = view.module.as_deref();
    let module_lookup = module_drilldowns
        .iter()
        .filter_map(|module| Some((module["module"].as_str()?, module)))
        .collect::<BTreeMap<_, _>>();

    let mut relationship_options = BTreeMap::<String, u64>::new();
    for link in module_links {
        if let Some(relationship) = link["relationship"].as_str() {
            *relationship_options
                .entry(relationship.to_string())
                .or_default() += value_u64(&link["count"]);
        }
    }

    let mut filtered_links = module_links
        .iter()
        .filter(|link| {
            let relationship = link["relationship"].as_str().unwrap_or_default();
            let relationship_matches =
                selected_relationships.is_empty() || selected_relationships.contains(relationship);
            let module_matches = module_filter
                .map(|filter| {
                    module_matches_filter(link["from_module"].as_str().unwrap_or_default(), filter)
                        || module_matches_filter(
                            link["to_module"].as_str().unwrap_or_default(),
                            filter,
                        )
                })
                .unwrap_or(true);
            relationship_matches && module_matches
        })
        .collect::<Vec<_>>();
    filtered_links.truncate(view.graph_limit());

    let mut node_names = BTreeSet::<String>::new();
    for link in &filtered_links {
        if let Some(module) = link["from_module"].as_str() {
            node_names.insert(module.to_string());
        }
        if let Some(module) = link["to_module"].as_str() {
            node_names.insert(module.to_string());
        }
    }
    if node_names.is_empty() {
        for module in module_drilldowns.iter().take(view.graph_limit()) {
            if let Some(name) = module["module"].as_str() {
                node_names.insert(name.to_string());
            }
        }
    }

    let nodes = node_names
        .iter()
        .map(|module| topology_graph_node(module, module_lookup.get(module.as_str()).copied()))
        .collect::<Vec<_>>();
    let edges = filtered_links
        .iter()
        .map(|link| {
            serde_json::json!({
                "from": link["from_module"].as_str().unwrap_or_default(),
                "to": link["to_module"].as_str().unwrap_or_default(),
                "relationship": link["relationship"].as_str().unwrap_or_default(),
                "count": value_u64(&link["count"]),
                "sample_calls": link["sample_calls"].clone(),
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "filters": {
            "module": module_filter,
            "relationships": &view.relationships,
            "limit": view.graph_limit(),
        },
        "available_filters": {
            "modules": module_drilldowns.iter().filter_map(|module| module["module"].as_str()).collect::<Vec<_>>(),
            "relationships": top_counts_map(&relationship_options, 50),
        },
        "node_count": nodes.len(),
        "edge_count": edges.len(),
        "nodes": nodes,
        "edges": edges,
    })
}

fn module_matches_filter(module: &str, filter: &str) -> bool {
    module == filter || module.starts_with(filter)
}

fn topology_graph_node(module: &str, drilldown: Option<&serde_json::Value>) -> serde_json::Value {
    let capabilities = drilldown
        .map(|item| value_u64(&item["capabilities"]))
        .unwrap_or_default();
    let entrypoints = drilldown
        .map(|item| value_u64(&item["entrypoints"]))
        .unwrap_or_default();
    let external_assets = drilldown
        .map(|item| value_u64(&item["external_assets"]))
        .unwrap_or_default();
    let inbound_calls = drilldown
        .map(|item| value_u64(&item["inbound_calls"]))
        .unwrap_or_default();
    let outbound_calls = drilldown
        .map(|item| value_u64(&item["outbound_calls"]))
        .unwrap_or_default();
    serde_json::json!({
        "id": module,
        "label": module,
        "ownership": drilldown.map(|item| item["ownership"].clone()).unwrap_or_else(|| serde_json::json!({})),
        "service": drilldown.map(|item| item["service"].clone()).unwrap_or(serde_json::Value::Null),
        "owner": drilldown.and_then(|item| item["ownership"]["owner"].as_str()).unwrap_or(module),
        "owner_source": drilldown.and_then(|item| item["ownership"]["source"].as_str()).unwrap_or("module_path"),
        "role": ownership_role(entrypoints, external_assets, inbound_calls, outbound_calls),
        "health": ownership_health(capabilities, entrypoints, external_assets, inbound_calls, outbound_calls),
        "capabilities": capabilities,
        "entrypoints": entrypoints,
        "external_assets": external_assets,
        "inbound_calls": inbound_calls,
        "outbound_calls": outbound_calls,
    })
}

fn ownership_drilldowns(
    asset_map: &serde_json::Value,
    module_drilldowns: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let module_summaries = asset_map["modules"]
        .as_array()
        .map(|modules| {
            modules
                .iter()
                .filter_map(|module| Some((module["name"].as_str()?, module)))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();

    module_drilldowns
        .iter()
        .take(30)
        .map(|drilldown| {
            let module = drilldown["module"].as_str().unwrap_or_default();
            let capabilities = value_u64(&drilldown["capabilities"]);
            let entrypoints = value_u64(&drilldown["entrypoints"]);
            let external_assets = value_u64(&drilldown["external_assets"]);
            let inbound_calls = value_u64(&drilldown["inbound_calls"]);
            let outbound_calls = value_u64(&drilldown["outbound_calls"]);
            let summary = module_summaries.get(module);
            let ownership = summary
                .map(|item| item["ownership"].clone())
                .unwrap_or_else(|| drilldown["ownership"].clone());
            let owner = ownership["owner"].as_str().unwrap_or(module);
            let owner_source = ownership["source"].as_str().unwrap_or("module_path");
            serde_json::json!({
                "owner": owner,
                "owner_source": owner_source,
                "owner_match": ownership["matched"].as_str(),
                "owner_kind": ownership["kind"].as_str(),
                "module": module,
                "ownership": ownership,
                "service": drilldown["service"].clone(),
                "role": ownership_role(entrypoints, external_assets, inbound_calls, outbound_calls),
                "health": ownership_health(capabilities, entrypoints, external_assets, inbound_calls, outbound_calls),
                "risk_flags": ownership_risk_flags(capabilities, entrypoints, external_assets, inbound_calls, outbound_calls),
                "owned_assets": capabilities,
                "owned_entrypoints": entrypoints,
                "owned_external_assets": external_assets,
                "inbound_calls": inbound_calls,
                "outbound_calls": outbound_calls,
                "primary_type": drilldown["primary_type"].as_str().unwrap_or_default(),
                "depends_on_modules": drilldown["outbound_by_module"].clone(),
                "depended_on_by_modules": drilldown["inbound_by_module"].clone(),
                "outbound_relationships": drilldown["outbound_by_relationship"].clone(),
                "inbound_relationships": drilldown["inbound_by_relationship"].clone(),
                "top_packages": summary.map(|item| item["top_packages"].clone()).unwrap_or_else(|| serde_json::json!([])),
                "top_entrypoints": summary
                    .and_then(|item| item["entrypoints"].as_array())
                    .map(|items| items.iter().take(8).cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
            })
        })
        .collect()
}

fn ownership_role(
    entrypoints: u64,
    external_assets: u64,
    inbound_calls: u64,
    outbound_calls: u64,
) -> &'static str {
    if entrypoints > 0 && outbound_calls > 0 {
        "entrypoint_orchestrator"
    } else if entrypoints > 0 {
        "entrypoint_owner"
    } else if inbound_calls > 0 && outbound_calls > 0 {
        "internal_service"
    } else if inbound_calls > 0 {
        "provider"
    } else if external_assets > 0 {
        "external_dependency_owner"
    } else {
        "internal_asset_owner"
    }
}

fn ownership_health(
    capabilities: u64,
    entrypoints: u64,
    external_assets: u64,
    inbound_calls: u64,
    outbound_calls: u64,
) -> &'static str {
    if capabilities == 0 {
        "empty"
    } else if entrypoints > 0 && outbound_calls == 0 {
        "entrypoint_without_downstream_edges"
    } else if inbound_calls == 0 && outbound_calls == 0 && external_assets == 0 {
        "isolated"
    } else {
        "mapped"
    }
}

fn ownership_risk_flags(
    capabilities: u64,
    entrypoints: u64,
    external_assets: u64,
    inbound_calls: u64,
    outbound_calls: u64,
) -> Vec<&'static str> {
    let mut flags = Vec::new();
    if capabilities == 0 {
        flags.push("no_assets");
    }
    if entrypoints > 0 && outbound_calls == 0 {
        flags.push("entrypoints_have_no_downstream_edges");
    }
    if inbound_calls == 0 && outbound_calls == 0 && external_assets == 0 {
        flags.push("isolated_module");
    }
    if external_assets > 0 && inbound_calls == 0 {
        flags.push("external_assets_without_internal_callers");
    }
    flags
}

#[derive(Default)]
struct ModuleTopologyStats {
    capabilities: u64,
    entrypoints: u64,
    external_assets: u64,
    outbound_calls: u64,
    inbound_calls: u64,
    by_type: BTreeMap<String, u64>,
    outbound_by_module: BTreeMap<String, u64>,
    inbound_by_module: BTreeMap<String, u64>,
    outbound_by_relationship: BTreeMap<String, u64>,
    inbound_by_relationship: BTreeMap<String, u64>,
}

#[derive(Default)]
struct ModuleLinkStat {
    count: u64,
    sample_calls: Vec<String>,
}

struct TopologyNode {
    module: String,
    name: String,
    type_name: String,
    framework: Option<String>,
}

fn module_health_rows(asset_map: &serde_json::Value) -> Vec<serde_json::Value> {
    asset_map["modules"]
        .as_array()
        .map(|modules| {
            modules
                .iter()
                .take(20)
                .map(|module| {
                    let assets = module["capability_count"].as_u64().unwrap_or_default();
                    let entrypoints = module["entrypoints"]
                        .as_array()
                        .map(|items| items.len() as u64)
                        .unwrap_or_default();
                    let external = module["external_assets"].as_u64().unwrap_or_default();
                    serde_json::json!({
                        "module": module["name"].as_str().unwrap_or_default(),
                        "owner": module["ownership"]["owner"].as_str().unwrap_or_default(),
                        "owner_source": module["ownership"]["source"].as_str().unwrap_or_default(),
                        "service": module["service"]["name"].as_str().unwrap_or_default(),
                        "service_tier": module["service"]["tier"].as_str().unwrap_or_default(),
                        "service_slo": module["service"]["slo"].as_str().unwrap_or_default(),
                        "assets": assets,
                        "entrypoints": entrypoints,
                        "external_assets": external,
                        "primary_type": primary_type(&module["by_type"]),
                        "health": module_health_label(assets, entrypoints, external),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn primary_type(by_type: &serde_json::Value) -> String {
    count_rows(by_type)
        .first()
        .and_then(|row| row["name"].as_str())
        .unwrap_or("unknown")
        .to_string()
}

fn module_health_label(assets: u64, entrypoints: u64, external_assets: u64) -> &'static str {
    if assets == 0 {
        "empty"
    } else if entrypoints > 0 {
        "entrypoint_visible"
    } else if external_assets > 0 {
        "dependency_visible"
    } else {
        "internal_only"
    }
}

fn count_rows(counts: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut rows = counts
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(name, count)| {
                    count
                        .as_u64()
                        .map(|count| serde_json::json!({"name": name, "count": count}))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    rows.sort_by(|a, b| {
        b["count"]
            .as_u64()
            .cmp(&a["count"].as_u64())
            .then(a["name"].as_str().cmp(&b["name"].as_str()))
    });
    rows
}

fn top_counts_map(counts: &BTreeMap<String, u64>, limit: usize) -> Vec<serde_json::Value> {
    let mut rows = counts
        .iter()
        .map(|(name, count)| serde_json::json!({"name": name, "count": count}))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b["count"]
            .as_u64()
            .cmp(&a["count"].as_u64())
            .then(a["name"].as_str().cmp(&b["name"].as_str()))
    });
    rows.truncate(limit);
    rows
}

fn top_name(counts: &BTreeMap<String, u64>) -> String {
    top_counts_map(counts, 1)
        .first()
        .and_then(|row| row["name"].as_str())
        .unwrap_or("unknown")
        .to_string()
}

fn count_list(value: &serde_json::Value) -> String {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(3)
                .map(|item| {
                    format!(
                        "{} ({})",
                        item["name"].as_str().unwrap_or_default(),
                        item["count"].as_u64().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

fn string_list(value: &serde_json::Value) -> String {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(3)
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

fn count_by<'a>(values: impl Iterator<Item = &'a str>) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for value in values {
        if value.is_empty() {
            continue;
        }
        *counts.entry(value.to_string()).or_insert(0) += 1;
    }
    counts
}

fn top_counts<'a>(values: impl Iterator<Item = &'a str>) -> Vec<serde_json::Value> {
    let mut counts = count_by(values).into_iter().collect::<Vec<_>>();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    counts
        .into_iter()
        .take(10)
        .map(|(name, count)| serde_json::json!({"name": name, "count": count}))
        .collect()
}

fn dashboard_html(summary: &serde_json::Value) -> String {
    let title = "capfind asset dashboard";
    let index = &summary["index"];
    let adoption = &summary["adoption"];
    let funnel = &adoption["funnel"];
    let rejection_taxonomy = &adoption["rejection_taxonomy"];
    let coverage = &summary["coverage"];
    let architecture = &summary["architecture"];
    let topology = &summary["topology"];
    let history = &summary["history"];
    let queries = &summary["queries"];
    let evals = &summary["evals"];
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; margin: 32px; color: #1f2933; background: #f7f8fa; }}
h1 {{ margin: 0 0 8px; font-size: 28px; }}
.sub {{ color: #5c6670; margin-bottom: 24px; }}
.grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 12px; margin-bottom: 24px; }}
.metric {{ background: white; border: 1px solid #d8dee4; border-radius: 8px; padding: 16px; }}
.label {{ color: #64748b; font-size: 12px; text-transform: uppercase; letter-spacing: .04em; }}
.value {{ font-size: 26px; font-weight: 700; margin-top: 6px; }}
section {{ background: white; border: 1px solid #d8dee4; border-radius: 8px; padding: 16px; margin-bottom: 16px; }}
table {{ width: 100%; border-collapse: collapse; font-size: 14px; }}
th, td {{ text-align: left; border-bottom: 1px solid #e5e7eb; padding: 8px 10px; vertical-align: top; }}
th {{ color: #475569; font-size: 12px; text-transform: uppercase; letter-spacing: .04em; }}
.empty {{ color: #64748b; }}
pre {{ white-space: pre-wrap; word-break: break-word; background: #f1f5f9; padding: 12px; border-radius: 6px; }}
.graph-wrap {{ overflow-x: auto; border: 1px solid #e5e7eb; border-radius: 8px; background: #fbfdff; margin-bottom: 12px; }}
.graph-svg {{ min-width: 760px; display: block; }}
.graph-edge {{ stroke: #64748b; stroke-width: 1.6; opacity: .68; fill: none; }}
.graph-edge-label {{ font-size: 11px; fill: #334155; }}
.graph-node {{ fill: #ffffff; stroke: #2563eb; stroke-width: 1.4; }}
.graph-node.dependency {{ stroke: #059669; }}
.graph-node.isolated {{ stroke: #d97706; stroke-dasharray: 4 3; }}
.graph-node-title {{ font-size: 12px; font-weight: 700; fill: #111827; }}
.graph-node-meta {{ font-size: 10px; fill: #64748b; }}
.chips {{ display: flex; flex-wrap: wrap; gap: 8px; margin: 8px 0 12px; }}
.chip {{ border: 1px solid #cbd5e1; border-radius: 999px; background: #f8fafc; color: #334155; padding: 4px 10px; font-size: 12px; }}
.trend-svg {{ width: 100%; min-height: 180px; display: block; margin-bottom: 12px; background: #fbfdff; border: 1px solid #e5e7eb; border-radius: 8px; }}
.trend-line {{ fill: none; stroke-width: 2.4; }}
.trend-label {{ font-size: 11px; fill: #475569; }}
</style>
</head>
<body>
<h1>{title}</h1>
<div class="sub">{repo}</div>
<div class="grid">
  <div class="metric"><div class="label">Assets</div><div class="value">{assets}</div></div>
  <div class="metric"><div class="label">Indexed Files</div><div class="value">{files}</div></div>
  <div class="metric"><div class="label">External Assets</div><div class="value">{external}</div></div>
  <div class="metric"><div class="label">Adoption Rate</div><div class="value">{adoption_rate}</div></div>
  <div class="metric"><div class="label">Final Outcomes</div><div class="value">{final_outcomes}</div></div>
  <div class="metric"><div class="label">Rejected</div><div class="value">{rejected}</div></div>
  <div class="metric"><div class="label">Query Events</div><div class="value">{query_events}</div></div>
  <div class="metric"><div class="label">Zero Result Events</div><div class="value">{zero_result_events}</div></div>
  <div class="metric"><div class="label">Estimated Tokens Saved</div><div class="value">{tokens}</div></div>
</div>
<section><h2>Adoption Funnel</h2>{funnel_table}</section>
<section><h2>Adoption Correlation</h2>{adoption_correlation}</section>
<section><h2>Adoption Quality</h2>{adoption_quality}</section>
<section><h2>Rejected Reasons</h2>{rejected_reasons}</section>
<section><h2>Query Trends</h2>{query_trends}</section>
<section><h2>Eval Quality History</h2>{eval_history}</section>
<section><h2>Service Registry</h2>{service_registry}</section>
<section><h2>Module Health</h2>{module_health}</section>
<section><h2>Service Edges</h2>{edge_counts}</section>
<section><h2>Service Map Diagnostics</h2>{service_diagnostics}</section>
<section><h2>Service Topology</h2>{service_topology}</section>
<section><h2>Metric History</h2>{history_chart}{history_table}</section>
<section><h2>Module Coverage History</h2>{module_history}</section>
<section><h2>Coverage By Type</h2>{by_type}</section>
<section><h2>Top Modules</h2>{top_modules}</section>
<section><h2>Raw Summary</h2><pre>{raw}</pre></section>
</body>
</html>
"#,
        repo = html_escape(summary["repo_root"].as_str().unwrap_or_default()),
        assets = index["capabilities"].as_u64().unwrap_or_default(),
        files = index["indexed_files"].as_u64().unwrap_or_default(),
        external = coverage["external_assets"].as_u64().unwrap_or_default(),
        adoption_rate = adoption["adoption_rate"].as_f64().unwrap_or_default(),
        final_outcomes = adoption["final_events"].as_u64().unwrap_or_default(),
        rejected = adoption["rejected"].as_u64().unwrap_or_default(),
        query_events = value_u64(&queries["events"]),
        zero_result_events = value_u64(&queries["zero_result_events"]),
        tokens = summary["impact"]["estimated_tokens_saved"]
            .as_u64()
            .unwrap_or_default(),
        funnel_table = count_table_from_object(&funnel["events"], "Stage", "Events"),
        rejected_reasons = count_table_from_object(
            &rejection_taxonomy["by_reason"],
            "Rejected Reason",
            "Events"
        ),
        adoption_correlation = adoption_correlation_html(adoption),
        adoption_quality = adoption_quality_html(&adoption["quality"]),
        query_trends = query_trends_html(queries),
        eval_history = eval_history_table(&evals["recent"]),
        service_registry = service_registry_table(&summary["asset_map"]["services"]["items"]),
        module_health = module_health_table(&architecture["module_health"]),
        edge_counts = count_table_from_rows(&architecture["edge_counts"], "Relationship", "Edges"),
        service_diagnostics = service_diagnostics_table(&architecture["diagnostics"]["items"]),
        service_topology = service_topology_html(topology),
        history_chart = dashboard_history_chart(&history["recent"]),
        history_table = dashboard_history_table(&history["recent"]),
        module_history = module_history_table(&history["recent"]),
        by_type = count_table_from_object(&coverage["by_type"], "Type", "Assets"),
        top_modules = count_table_from_rows(&coverage["top_modules"], "Module", "Assets"),
        raw = html_escape(&serde_json::to_string_pretty(summary).unwrap_or_default()),
    )
}

fn adoption_correlation_html(adoption: &serde_json::Value) -> String {
    format!(
        "<h3>By Task</h3>{}<h3>By Candidate</h3>{}<h3>By Session</h3>{}",
        adoption_task_table(&adoption["funnel_by_task"]),
        adoption_candidate_table(&adoption["funnel_by_candidate"]),
        adoption_session_table(&adoption["sessions"]),
    )
}

fn adoption_quality_html(quality: &serde_json::Value) -> String {
    format!(
        "<h3>Summary</h3>{}<h3>Candidate Quality</h3>{}<h3>Top Reusable Candidates</h3>{}<h3>Noise Risk Candidates</h3>{}<h3>Low Confidence Pockets</h3>{}<h3>Task Quality</h3>{}<h3>Session Quality</h3>{}",
        adoption_quality_summary_table(&quality["summary"]),
        adoption_quality_candidate_table(&quality["candidate_quality"]),
        adoption_quality_candidate_table(&quality["top_reusable_candidates"]),
        adoption_quality_candidate_table(&quality["noise_risk_candidates"]),
        adoption_quality_candidate_table(&quality["low_confidence_pockets"]),
        adoption_quality_scope_table(&quality["task_quality"], "task", "Task"),
        adoption_quality_scope_table(&quality["session_quality"], "session_id", "Session"),
    )
}

fn query_trends_html(queries: &serde_json::Value) -> String {
    format!(
        "<h3>Summary</h3>{}<h3>Top Queries</h3>{}<h3>Zero Result Queries</h3>{}<h3>Daily Timeline</h3>{}<h3>Recent Query Events</h3>{}<h3>By Tool</h3>{}<h3>By Source</h3>{}",
        query_summary_table(queries),
        query_rollup_table(&queries["top_queries"]),
        query_rollup_table(&queries["zero_result_queries"]),
        query_timeline_table(&queries["timeline"]),
        recent_query_table(&queries["recent"]),
        count_table_from_object(&queries["by_tool"], "Tool", "Events"),
        count_table_from_object(&queries["by_source"], "Source", "Events"),
    )
}

fn query_summary_table(queries: &serde_json::Value) -> String {
    html_table(
        &["Metric", "Value"],
        vec![
            vec![
                "Events".to_string(),
                value_u64(&queries["events"]).to_string(),
            ],
            vec![
                "Unique Queries".to_string(),
                value_u64(&queries["unique_queries"]).to_string(),
            ],
            vec![
                "Zero Result Events".to_string(),
                value_u64(&queries["zero_result_events"]).to_string(),
            ],
            vec![
                "Zero Result Rate".to_string(),
                format!("{:.3}", value_f64(&queries["zero_result_rate"])),
            ],
        ],
    )
}

fn query_rollup_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(20)
                .map(|item| {
                    vec![
                        item["query"].as_str().unwrap_or_default().to_string(),
                        value_u64(&item["events"]).to_string(),
                        value_u64(&item["zero_result_events"]).to_string(),
                        format!("{:.3}", value_f64(&item["zero_result_rate"])),
                        format!("{:.3}", value_f64(&item["average_results"])),
                        value_u64(&item["latest_event_unix"]).to_string(),
                        count_list(&count_rows_value(&item["tools"])),
                        count_list(&count_rows_value(&item["sources"])),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Query",
            "Events",
            "Zero",
            "Zero Rate",
            "Avg Results",
            "Latest",
            "Tools",
            "Sources",
        ],
        rows,
    )
}

fn query_timeline_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .rev()
                .take(30)
                .map(|item| {
                    vec![
                        value_u64(&item["unix_day"]).to_string(),
                        value_u64(&item["events"]).to_string(),
                        value_u64(&item["unique_queries"]).to_string(),
                        value_u64(&item["zero_result_events"]).to_string(),
                        format!("{:.3}", value_f64(&item["average_results"])),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Unix Day",
            "Events",
            "Unique Queries",
            "Zero Results",
            "Avg Results",
        ],
        rows,
    )
}

fn recent_query_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .rev()
                .take(20)
                .map(|item| {
                    vec![
                        value_u64(&item["created_unix"]).to_string(),
                        item["tool"].as_str().unwrap_or_default().to_string(),
                        item["source"].as_str().unwrap_or_default().to_string(),
                        item["query"].as_str().unwrap_or_default().to_string(),
                        value_u64(&item["result_count"]).to_string(),
                        item["diagnosis"].as_str().unwrap_or_default().to_string(),
                        compact_list(&item["top_candidate_ids"]),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Unix Time",
            "Tool",
            "Source",
            "Query",
            "Results",
            "Diagnosis",
            "Top Candidates",
        ],
        rows,
    )
}

fn count_rows_value(value: &serde_json::Value) -> serde_json::Value {
    serde_json::Value::Array(count_rows(value))
}

fn adoption_quality_summary_table(summary: &serde_json::Value) -> String {
    html_table(
        &["Metric", "Value"],
        vec![
            vec![
                "Candidates".to_string(),
                value_u64(&summary["candidate_count"]).to_string(),
            ],
            vec![
                "Tasks".to_string(),
                value_u64(&summary["task_count"]).to_string(),
            ],
            vec![
                "Sessions".to_string(),
                value_u64(&summary["session_count"]).to_string(),
            ],
            vec![
                "Average Adoption Probability".to_string(),
                format!("{:.3}", value_f64(&summary["average_adoption_probability"])),
            ],
            vec![
                "High Quality Candidates".to_string(),
                value_u64(&summary["high_quality_candidates"]).to_string(),
            ],
            vec![
                "Noise Risk Candidates".to_string(),
                value_u64(&summary["noise_risk_candidates"]).to_string(),
            ],
            vec![
                "Low Confidence Candidates".to_string(),
                value_u64(&summary["low_confidence_candidates"]).to_string(),
            ],
        ],
    )
}

fn adoption_quality_candidate_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(20)
                .map(|item| {
                    vec![
                        value_u64(&item["candidate_id"]).to_string(),
                        item["candidate_type"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        format!("{:.3}", value_f64(&item["adoption_probability"])),
                        format!("{:.3}", value_f64(&item["confidence"])),
                        item["quality_label"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        item["recommended_action"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        value_u64(&item["adopted"]).to_string(),
                        value_u64(&item["rejected"]).to_string(),
                        value_u64(&item["opportunities"]).to_string(),
                        compact_list(&item["tasks"]),
                        compact_list(&item["session_ids"]),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Candidate",
            "Type",
            "Adoption Probability",
            "Confidence",
            "Label",
            "Action",
            "Adopted",
            "Rejected",
            "Opportunities",
            "Tasks",
            "Sessions",
        ],
        rows,
    )
}

fn adoption_quality_scope_table(value: &serde_json::Value, key: &str, label: &str) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(20)
                .map(|item| {
                    vec![
                        item[key].as_str().unwrap_or_default().to_string(),
                        format!("{:.3}", value_f64(&item["adoption_probability"])),
                        format!("{:.3}", value_f64(&item["confidence"])),
                        item["quality_label"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        item["recommended_action"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        value_u64(&item["adopted"]).to_string(),
                        value_u64(&item["rejected"]).to_string(),
                        value_u64(&item["opportunities"]).to_string(),
                        compact_list(&item["candidate_ids"]),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            label,
            "Adoption Probability",
            "Confidence",
            "Label",
            "Action",
            "Adopted",
            "Rejected",
            "Opportunities",
            "Candidates",
        ],
        rows,
    )
}

fn adoption_task_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(20)
                .map(|item| {
                    vec![
                        item["task"].as_str().unwrap_or_default().to_string(),
                        value_u64(&item["events"]).to_string(),
                        compact_list(&item["stage_path"]),
                        value_u64(&item["final_events"]).to_string(),
                        format!("{:.3}", value_f64(&item["adoption_rate"])),
                        item["outcome"].as_str().unwrap_or_default().to_string(),
                        compact_list(&item["candidate_ids"]),
                        compact_list(&item["session_ids"]),
                        item["latest_stage"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Task",
            "Events",
            "Path",
            "Final",
            "Rate",
            "Outcome",
            "Candidates",
            "Sessions",
            "Latest",
        ],
        rows,
    )
}

fn adoption_candidate_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(20)
                .map(|item| {
                    vec![
                        value_u64(&item["candidate_id"]).to_string(),
                        item["candidate_type"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        item["candidate_file"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        value_u64(&item["events"]).to_string(),
                        compact_list(&item["stage_path"]),
                        value_u64(&item["final_events"]).to_string(),
                        format!("{:.3}", value_f64(&item["adoption_rate"])),
                        item["outcome"].as_str().unwrap_or_default().to_string(),
                        compact_list(&item["tasks"]),
                        compact_list(&item["session_ids"]),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Candidate",
            "Type",
            "File",
            "Events",
            "Path",
            "Final",
            "Rate",
            "Outcome",
            "Tasks",
            "Sessions",
        ],
        rows,
    )
}

fn adoption_session_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(20)
                .map(|item| {
                    vec![
                        item["session_id"].as_str().unwrap_or_default().to_string(),
                        value_u64(&item["events"]).to_string(),
                        compact_list(&item["stage_path"]),
                        value_u64(&item["final_events"]).to_string(),
                        format!("{:.3}", value_f64(&item["adoption_rate"])),
                        item["outcome"].as_str().unwrap_or_default().to_string(),
                        compact_list(&item["tasks"]),
                        compact_list(&item["candidate_ids"]),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Session",
            "Events",
            "Path",
            "Final",
            "Rate",
            "Outcome",
            "Tasks",
            "Candidates",
        ],
        rows,
    )
}

fn dashboard_history_chart(value: &serde_json::Value) -> String {
    let Some(items) = value.as_array().filter(|items| !items.is_empty()) else {
        return "<p class=\"empty\">No data</p>".to_string();
    };
    let width = 920.0;
    let height = 180.0;
    let left = 44.0;
    let right = 24.0;
    let top = 18.0;
    let bottom = 30.0;
    let plot_width = width - left - right;
    let plot_height = height - top - bottom;
    let series = [
        (
            "Assets",
            "#2563eb",
            history_series(items, |item| value_f64(&item["index"]["capabilities"])),
        ),
        (
            "Edges",
            "#059669",
            history_series(items, |item| value_f64(&item["architecture"]["edge_count"])),
        ),
        (
            "Queries",
            "#d97706",
            history_series(items, |item| value_f64(&item["queries"]["events"])),
        ),
        (
            "Adoption",
            "#7c3aed",
            history_series(items, |item| {
                value_f64(&item["adoption"]["adoption_rate"]) * 100.0
            }),
        ),
    ];
    let max_value = series
        .iter()
        .flat_map(|(_, _, values)| values.iter().copied())
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let mut svg = format!(
        r#"<svg class="trend-svg" viewBox="0 0 {width} {height}" role="img" aria-label="metric history chart">"#
    );
    svg.push_str(&format!(
        r##"<line x1="{left}" y1="{top}" x2="{left}" y2="{}" stroke="#cbd5e1"/><line x1="{left}" y1="{}" x2="{}" y2="{}" stroke="#cbd5e1"/>"##,
        height - bottom,
        height - bottom,
        width - right,
        height - bottom
    ));
    for (idx, (label, color, values)) in series.iter().enumerate() {
        if values.is_empty() {
            continue;
        }
        let points = values
            .iter()
            .enumerate()
            .map(|(point_idx, value)| {
                let x = if values.len() == 1 {
                    left
                } else {
                    left + (point_idx as f64 * plot_width / (values.len() - 1) as f64)
                };
                let y = top + plot_height - (value / max_value * plot_height);
                format!("{x:.1},{y:.1}")
            })
            .collect::<Vec<_>>()
            .join(" ");
        svg.push_str(&format!(
            r##"<polyline class="trend-line" points="{points}" stroke="{color}"/>"##
        ));
        svg.push_str(&format!(
            r##"<text class="trend-label" x="{}" y="{}" fill="{color}">{}</text>"##,
            left + idx as f64 * 130.0,
            height - 8.0,
            html_escape(label)
        ));
    }
    svg.push_str("</svg>");
    svg
}

fn history_series(
    items: &[serde_json::Value],
    read: impl Fn(&serde_json::Value) -> f64,
) -> Vec<f64> {
    items.iter().map(read).collect()
}

fn dashboard_history_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .rev()
                .take(10)
                .map(|item| {
                    vec![
                        value_u64(&item["created_unix"]).to_string(),
                        value_u64(&item["index"]["capabilities"]).to_string(),
                        value_u64(&item["index"]["indexed_files"]).to_string(),
                        format!("{:.3}", value_f64(&item["adoption"]["adoption_rate"])),
                        value_u64(&item["adoption"]["final_events"]).to_string(),
                        value_u64(&item["queries"]["events"]).to_string(),
                        value_u64(&item["queries"]["zero_result_events"]).to_string(),
                        value_u64(&item["architecture"]["edge_count"]).to_string(),
                        value_u64(&item["impact"]["estimated_tokens_saved"]).to_string(),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Unix Time",
            "Assets",
            "Files",
            "Adoption Rate",
            "Final Events",
            "Query Events",
            "Zero Results",
            "Edges",
            "Tokens Saved",
        ],
        rows,
    )
}

fn module_history_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .rev()
                .flat_map(|snapshot| {
                    let created_unix = value_u64(&snapshot["created_unix"]).to_string();
                    snapshot["modules"]
                        .as_array()
                        .map(|modules| {
                            modules
                                .iter()
                                .take(8)
                                .map(|module| {
                                    vec![
                                        created_unix.clone(),
                                        module["module"].as_str().unwrap_or_default().to_string(),
                                        module["service"].as_str().unwrap_or_default().to_string(),
                                        module["owner"].as_str().unwrap_or_default().to_string(),
                                        value_u64(&module["assets"]).to_string(),
                                        value_u64(&module["entrypoints"]).to_string(),
                                        value_u64(&module["external_assets"]).to_string(),
                                        module["health"].as_str().unwrap_or_default().to_string(),
                                    ]
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
                .take(40)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Unix Time",
            "Module",
            "Service",
            "Owner",
            "Assets",
            "Entrypoints",
            "External",
            "Health",
        ],
        rows,
    )
}

fn eval_history_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .rev()
                .take(10)
                .map(|item| {
                    vec![
                        value_u64(&item["created_unix"]).to_string(),
                        html_escape(item["suite"].as_str().unwrap_or_default()),
                        format!("{:.3}", value_f64(&item["metrics"]["query"]["recall_at_k"])),
                        format!("{:.3}", value_f64(&item["metrics"]["query"]["mrr"])),
                        format!(
                            "{:.3}",
                            value_f64(&item["metrics"]["file"]["diagnosis_pass_rate"])
                        ),
                        format!(
                            "{:.3}",
                            value_f64(&item["metrics"]["edge"]["edge_pass_rate"])
                        ),
                        format!(
                            "{:.3}",
                            value_f64(&item["metrics"]["diagnostic"]["diagnostic_pass_rate"])
                        ),
                        format!(
                            "{:.3}",
                            value_f64(&item["metrics"]["ownership"]["ownership_pass_rate"])
                        ),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Unix Time",
            "Suite",
            "Recall",
            "MRR",
            "File Pass",
            "Edge Pass",
            "Diagnostic Pass",
            "Ownership Pass",
        ],
        rows,
    )
}

fn compact_list(value: &serde_json::Value) -> String {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(3)
                .map(compact_value)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

fn compact_value(value: &serde_json::Value) -> String {
    if let Some(value) = value.as_str() {
        value.to_string()
    } else if let Some(value) = value.as_u64() {
        value.to_string()
    } else if let Some(value) = value.as_i64() {
        value.to_string()
    } else if let Some(value) = value.as_bool() {
        value.to_string()
    } else {
        String::new()
    }
}

fn service_topology_html(topology: &serde_json::Value) -> String {
    format!(
        "<h3>Service Graph</h3>{}<h3>Graph Filters</h3>{}<h3>Ownership Drilldowns</h3>{}<h3>Module Links</h3>{}<h3>Module Drilldowns</h3>{}<h3>Sample Edges</h3>{}",
        topology_graph_svg(&topology["graph_view"]),
        topology_graph_filters_html(&topology["graph_view"]),
        ownership_drilldowns_table(&topology["ownership_drilldowns"]),
        module_links_table(&topology["module_links"]),
        module_drilldowns_table(&topology["module_drilldowns"]),
        sample_edges_table(&topology["sample_edges"])
    )
}

fn topology_graph_filters_html(graph_view: &serde_json::Value) -> String {
    let filters = &graph_view["filters"];
    let module = filters["module"].as_str().unwrap_or("all modules");
    let relationships = filters["relationships"]
        .as_array()
        .filter(|items| !items.is_empty())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "all relationships".to_string());
    format!(
        "<div class=\"chips\"><span class=\"chip\">module: {}</span><span class=\"chip\">relationships: {}</span><span class=\"chip\">graph limit: {}</span></div>{}",
        html_escape(module),
        html_escape(&relationships),
        value_u64(&filters["limit"]),
        count_table_from_rows(
            &graph_view["available_filters"]["relationships"],
            "Available Relationship",
            "Edges"
        )
    )
}

fn topology_graph_svg(graph_view: &serde_json::Value) -> String {
    let nodes = graph_view["nodes"]
        .as_array()
        .map(|items| items.as_slice())
        .unwrap_or(&[]);
    if nodes.is_empty() {
        return "<p class=\"empty\">No graph nodes for current filters</p>".to_string();
    }

    let columns = 4usize;
    let rows = nodes.len().div_ceil(columns);
    let width = 960i32;
    let height = (rows as i32 * 118 + 72).max(240);
    let mut positions = BTreeMap::<String, (i32, i32)>::new();
    for (idx, node) in nodes.iter().enumerate() {
        let col = idx % columns;
        let row = idx / columns;
        let x = 120 + col as i32 * 230;
        let y = 72 + row as i32 * 118;
        if let Some(id) = node["id"].as_str() {
            positions.insert(id.to_string(), (x, y));
        }
    }

    let mut svg = format!(
        "<div class=\"graph-wrap\"><svg class=\"graph-svg\" viewBox=\"0 0 {width} {height}\" role=\"img\" aria-label=\"Service topology graph\"><defs><marker id=\"arrow\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"6\" markerHeight=\"6\" orient=\"auto-start-reverse\"><path d=\"M0,0 L8,4 L0,8 Z\" fill=\"#64748b\"/></marker></defs>"
    );
    if let Some(edges) = graph_view["edges"].as_array() {
        for edge in edges {
            let from = edge["from"].as_str().unwrap_or_default();
            let to = edge["to"].as_str().unwrap_or_default();
            let Some((x1, y1)) = positions.get(from).copied() else {
                continue;
            };
            let Some((x2, y2)) = positions.get(to).copied() else {
                continue;
            };
            let label = format!(
                "{} ({})",
                edge["relationship"].as_str().unwrap_or_default(),
                value_u64(&edge["count"])
            );
            if from == to {
                let loop_x = x1 + 62;
                let loop_y = y1 - 42;
                svg.push_str(&format!(
                    "<path class=\"graph-edge\" marker-end=\"url(#arrow)\" d=\"M{x1} {y1} C {loop_x} {loop_y}, {loop_x} {loop_y}, {} {}\"/><text class=\"graph-edge-label\" x=\"{}\" y=\"{}\">{}</text>",
                    x1 + 1,
                    y1 - 1,
                    x1 + 42,
                    y1 - 36,
                    html_escape(&label)
                ));
            } else {
                let mid_x = (x1 + x2) / 2;
                let mid_y = (y1 + y2) / 2 - 6;
                svg.push_str(&format!(
                    "<line class=\"graph-edge\" marker-end=\"url(#arrow)\" x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\"/><text class=\"graph-edge-label\" x=\"{mid_x}\" y=\"{mid_y}\">{}</text>",
                    html_escape(&label)
                ));
            }
        }
    }

    for node in nodes {
        let id = node["id"].as_str().unwrap_or_default();
        let Some((x, y)) = positions.get(id).copied() else {
            continue;
        };
        let label = shorten_label(node["label"].as_str().unwrap_or(id), 24);
        let role = node["role"].as_str().unwrap_or_default();
        let health = node["health"].as_str().unwrap_or_default();
        let class = if role.contains("dependency") {
            "graph-node dependency"
        } else if health == "isolated" || health == "entrypoint_without_downstream_edges" {
            "graph-node isolated"
        } else {
            "graph-node"
        };
        let meta = format!(
            "{} assets / {} in / {} out",
            value_u64(&node["capabilities"]),
            value_u64(&node["inbound_calls"]),
            value_u64(&node["outbound_calls"])
        );
        svg.push_str(&format!(
            "<rect class=\"{class}\" x=\"{}\" y=\"{}\" width=\"170\" height=\"56\" rx=\"8\"/><text class=\"graph-node-title\" x=\"{}\" y=\"{}\">{}</text><text class=\"graph-node-meta\" x=\"{}\" y=\"{}\">{}</text><title>{}</title>",
            x - 85,
            y - 28,
            x - 74,
            y - 6,
            html_escape(&label),
            x - 74,
            y + 13,
            html_escape(&meta),
            html_escape(&format!("{id} · {role} · {health}"))
        ));
    }
    svg.push_str("</svg></div>");
    svg
}

fn shorten_label(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let shortened = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{shortened}...")
    } else {
        shortened
    }
}

fn ownership_drilldowns_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(20)
                .map(|item| {
                    vec![
                        item["owner"].as_str().unwrap_or_default().to_string(),
                        item["owner_source"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        item["role"].as_str().unwrap_or_default().to_string(),
                        item["health"].as_str().unwrap_or_default().to_string(),
                        value_u64(&item["owned_assets"]).to_string(),
                        value_u64(&item["owned_entrypoints"]).to_string(),
                        value_u64(&item["owned_external_assets"]).to_string(),
                        count_list(&item["depends_on_modules"]),
                        count_list(&item["depended_on_by_modules"]),
                        compact_list(&item["risk_flags"]),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Owner",
            "Owner Source",
            "Role",
            "Health",
            "Assets",
            "Entrypoints",
            "External",
            "Depends On",
            "Depended On By",
            "Flags",
        ],
        rows,
    )
}

fn service_diagnostics_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(20)
                .map(|item| {
                    vec![
                        item["kind"].as_str().unwrap_or_default().to_string(),
                        item["reason"].as_str().unwrap_or_default().to_string(),
                        item["file"].as_str().unwrap_or_default().to_string(),
                        value_u64(&item["line"]).to_string(),
                        item["symbol"].as_str().unwrap_or_default().to_string(),
                        value_u64(&item["candidate_count"]).to_string(),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &["Kind", "Reason", "File", "Line", "Symbol", "Candidates"],
        rows,
    )
}

fn module_links_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(20)
                .map(|item| {
                    vec![
                        item["from_module"].as_str().unwrap_or_default().to_string(),
                        item["to_module"].as_str().unwrap_or_default().to_string(),
                        item["relationship"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        value_u64(&item["count"]).to_string(),
                        string_list(&item["sample_calls"]),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "From Module",
            "To Module",
            "Relationship",
            "Edges",
            "Sample Calls",
        ],
        rows,
    )
}

fn module_drilldowns_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(20)
                .map(|item| {
                    vec![
                        item["module"].as_str().unwrap_or_default().to_string(),
                        item["ownership"]["owner"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        item["service"]["name"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        item["service"]["slo"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        value_u64(&item["capabilities"]).to_string(),
                        value_u64(&item["entrypoints"]).to_string(),
                        value_u64(&item["external_assets"]).to_string(),
                        value_u64(&item["outbound_calls"]).to_string(),
                        value_u64(&item["inbound_calls"]).to_string(),
                        item["primary_type"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        count_list(&item["outbound_by_module"]),
                        count_list(&item["inbound_by_module"]),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Module",
            "Owner",
            "Service",
            "SLO",
            "Assets",
            "Entrypoints",
            "External",
            "Outbound",
            "Inbound",
            "Primary Type",
            "Top Outbound",
            "Top Inbound",
        ],
        rows,
    )
}

fn service_registry_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(30)
                .map(|item| {
                    vec![
                        item["id"].as_str().unwrap_or_default().to_string(),
                        item["name"].as_str().unwrap_or_default().to_string(),
                        item["owner"].as_str().unwrap_or_default().to_string(),
                        item["tier"].as_str().unwrap_or_default().to_string(),
                        item["slo"].as_str().unwrap_or_default().to_string(),
                        item["module"].as_str().unwrap_or_default().to_string(),
                        item["package"].as_str().unwrap_or_default().to_string(),
                        value_u64(&item["capability_count"]).to_string(),
                        value_u64(&item["entrypoint_count"]).to_string(),
                        item["runbook"].as_str().unwrap_or_default().to_string(),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Service",
            "Name",
            "Owner",
            "Tier",
            "SLO",
            "Module",
            "Package",
            "Assets",
            "Entrypoints",
            "Runbook",
        ],
        rows,
    )
}

fn sample_edges_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(20)
                .map(|item| {
                    vec![
                        item["from_module"].as_str().unwrap_or_default().to_string(),
                        item["from"].as_str().unwrap_or_default().to_string(),
                        item["relationship"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        item["to_module"].as_str().unwrap_or_default().to_string(),
                        item["to"].as_str().unwrap_or_default().to_string(),
                        item["call"].as_str().unwrap_or_default().to_string(),
                        item["confidence"].as_str().unwrap_or_default().to_string(),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "From Module",
            "From",
            "Relationship",
            "To Module",
            "To",
            "Call",
            "Confidence",
        ],
        rows,
    )
}

fn module_health_table(value: &serde_json::Value) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    vec![
                        item["module"].as_str().unwrap_or_default().to_string(),
                        item["owner"].as_str().unwrap_or_default().to_string(),
                        item["owner_source"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        item["service"].as_str().unwrap_or_default().to_string(),
                        item["service_tier"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        item["service_slo"].as_str().unwrap_or_default().to_string(),
                        item["assets"].as_u64().unwrap_or_default().to_string(),
                        item["entrypoints"].as_u64().unwrap_or_default().to_string(),
                        item["external_assets"]
                            .as_u64()
                            .unwrap_or_default()
                            .to_string(),
                        item["primary_type"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        item["health"].as_str().unwrap_or_default().to_string(),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(
        &[
            "Module",
            "Owner",
            "Owner Source",
            "Service",
            "Tier",
            "SLO",
            "Assets",
            "Entrypoints",
            "External",
            "Primary Type",
            "Health",
        ],
        rows,
    )
}

fn count_table_from_object(
    value: &serde_json::Value,
    name_label: &str,
    count_label: &str,
) -> String {
    count_table_from_rows(
        &serde_json::Value::Array(count_rows(value)),
        name_label,
        count_label,
    )
}

fn count_table_from_rows(value: &serde_json::Value, name_label: &str, count_label: &str) -> String {
    let rows = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    vec![
                        item["name"].as_str().unwrap_or_default().to_string(),
                        item["count"].as_u64().unwrap_or_default().to_string(),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    html_table(&[name_label, count_label], rows)
}

fn html_table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    if rows.is_empty() {
        return "<p class=\"empty\">No data</p>".to_string();
    }
    let header_html = headers
        .iter()
        .map(|header| format!("<th>{}</th>", html_escape(header)))
        .collect::<Vec<_>>()
        .join("");
    let rows_html = rows
        .into_iter()
        .map(|row| {
            let cells = row
                .into_iter()
                .map(|cell| format!("<td>{}</td>", html_escape(&cell)))
                .collect::<Vec<_>>()
                .join("");
            format!("<tr>{cells}</tr>")
        })
        .collect::<Vec<_>>()
        .join("");
    format!("<table><thead><tr>{header_html}</tr></thead><tbody>{rows_html}</tbody></table>")
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `capfind diagnose` — show what parser extracts from a file.
pub fn diagnose(file: &Path) -> Result<()> {
    if !file.exists() {
        bail!("File not found: {}", file.display());
    }
    let content = std::fs::read_to_string(file).context("Cannot read file")?;
    let caps = capfind_parsers::parse_file(file, &content);

    if caps.is_empty() {
        println!("No capabilities detected in {}", file.display());
        println!("(File may lack recognized annotations like @Controller, @Service, etc.)");
        return Ok(());
    }

    println!("Found {} capabilities in {}:\n", caps.len(), file.display());
    for (i, cap) in caps.iter().enumerate() {
        println!(
            "  [{}] {} · {} · line {}",
            i,
            cap.kind.as_str(),
            cap.qualified(),
            cap.line
        );
        if let Some(ref http) = cap.http {
            println!("       {} {}", http.method, http.path);
        }
        println!("       sig: {}", cap.signature);
        if let Some(ref doc) = cap.doc {
            println!("       doc: {}", doc);
        }
        println!();
    }
    Ok(())
}

const DEFAULT_CONFIG: &str = r#"# capfind configuration
# See https://github.com/a1418507570/capfind for docs.

version = 1

[index]
# roots = ["."]
# include = ["**/*.java", "**/*.go", "**/*.proto"]

[search]
# BM25 tuning. k1 must be > 0; b must be between 0 and 1.
# k1 = 1.2
# b = 0.4

[ownership.modules]
# "services/order-api" = "Order Platform"

[ownership.packages]
# "com.example.billing" = "Billing Team"

[ownership.external]
# "com.fasterxml.jackson.core" = "Runtime Platform"

[synonyms]
# Project-specific business vocabulary. Triggers may be Chinese or code terms;
# values are tokenized like source identifiers, so legalPersonId matches
# legalpersonid / legal / person / id where those tokens exist in the index.
# mdm = ["master-data"]
# "商户资料" = ["merchantAccount", "merchantId"]
"#;

const DEFAULT_CAPFINDIGNORE: &str = r#"# capfind ignore file — layered on top of .gitignore.
# Use gitignore syntax. Paths are repo-relative.

/target
/build
/node_modules
/vendor
/.gradle
/generated
**/*Test.java
**/*Tests.java
**/src/test/**
**/*_test.go
**/*.pb.go
"#;
