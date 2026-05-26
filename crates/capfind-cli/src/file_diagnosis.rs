//! File-level parser/index coverage diagnosis for agents and dashboards.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use capfind_core::{Capability, IndexBody};
use serde_json::{json, Value};

use crate::config::CapfindConfig;
use crate::indexer;

pub const FILE_DIAGNOSIS_SCHEMA_VERSION: &str = "capfind.file_diagnosis.v1";

pub fn diagnose_file_json(
    repo_root: &Path,
    file_path: &Path,
    cfg: &CapfindConfig,
    index_body: Option<&IndexBody>,
    index_present: bool,
    index_error: Option<&str>,
) -> Result<Value> {
    let repo_root = repo_root
        .canonicalize()
        .with_context(|| format!("failed to resolve repo root {}", repo_root.display()))?;
    let cwd = std::env::current_dir().context("cannot determine CWD")?;
    let requested = if file_path.is_absolute() {
        file_path.to_path_buf()
    } else {
        cwd.join(file_path)
    };
    let absolute = requested.canonicalize().unwrap_or(requested.clone());
    let exists = absolute.exists();
    let is_dir = exists
        && std::fs::metadata(&absolute)
            .map(|m| m.is_dir())
            .unwrap_or(false);
    let rel_path = relative_path(&repo_root, &absolute, file_path);
    let rel_path_str = rel_path.to_string_lossy().to_string();

    let mut config_errors = Vec::new();
    let resolved_roots = match indexer::resolve_scan_roots(&repo_root, &cfg.index.roots) {
        Ok(roots) => Some(roots),
        Err(err) => {
            config_errors.push(err.to_string());
            None
        }
    };
    let include_set = match indexer::build_include_set(&cfg.index.include) {
        Ok(set) => set,
        Err(err) => {
            config_errors.push(err.to_string());
            None
        }
    };
    let capfind_ignore = match indexer::load_capfindignore(&repo_root) {
        Ok(ignore) => ignore,
        Err(err) => {
            config_errors.push(err.to_string());
            None
        }
    };

    let in_roots = resolved_roots
        .as_ref()
        .map(|roots| roots.iter().any(|root| absolute.starts_with(root)))
        .unwrap_or(true);
    let ignored_by_capfindignore =
        indexer::is_capfind_ignored(capfind_ignore.as_ref(), &absolute, is_dir);
    let included_by_config = indexer::matches_include(include_set.as_ref(), &rel_path);
    let supported_input = indexer::is_supported_index_input(&absolute);
    let built_in_skip = indexer::is_skipped_source_path(&absolute);
    let size_bytes = std::fs::metadata(&absolute).map(|m| m.len()).unwrap_or(0);
    let is_jar = absolute.extension().and_then(|e| e.to_str()) == Some("jar");
    let too_large = if is_jar {
        size_bytes > indexer::MAX_JAR_BYTES
    } else {
        size_bytes > indexer::MAX_SOURCE_BYTES
    };
    let parser_kind = parser_kind(&absolute);
    let readable = exists && !is_dir && (is_jar || size_bytes <= indexer::MAX_SOURCE_BYTES);
    let source_text = if readable && matches!(parser_kind.as_deref(), Some("java" | "go" | "proto"))
    {
        std::fs::read_to_string(&absolute).ok()
    } else {
        None
    };
    let preview_caps = preview_capabilities(&absolute, &source_text);
    let preview_count = preview_caps.len();

    let indexed_capabilities = index_body
        .map(|body| {
            body.capabilities
                .iter()
                .filter(|cap| cap.file == rel_path_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let indexed_file = index_body
        .map(|body| body.file_stats.contains_key(&rel_path_str))
        .unwrap_or(false);
    let stale = index_body.and_then(|body| stale_status(&absolute, &rel_path_str, body));

    let diagnosis = diagnose(DiagnosisInput {
        exists,
        is_dir,
        in_roots,
        ignored_by_capfindignore,
        included_by_glob: included_by_config,
        supported_input,
        builtin_skip: built_in_skip,
        too_large,
        index_present,
        index_error,
        index_loaded: index_body.is_some(),
        indexed_file,
        stale: index_body.is_some() && stale == Some(true),
        parser_kind: parser_kind.as_deref(),
        indexed_capabilities: indexed_capabilities.len(),
        preview_capabilities: preview_count,
    });
    let next_actions = next_actions(&diagnosis);

    Ok(json!({
        "schema_version": FILE_DIAGNOSIS_SCHEMA_VERSION,
        "repo_root": repo_root.display().to_string(),
        "file": {
            "input": file_path.display().to_string(),
            "absolute": absolute.display().to_string(),
            "relative": rel_path.display().to_string(),
            "exists": exists,
            "is_dir": is_dir,
            "size_bytes": size_bytes,
            "parser_kind": parser_kind,
        },
        "config": {
            "roots": &cfg.index.roots,
            "include": &cfg.index.include,
            "in_roots": in_roots,
            "capfindignore_ignored": ignored_by_capfindignore,
            "included_by_glob": included_by_config,
            "builtin_skip": built_in_skip,
            "supported_input": supported_input,
            "too_large": too_large,
            "config_errors": config_errors,
        },
        "index": {
            "present": index_present,
            "path": repo_root.join(".capfind/index.cfi").display().to_string(),
            "load_error": index_error,
            "indexed_file": indexed_file,
            "indexed_capabilities": indexed_capabilities.len(),
            "stale": stale,
        },
        "parser": {
            "supported_langs": supported_langs(),
            "supported_frameworks": supported_frameworks(parser_kind.as_deref()),
            "coverage_hints": parser_coverage_hints(parser_kind.as_deref(), preview_count),
            "preview_capabilities": preview_caps,
            "preview_count": preview_count,
            "supported_source_parser": parser_kind.as_deref().is_some_and(|kind| matches!(kind, "java" | "go" | "proto")),
            "reference_input": parser_kind.as_deref() == Some("reference_input"),
        },
        "diagnosis": diagnosis,
        "next_actions": next_actions,
        "signals": {
            "exists": exists,
            "in_roots": in_roots,
            "ignored_by_capfindignore": ignored_by_capfindignore,
            "included_by_glob": included_by_config,
            "supported_input": supported_input,
            "builtin_skip": built_in_skip,
            "too_large": too_large,
            "indexed_file": indexed_file,
            "indexed_capabilities": indexed_capabilities.len(),
            "parser_preview_capabilities": preview_count,
        }
    }))
}

struct DiagnosisInput<'a> {
    exists: bool,
    is_dir: bool,
    in_roots: bool,
    ignored_by_capfindignore: bool,
    included_by_glob: bool,
    supported_input: bool,
    builtin_skip: bool,
    too_large: bool,
    index_present: bool,
    index_error: Option<&'a str>,
    index_loaded: bool,
    indexed_file: bool,
    stale: bool,
    parser_kind: Option<&'a str>,
    indexed_capabilities: usize,
    preview_capabilities: usize,
}

fn diagnose(input: DiagnosisInput<'_>) -> String {
    if !input.exists {
        return "file_missing".to_string();
    }
    if input.is_dir {
        return "file_is_directory".to_string();
    }
    if !input.in_roots {
        return "outside_configured_roots".to_string();
    }
    if input.ignored_by_capfindignore {
        return "ignored_by_capfindignore".to_string();
    }
    if !input.included_by_glob {
        return "excluded_by_index_include".to_string();
    }
    if !input.supported_input {
        return "unsupported_file_type".to_string();
    }
    if input.builtin_skip {
        return "skipped_by_builtin_rule".to_string();
    }
    if input.too_large {
        return "file_too_large".to_string();
    }
    if !input.index_present {
        return "index_missing".to_string();
    }
    if input.index_error.is_some() {
        return "index_load_error".to_string();
    }
    if input.stale {
        return "index_stale_for_file".to_string();
    }
    if input.indexed_capabilities > 0 {
        return "indexed_with_capabilities".to_string();
    }
    if input.parser_kind.is_some() && input.preview_capabilities == 0 {
        return "parser_produced_zero_capabilities".to_string();
    }
    if input.parser_kind.is_some() && input.preview_capabilities > 0 && !input.indexed_file {
        return "parser_produced_capabilities_but_not_indexed".to_string();
    }
    if input.index_loaded && !input.indexed_file {
        return "eligible_but_not_indexed".to_string();
    }
    "eligible_but_no_capabilities".to_string()
}

fn next_actions(diagnosis: &str) -> Vec<&'static str> {
    match diagnosis {
        "file_missing" => vec!["verify_file_path", "check_repository_root"],
        "file_is_directory" => vec!["point_to_a_file", "diagnose_one_source_file"],
        "outside_configured_roots" => vec!["expand_index_roots", "move_file_under_index_root"],
        "ignored_by_capfindignore" => vec!["adjust_capfindignore", "remove_over_broad_ignore_rule"],
        "excluded_by_index_include" => vec!["expand_index_include_patterns", "reindex"],
        "unsupported_file_type" => vec!["use_supported_source_or_reference_file"],
        "skipped_by_builtin_rule" => vec!["move_file_out_of_generated_or_test_path"],
        "file_too_large" => vec!["split_file_or_raise_limit", "use_smaller_fixture"],
        "index_missing" => vec!["run_capfind_index", "allow_auto_index_in_mcp"],
        "index_load_error" => vec!["rebuild_index", "inspect_index_error"],
        "index_stale_for_file" => vec!["reindex", "inspect_changed_file_content"],
        "indexed_with_capabilities" => vec!["open_candidate_entries", "use_capfind_context_or_map"],
        "parser_produced_zero_capabilities" => vec![
            "add_supported_annotations_or_routes",
            "check_parser_support_matrix",
        ],
        "parser_produced_capabilities_but_not_indexed" => vec!["reindex", "check_config_filters"],
        "eligible_but_not_indexed" => vec!["reindex", "check_index_health"],
        _ => vec!["inspect_parser_output", "run_capfind_doctor"],
    }
}

fn supported_langs() -> Value {
    json!({
        "source": [
            {"language": "Java", "extensions": [".java"]},
            {"language": "Java/MyBatis XML", "extensions": [".xml"], "signals": ["<mapper namespace=...>"]},
            {"language": "Go", "extensions": [".go"]},
            {"language": "Proto", "extensions": [".proto"]},
        ],
        "reference": [
            {"kind": "maven_dependency", "paths": ["pom.xml"]},
            {"kind": "gradle_dependency", "paths": ["build.gradle", "build.gradle.kts"]},
            {"kind": "local_jar", "paths": ["*.jar"]},
        ],
        "builtin_skips": [
            "target/",
            "build/",
            "node_modules/",
            "vendor/",
            ".gradle/",
            "generated/",
            "*Test.java",
            "src/test/",
            "*_test.go",
            "*.pb.go"
        ]
    })
}

fn supported_frameworks(parser_kind: Option<&str>) -> Value {
    match parser_kind {
        Some("java") => json!([
            "spring_mvc",
            "spring_service",
            "spring_repository",
            "openfeign",
            "dubbo",
            "mybatis_xml",
            "jax_rs"
        ]),
        Some("go") => json!(["gin", "echo", "hertz", "chi", "net_http", "gorilla_mux"]),
        Some("proto") => json!(["grpc", "google_api_http"]),
        Some("java_mybatis_xml") => json!(["mybatis_xml"]),
        Some("reference_input") => {
            json!(["maven", "gradle", "local_jar", "sources_jar", "javadoc_jar"])
        }
        _ => json!([]),
    }
}

fn parser_coverage_hints(parser_kind: Option<&str>, preview_count: usize) -> Vec<&'static str> {
    match (parser_kind, preview_count) {
        (Some("java"), 0) => vec![
            "java_parser_requires_supported_annotations_or_component_stereotypes",
            "supported_java_frameworks_include_spring_feign_dubbo_mybatis_jaxrs",
            "if_framework_is_custom_add_eval_fixture_before_parser_work",
        ],
        (Some("go"), 0) => vec![
            "go_parser_targets_common_route_registration_calls",
            "supported_go_frameworks_include_gin_echo_hertz_chi_net_http_gorilla_mux",
        ],
        (Some("proto"), 0) => vec![
            "proto_parser_requires_service_rpc_blocks",
            "google_api_http_annotations_are_used_as_http_evidence_when_present",
        ],
        (Some("java_mybatis_xml"), 0) => {
            vec!["mybatis_xml_parser_requires_mapper_namespace_and_statement_ids"]
        }
        (Some("reference_input"), _) => {
            vec!["reference_inputs_expose_dependencies_and_available_jar_api_surface"]
        }
        (Some(_), _) => vec!["parser_preview_has_capabilities_reindex_if_index_is_stale"],
        (None, _) => vec!["unsupported_extension_add_parser_or_reference_input"],
    }
}

