//! CLI subcommand implementations.

use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use capfind_core::{load_index, write_index, Kind, Lang};
use capfind_search::search;

use crate::config;
use crate::fmt as output;
use crate::indexer;
use crate::{AgentArgs, FindArgs};

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
pub fn index(repo_root: &Path, _rehash: bool) -> Result<()> {
    let start = Instant::now();

    println!("Scanning {}...", repo_root.display());
    let caps = indexer::scan_and_parse(repo_root)?;
    let scan_elapsed = start.elapsed();
    println!(
        "  Found {} capabilities in {:.2}s",
        caps.len(),
        scan_elapsed.as_secs_f64()
    );

    let build_start = Instant::now();
    let body = indexer::build_index(caps);
    let build_elapsed = build_start.elapsed();
    println!(
        "  Built index ({} vocab terms) in {:.2}s",
        body.vocab.len(),
        build_elapsed.as_secs_f64()
    );

    let index_path = repo_root.join(".capfind/index.cfi");
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    write_index(&index_path, &body, created, [0u8; 32])?;

    let size = std::fs::metadata(&index_path).map(|m| m.len()).unwrap_or(0);
    println!(
        "  Wrote {} ({:.1} KB)",
        index_path.display(),
        size as f64 / 1024.0
    );
    println!("  Total: {:.2}s", start.elapsed().as_secs_f64());
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
        bail!("No index found. Run `capfind index` first.");
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

    output::print_agent_json(&query, &filtered, &body.capabilities, start.elapsed())?;
    Ok(())
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
