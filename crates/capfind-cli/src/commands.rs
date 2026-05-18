//! CLI subcommand implementations.

use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use capfind_core::{load_index, write_index, Capability, Kind, Lang};
use capfind_search::search;

use crate::config;
use crate::fmt as output;
use crate::indexer;
use crate::{AgentArgs, FindArgs, McpArgs};

/// `capfind init` — create .capfind/ directory with config.
pub fn init(repo_root: &Path) -> Result<()> {
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

    println!("Initialized .capfind/ at {}", repo_root.display());
    Ok(())
}

/// `capfind index` — scan + build index.
pub fn index(repo_root: &Path, rehash: bool) -> Result<()> {
    build_index_file(repo_root, rehash, true)
}

fn build_index_file(repo_root: &Path, rehash: bool, verbose: bool) -> Result<()> {
    let start = Instant::now();

    if verbose {
        println!("Scanning {}...", repo_root.display());
    }
    let build_start = Instant::now();
    let result = indexer::index_repo(repo_root, rehash)?;
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
    if !index_path.exists() {
        if args.auto_index {
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

    let has_candidates = !filtered.is_empty();
    output::print_agent_json(
        &query,
        &filtered,
        &body.capabilities,
        start.elapsed(),
        args.fail_on_candidates,
    )?;
    if args.fail_on_candidates && has_candidates {
        use std::io::Write;
        std::io::stdout().flush()?;
        std::process::exit(2);
    }
    Ok(())
}

/// `capfind mcp` — MCP-compatible tool catalog and local tool-call shim.
pub fn mcp(repo_root: &Path, args: &McpArgs) -> Result<()> {
    use serde_json::{json, Value};

    if args.stdio {
        return mcp_stdio(repo_root);
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
    let output = mcp_call_tool(repo_root, tool_name, &input)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "tool": tool_name,
            "result": output,
        }))?
    );
    Ok(())
}

fn mcp_stdio(repo_root: &Path) -> Result<()> {
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
            "tools/call" => Some(handle_mcp_tools_call(repo_root, id, &request)),
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
    match mcp_call_tool(repo_root, tool_name, &arguments) {
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

fn mcp_call_tool(
    repo_root: &Path,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<serde_json::Value> {
    match tool_name {
        "capfind_search" => mcp_search(repo_root, input),
        "capfind_show" => mcp_show(repo_root, input),
        "capfind_agent_preflight" => mcp_agent_preflight(repo_root, input),
        other => bail!("Unknown MCP tool: {other}"),
    }
}

fn mcp_tools_json() -> serde_json::Value {
    use serde_json::json;
    json!({
        "server": "capfind",
        "schema_version": "capfind.mcp.v1",
        "tools": [
            {
                "name": "capfind_search",
                "description": "Search indexed capabilities and referenced external APIs.",
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
                    "properties": {"id": {"type": "integer"}},
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
            }
        ]
    })
}

fn mcp_search(repo_root: &Path, input: &serde_json::Value) -> Result<serde_json::Value> {
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
    let (_header, body) = load_index(&index_path).context("Failed to load index")?;
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
    Ok(json!({
        "query": query,
        "took_ms": start.elapsed().as_millis(),
        "results": results,
    }))
}

fn mcp_show(repo_root: &Path, input: &serde_json::Value) -> Result<serde_json::Value> {
    let id = input
        .get("id")
        .and_then(|value| value.as_u64())
        .context("capfind_show requires integer field `id`")? as usize;
    let index_path = repo_root.join(".capfind/index.cfi");
    let (_header, body) = load_index(&index_path).context("Failed to load index")?;
    let cap = body
        .capabilities
        .get(id)
        .context(format!("No capability with id={id}"))?;
    Ok(capability_json(cap, 0.0))
}

fn mcp_agent_preflight(repo_root: &Path, input: &serde_json::Value) -> Result<serde_json::Value> {
    use serde_json::json;
    let task = input
        .get("task")
        .and_then(|value| value.as_str())
        .context("capfind_agent_preflight requires string field `task`")?;
    let limit = input
        .get("limit")
        .and_then(|value| value.as_u64())
        .unwrap_or(5) as usize;
    let search_result = mcp_search(repo_root, &json!({"query": task, "limit": limit}))?;
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
pub fn show(repo_root: &Path, id: u32, _no_color: bool) -> Result<()> {
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

[synonyms]
# mdm = ["master-data"]
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