fn parser_kind(path: &Path) -> Option<String> {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if matches!(file_name, "pom.xml" | "build.gradle" | "build.gradle.kts") {
        return Some("reference_input".to_string());
    }
    match ext {
        "java" => Some("java".to_string()),
        "xml" => Some("java_mybatis_xml".to_string()),
        "go" => Some("go".to_string()),
        "proto" => Some("proto".to_string()),
        "jar" => Some("reference_input".to_string()),
        _ => None,
    }
}

fn preview_capabilities(path: &Path, source_text: &Option<String>) -> Vec<Value> {
    let Some(text) = source_text.as_ref() else {
        return Vec::new();
    };
    let caps = capfind_parsers::parse_file(path, text);
    caps.into_iter().take(8).map(capability_preview).collect()
}

fn capability_preview(cap: Capability) -> Value {
    json!({
        "kind": cap.kind.as_str(),
        "lang": cap.lang.as_str(),
        "class": cap.class,
        "method": cap.method,
        "signature": cap.signature,
        "file": cap.file,
        "line": cap.line,
        "http": cap.http.map(|http| json!({"method": http.method, "path": http.path})),
        "rpc": cap.rpc.map(|rpc| json!({
            "service": rpc.service,
            "rpc": rpc.rpc,
            "req": rpc.req,
            "rsp": rpc.rsp
        })),
    })
}

fn stale_status(path: &Path, rel_path: &str, body: &IndexBody) -> Option<bool> {
    let content = std::fs::read_to_string(path).ok()?;
    let stat = indexer::file_stat(path, &content)?;
    body.file_stats.get(rel_path).map(|old| old != &stat)
}

fn relative_path(repo_root: &Path, absolute: &Path, fallback: &Path) -> PathBuf {
    absolute
        .strip_prefix(repo_root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| fallback.to_path_buf())
}
