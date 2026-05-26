//! Golden evaluation for search and file-diagnosis quality.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use capfind_core::{Capability, IndexBody, Kind, Lang};
use serde::Deserialize;
use serde_json::{json, Value};

use capfind_search::{search, Hit};

use crate::asset_map;
use crate::config::CapfindConfig;
use crate::context;
use crate::file_diagnosis;
use crate::{EvalArgs, KindFilter, LangFilter};

pub const EVAL_SCHEMA_VERSION: &str = "capfind.eval.v1";

#[derive(Debug, Deserialize)]
struct QueryCase {
    id: String,
    query: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    match_mode: Option<String>,
    #[serde(default)]
    expected: Vec<ExpectedCapability>,
}

#[derive(Debug, Deserialize)]
struct FileCase {
    id: String,
    file: String,
    #[serde(default)]
    expected_diagnosis: Option<String>,
    #[serde(default)]
    signals: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct EdgeCase {
    id: String,
    relationship: String,
    #[serde(default)]
    source: Option<ExpectedCapability>,
    #[serde(default)]
    target: Option<ExpectedCapability>,
    #[serde(default)]
    evidence: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct DiagnosticCase {
    id: String,
    kind: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    file_contains: Option<String>,
    #[serde(default)]
    call: Option<String>,
    #[serde(default)]
    dependency_name: Option<String>,
    #[serde(default)]
    declared_type: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    candidate_count_min: Option<usize>,
    #[serde(default)]
    candidates: Vec<ExpectedDiagnosticCandidate>,
}

#[derive(Debug, Deserialize, Clone)]
struct ExpectedDiagnosticCandidate {
    #[serde(default, rename = "type")]
    candidate_type: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    name_contains: Option<String>,
    #[serde(default)]
    file_contains: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OwnershipCase {
    id: String,
    #[serde(default)]
    node_kind: Option<String>,
    #[serde(default, rename = "type")]
    candidate_type: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    name_contains: Option<String>,
    #[serde(default)]
    file_contains: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    ownership_kind: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct ExpectedCapability {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default, rename = "type")]
    candidate_type: Option<String>,
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    http_method: Option<String>,
    #[serde(default)]
    http_path: Option<String>,
    #[serde(default)]
    rpc_service: Option<String>,
    #[serde(default)]
    rpc_method: Option<String>,
    #[serde(default)]
    signature_contains: Option<String>,
    #[serde(default)]
    file_contains: Option<String>,
    #[serde(default)]
    is_reference: Option<bool>,
    #[serde(default)]
    tags_contains: Vec<String>,
    #[serde(default)]
    annotations_contains: Vec<String>,
}

pub fn eval_json(
    repo_root: &Path,
    body: &IndexBody,
    cfg: &CapfindConfig,
    args: &EvalArgs,
    index_present_before: bool,
) -> Result<Value> {
    let query_path = resolve_fixture_path(repo_root, &args.queries);
    let file_path = resolve_fixture_path(repo_root, &args.files);
    let edge_path = resolve_fixture_path(repo_root, &args.edges);
    let diagnostic_path = resolve_fixture_path(repo_root, &args.diagnostics);
    let ownership_path = resolve_fixture_path(repo_root, &args.ownerships);
    let query_cases = load_jsonl::<QueryCase>(&query_path)?;
    let file_cases = load_jsonl::<FileCase>(&file_path)?;
    let edge_cases = load_jsonl::<EdgeCase>(&edge_path)?;
    let diagnostic_cases = load_jsonl::<DiagnosticCase>(&diagnostic_path)?;
    let ownership_cases = load_jsonl::<OwnershipCase>(&ownership_path)?;

    if query_cases.is_empty()
        && file_cases.is_empty()
        && edge_cases.is_empty()
        && diagnostic_cases.is_empty()
        && ownership_cases.is_empty()
    {
        bail!(
            "No eval fixtures found. Provide at least one JSONL file at {}, {}, {}, {}, or {}.",
            query_path.display(),
            file_path.display(),
            edge_path.display(),
            diagnostic_path.display(),
            ownership_path.display()
        );
    }

    let query_results = query_cases
        .iter()
        .map(|case| evaluate_query_case(body, cfg, case, args.limit))
        .collect::<Result<Vec<_>>>()?;
    let file_results = file_cases
        .iter()
        .map(|case| evaluate_file_case(repo_root, body, cfg, case))
        .collect::<Result<Vec<_>>>()?;
    let asset_map =
        if edge_cases.is_empty() && diagnostic_cases.is_empty() && ownership_cases.is_empty() {
            None
        } else {
            Some(eval_asset_map(repo_root, body))
        };
    let edge_results = evaluate_edge_cases(asset_map.as_ref(), body, &edge_cases);
    let diagnostic_results = evaluate_diagnostic_cases(asset_map.as_ref(), &diagnostic_cases);
    let ownership_results = evaluate_ownership_cases(asset_map.as_ref(), &ownership_cases);

    let query_summary = summarize_query_results(&query_results);
    let file_summary = summarize_file_results(&file_results);
    let edge_summary = summarize_edge_results(&edge_results);
    let diagnostic_summary = summarize_diagnostic_results(&diagnostic_results);
    let ownership_summary = summarize_ownership_results(&ownership_results);

    Ok(json!({
        "schema_version": EVAL_SCHEMA_VERSION,
        "repo_root": repo_root.display().to_string(),
        "index": {
            "path": repo_root.join(".capfind/index.cfi").display().to_string(),
            "present_before": index_present_before,
            "capabilities": body.capabilities.len(),
            "indexed_files": body.file_stats.len(),
            "vocab": body.vocab.len(),
        },
        "fixtures": {
            "queries": query_path.display().to_string(),
            "files": file_path.display().to_string(),
            "edges": edge_path.display().to_string(),
            "diagnostics": diagnostic_path.display().to_string(),
            "ownerships": ownership_path.display().to_string(),
            "query_cases": query_results.len(),
            "file_cases": file_results.len(),
            "edge_cases": edge_results.len(),
            "diagnostic_cases": diagnostic_results.len(),
            "ownership_cases": ownership_results.len(),
        },
        "metrics": {
            "query": query_summary,
            "file": file_summary,
            "edge": edge_summary,
            "diagnostic": diagnostic_summary,
            "ownership": ownership_summary,
        },
        "query_results": query_results,
        "file_results": file_results,
        "edge_results": edge_results,
        "diagnostic_results": diagnostic_results,
        "ownership_results": ownership_results,
    }))
}

fn evaluate_query_case(
    body: &IndexBody,
    cfg: &CapfindConfig,
    case: &QueryCase,
    default_limit: usize,
) -> Result<Value> {
    let limit = case.limit.unwrap_or(default_limit).max(1);
    let lang = case.lang.as_deref().and_then(parse_lang_filter);
    let kind = case.kind.as_deref().and_then(parse_kind_filter);
    let path = case.path.clone();
    let search_limit = body.capabilities.len().max(limit).max(50);
    let hits = search(
        &case.query,
        &body.capabilities,
        &body.postings,
        &body.vocab,
        body.avgdl,
        &cfg.search,
        search_limit,
        false,
    );
    let filtered = filter_hits(&hits, &body.capabilities, &lang, &kind, &path, limit);
    let match_mode = case.match_mode.as_deref().unwrap_or("all");
    let expected_matches = case
        .expected
        .iter()
        .map(|expected| expected_rank(expected, &filtered, &body.capabilities))
        .collect::<Vec<_>>();
    let matched_expected = expected_matches
        .iter()
        .filter(|rank| rank.is_some())
        .count();
    let first_rank = expected_matches
        .iter()
        .flatten()
        .copied()
        .min()
        .unwrap_or(0);
    let passed = match match_mode {
        "any" => matched_expected > 0,
        _ => !case.expected.is_empty() && matched_expected == case.expected.len(),
    };
    let result_matches = filtered
        .iter()
        .filter(|hit| {
            case.expected
                .iter()
                .any(|expected| expected.matches(&body.capabilities[hit.cap_id as usize]))
        })
        .count();

    Ok(json!({
        "id": case.id,
        "query": case.query,
        "limit": limit,
        "expected_count": case.expected.len(),
        "matched_expected": matched_expected,
        "match_mode": match_mode,
        "passed": passed,
        "first_expected_rank": first_rank,
        "reciprocal_rank": if first_rank > 0 { 1.0 / first_rank as f64 } else { 0.0 },
        "filtered_results": filtered.iter().take(5).map(|hit| capability_summary(&body.capabilities[hit.cap_id as usize], hit.score)).collect::<Vec<_>>(),
        "expected": case.expected.iter().map(ExpectedCapability::to_json).collect::<Vec<_>>(),
        "result_match_count": result_matches,
    }))
}

fn evaluate_file_case(
    repo_root: &Path,
    body: &IndexBody,
    cfg: &CapfindConfig,
    case: &FileCase,
) -> Result<Value> {
    let file_path = resolve_fixture_path(repo_root, Path::new(&case.file));
    let diagnosis =
        file_diagnosis::diagnose_file_json(repo_root, &file_path, cfg, Some(body), true, None)?;
    let actual = diagnosis["diagnosis"].as_str().unwrap_or_default();
    let expected_diagnosis = case.expected_diagnosis.clone().unwrap_or_default();
    let signals = case
        .signals
        .iter()
        .map(|(key, expected)| {
            let actual_value = diagnosis["signals"]
                .get(key)
                .cloned()
                .unwrap_or(Value::Null);
            let passed = actual_value == *expected;
            json!({
                "name": key,
                "expected": expected,
                "actual": actual_value,
                "passed": passed,
            })
        })
        .collect::<Vec<_>>();
    let signals_passed = signals
        .iter()
        .all(|entry| entry["passed"].as_bool().unwrap_or(false));

    Ok(json!({
        "id": case.id,
        "file": case.file,
        "passed": actual == expected_diagnosis && signals_passed,
        "expected_diagnosis": expected_diagnosis,
        "diagnosis": actual,
        "signals": signals,
        "next_actions": diagnosis["next_actions"].clone(),
    }))
}

fn eval_asset_map(repo_root: &Path, body: &IndexBody) -> Value {
    let limit = body.capabilities.len().max(1);
    asset_map::asset_map_json_with_repo(body, repo_root, None, limit)
}

fn evaluate_edge_cases(map: Option<&Value>, body: &IndexBody, cases: &[EdgeCase]) -> Vec<Value> {
    if cases.is_empty() {
        return Vec::new();
    }
    let Some(map) = map else {
        return Vec::new();
    };
    cases
        .iter()
        .map(|case| evaluate_edge_case(map, &body.capabilities, case))
        .collect()
}

fn evaluate_edge_case(map: &Value, caps: &[Capability], case: &EdgeCase) -> Value {
    let matched_edge = map["graph"]["edges"]
        .as_array()
        .and_then(|edges| {
            edges.iter().find(|edge| {
                edge["relationship"] == case.relationship
                    && edge_endpoint_matches(edge["from"].as_str(), case.source.as_ref(), caps)
                    && edge_endpoint_matches(edge["to"].as_str(), case.target.as_ref(), caps)
                    && evidence_matches(edge, &case.evidence)
            })
        })
        .cloned();

    json!({
        "id": case.id,
        "relationship": case.relationship,
        "passed": matched_edge.is_some(),
        "expected": {
            "source": case.source.as_ref().map(ExpectedCapability::to_json),
            "target": case.target.as_ref().map(ExpectedCapability::to_json),
            "evidence": case.evidence,
        },
        "matched_edge": matched_edge.unwrap_or(Value::Null),
    })
}

fn evaluate_diagnostic_cases(map: Option<&Value>, cases: &[DiagnosticCase]) -> Vec<Value> {
    if cases.is_empty() {
        return Vec::new();
    }
    let Some(map) = map else {
        return Vec::new();
    };
    cases
        .iter()
        .map(|case| evaluate_diagnostic_case(map, case))
        .collect()
}

fn evaluate_diagnostic_case(map: &Value, case: &DiagnosticCase) -> Value {
    let matched_diagnostic = map["graph"]["diagnostics"]
        .as_array()
        .and_then(|diagnostics| {
            diagnostics
                .iter()
                .find(|diagnostic| diagnostic_matches(diagnostic, case))
        })
        .cloned();

    json!({
        "id": case.id,
        "kind": case.kind,
        "passed": matched_diagnostic.is_some(),
        "expected": diagnostic_case_json(case),
        "matched_diagnostic": matched_diagnostic.unwrap_or(Value::Null),
    })
}

fn diagnostic_matches(diagnostic: &Value, case: &DiagnosticCase) -> bool {
    if diagnostic["kind"].as_str() != Some(case.kind.as_str()) {
        return false;
    }
    if let Some(ref expected) = case.reason {
        if diagnostic["reason"].as_str() != Some(expected.as_str()) {
            return false;
        }
    }
    if let Some(ref expected) = case.file_contains {
        if !diagnostic["file"]
            .as_str()
            .is_some_and(|file| file.contains(expected))
        {
            return false;
        }
    }
    if let Some(ref expected) = case.call {
        if diagnostic["call"].as_str() != Some(expected.as_str()) {
            return false;
        }
    }
    if let Some(ref expected) = case.dependency_name {
        if diagnostic["dependency_name"].as_str() != Some(expected.as_str()) {
            return false;
        }
    }
    if let Some(ref expected) = case.declared_type {
        if diagnostic["declared_type"].as_str() != Some(expected.as_str()) {
            return false;
        }
    }
    if let Some(ref expected) = case.method {
        if diagnostic["method"].as_str() != Some(expected.as_str()) {
            return false;
        }
    }
    let candidates = diagnostic["candidates"].as_array();
    if let Some(min_count) = case.candidate_count_min {
        if candidates.map_or(0, Vec::len) < min_count {
            return false;
        }
    }
    case.candidates.iter().all(|expected| {
        candidates.is_some_and(|items| {
            items
                .iter()
                .any(|candidate| expected_diagnostic_candidate_matches(expected, candidate))
        })
    })
}

fn expected_diagnostic_candidate_matches(
    expected: &ExpectedDiagnosticCandidate,
    candidate: &Value,
) -> bool {
    if let Some(ref expected_type) = expected.candidate_type {
        if candidate["type"].as_str() != Some(expected_type.as_str()) {
            return false;
        }
    }
    if let Some(ref expected_owner) = expected.owner {
        if candidate["owner"].as_str() != Some(expected_owner.as_str()) {
            return false;
        }
    }
    if let Some(ref expected_name) = expected.name_contains {
        if !candidate["name"]
            .as_str()
            .is_some_and(|name| name.contains(expected_name))
        {
            return false;
        }
    }
    if let Some(ref expected_file) = expected.file_contains {
        if !candidate["file"]
            .as_str()
            .is_some_and(|file| file.contains(expected_file))
        {
            return false;
        }
    }
    true
}

fn diagnostic_case_json(case: &DiagnosticCase) -> Value {
    json!({
        "kind": case.kind,
        "reason": case.reason,
        "file_contains": case.file_contains,
        "call": case.call,
        "dependency_name": case.dependency_name,
        "declared_type": case.declared_type,
        "method": case.method,
        "candidate_count_min": case.candidate_count_min,
        "candidates": case.candidates.iter().map(ExpectedDiagnosticCandidate::to_json).collect::<Vec<_>>(),
    })
}

fn evaluate_ownership_cases(map: Option<&Value>, cases: &[OwnershipCase]) -> Vec<Value> {
    if cases.is_empty() {
        return Vec::new();
    }
    let Some(map) = map else {
        return Vec::new();
    };
    let entries = ownership_entries(map);
    cases
        .iter()
        .map(|case| evaluate_ownership_case(&entries, case))
        .collect()
}

fn evaluate_ownership_case(entries: &[Value], case: &OwnershipCase) -> Value {
    let matched_entry = entries
        .iter()
        .find(|entry| ownership_entry_matches(entry, case))
        .cloned();
    json!({
        "id": case.id,
        "passed": matched_entry.is_some(),
        "expected": ownership_case_json(case),
        "matched_entry": matched_entry.unwrap_or(Value::Null),
    })
}

fn ownership_entries(map: &Value) -> Vec<Value> {
    let mut entries = Vec::new();
    if let Some(modules) = map["modules"].as_array() {
        for module in modules {
            entries.push(json!({
                "node_kind": "module",
                "type": "module",
                "name": module["name"],
                "file": Value::Null,
                "ownership": module["ownership"],
            }));
        }
    }
    if let Some(nodes) = map["graph"]["nodes"].as_array() {
        for node in nodes {
            entries.push(json!({
                "node_kind": node["kind"],
                "type": node["type"],
                "name": node["name"],
                "file": node["file"],
                "ownership": node["ownership"],
            }));
        }
    }
    for key in ["dependencies", "api_usages", "jar_methods"] {
        if let Some(items) = map["external"][key].as_array() {
            for item in items {
                entries.push(json!({
                    "node_kind": item["type"],
                    "type": item["type"],
                    "name": item["name"],
                    "file": item["file"],
                    "ownership": item["ownership"],
                }));
            }
        }
    }
    entries
}

fn ownership_entry_matches(entry: &Value, case: &OwnershipCase) -> bool {
    if let Some(ref expected) = case.node_kind {
        if entry["node_kind"].as_str() != Some(expected.as_str()) {
            return false;
        }
    }
    if let Some(ref expected) = case.candidate_type {
        if entry["type"].as_str() != Some(expected.as_str()) {
            return false;
        }
    }
    if let Some(ref expected) = case.name {
        if entry["name"].as_str() != Some(expected.as_str()) {
            return false;
        }
    }
    if let Some(ref expected) = case.name_contains {
        if !entry["name"]
            .as_str()
            .is_some_and(|name| name.contains(expected))
        {
            return false;
        }
    }
    if let Some(ref expected) = case.file_contains {
        if !entry["file"]
            .as_str()
            .is_some_and(|file| file.contains(expected))
        {
            return false;
        }
    }
    if let Some(ref expected) = case.owner {
        if entry["ownership"]["owner"].as_str() != Some(expected.as_str()) {
            return false;
        }
    }
    if let Some(ref expected) = case.source {
        if entry["ownership"]["source"].as_str() != Some(expected.as_str()) {
            return false;
        }
    }
    if let Some(ref expected) = case.ownership_kind {
        if entry["ownership"]["kind"].as_str() != Some(expected.as_str()) {
            return false;
        }
    }
    true
}

fn ownership_case_json(case: &OwnershipCase) -> Value {
    json!({
        "node_kind": case.node_kind,
        "type": case.candidate_type,
        "name": case.name,
        "name_contains": case.name_contains,
        "file_contains": case.file_contains,
        "owner": case.owner,
        "source": case.source,
        "ownership_kind": case.ownership_kind,
    })
}

fn edge_endpoint_matches(
    node_id: Option<&str>,
    expected: Option<&ExpectedCapability>,
    caps: &[Capability],
) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let Some(cap_id) = node_id.and_then(cap_id_from_node_id) else {
        return false;
    };
    caps.iter()
        .find(|cap| cap.id == cap_id)
        .is_some_and(|cap| expected.matches(cap))
}

fn cap_id_from_node_id(node_id: &str) -> Option<u32> {
    node_id.strip_prefix("cap:")?.parse().ok()
}

fn evidence_matches(edge: &Value, expected: &BTreeMap<String, Value>) -> bool {
    expected
        .iter()
        .all(|(key, value)| edge["evidence"].get(key) == Some(value))
}

fn summarize_query_results(results: &[Value]) -> Value {
    let total = results.len();
    let passed = results
        .iter()
        .filter(|result| result["passed"].as_bool().unwrap_or(false))
        .count();
    let expected_total = results
        .iter()
        .map(|result| result["expected_count"].as_u64().unwrap_or(0))
        .sum::<u64>();
    let matched_expected = results
        .iter()
        .map(|result| result["matched_expected"].as_u64().unwrap_or(0))
        .sum::<u64>();
    let reciprocal_sum = results
        .iter()
        .map(|result| result["reciprocal_rank"].as_f64().unwrap_or(0.0))
        .sum::<f64>();
    let matched_results = results
        .iter()
        .map(|result| result["result_match_count"].as_u64().unwrap_or(0))
        .sum::<u64>();
    let result_total = results
        .iter()
        .map(|result| {
            result["filtered_results"]
                .as_array()
                .map(|arr| arr.len() as u64)
                .unwrap_or(0)
        })
        .sum::<u64>();

    json!({
        "cases": total,
        "passed": passed,
        "pass_rate": ratio(passed as u64, total as u64),
        "recall_at_k": ratio(matched_expected, expected_total),
        "mrr": if total > 0 { reciprocal_sum / total as f64 } else { 0.0 },
        "matched_expected": matched_expected,
        "expected_total": expected_total,
        "known_precision_at_k": ratio(matched_results, result_total),
        "result_match_rate": ratio(matched_results, result_total),
        "failed_case_ids": results
            .iter()
            .filter(|result| !result["passed"].as_bool().unwrap_or(false))
            .map(|result| result["id"].clone())
            .collect::<Vec<_>>(),
    })
}

fn summarize_file_results(results: &[Value]) -> Value {
    let total = results.len();
    let passed = results
        .iter()
        .filter(|result| result["passed"].as_bool().unwrap_or(false))
        .count();
    json!({
        "cases": total,
        "passed": passed,
        "pass_rate": ratio(passed as u64, total as u64),
        "diagnosis_pass_rate": ratio(passed as u64, total as u64),
        "failed_case_ids": results
            .iter()
            .filter(|result| !result["passed"].as_bool().unwrap_or(false))
            .map(|result| result["id"].clone())
            .collect::<Vec<_>>(),
    })
}

fn summarize_edge_results(results: &[Value]) -> Value {
    let total = results.len();
    let passed = results
        .iter()
        .filter(|result| result["passed"].as_bool().unwrap_or(false))
        .count();
    json!({
        "cases": total,
        "passed": passed,
        "pass_rate": ratio(passed as u64, total as u64),
        "edge_pass_rate": ratio(passed as u64, total as u64),
        "failed_case_ids": results
            .iter()
            .filter(|result| !result["passed"].as_bool().unwrap_or(false))
            .map(|result| result["id"].clone())
            .collect::<Vec<_>>(),
    })
}

fn summarize_diagnostic_results(results: &[Value]) -> Value {
    let total = results.len();
    let passed = results
        .iter()
        .filter(|result| result["passed"].as_bool().unwrap_or(false))
        .count();
    json!({
        "cases": total,
        "passed": passed,
        "pass_rate": ratio(passed as u64, total as u64),
        "diagnostic_pass_rate": ratio(passed as u64, total as u64),
        "failed_case_ids": results
            .iter()
            .filter(|result| !result["passed"].as_bool().unwrap_or(false))
            .map(|result| result["id"].clone())
            .collect::<Vec<_>>(),
    })
}

fn summarize_ownership_results(results: &[Value]) -> Value {
    let total = results.len();
    let passed = results
        .iter()
        .filter(|result| result["passed"].as_bool().unwrap_or(false))
        .count();
    json!({
        "cases": total,
        "passed": passed,
        "pass_rate": ratio(passed as u64, total as u64),
        "ownership_pass_rate": ratio(passed as u64, total as u64),
        "failed_case_ids": results
            .iter()
            .filter(|result| !result["passed"].as_bool().unwrap_or(false))
            .map(|result| result["id"].clone())
            .collect::<Vec<_>>(),
    })
}

fn capability_summary(cap: &Capability, score: f32) -> Value {
    let mut summary = json!({
        "id": cap.id,
        "score": (score * 10.0).round() / 10.0,
        "kind": cap.kind.as_str(),
        "lang": cap.lang.as_str(),
        "class": cap.class,
        "method": cap.method,
        "signature": cap.signature,
        "file": cap.file,
        "line": cap.line,
        "is_reference": context::is_external(cap),
    });
    if let Some(ref http) = cap.http {
        summary["http"] = json!({"method": http.method, "path": http.path});
    }
    if let Some(ref rpc) = cap.rpc {
        summary["rpc"] = json!({
            "service": rpc.service,
            "rpc": rpc.rpc,
            "req": rpc.req,
            "rsp": rpc.rsp,
        });
    }
    summary
}

fn expected_rank(
    expected: &ExpectedCapability,
    hits: &[&Hit],
    caps: &[Capability],
) -> Option<usize> {
    hits.iter()
        .position(|hit| expected.matches(&caps[hit.cap_id as usize]))
        .map(|idx| idx + 1)
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

fn resolve_fixture_path(repo_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
}

fn load_jsonl<T>(path: &Path) -> Result<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let mut values = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let value = serde_json::from_str(trimmed)
            .with_context(|| format!("invalid JSON at {}:{}", path.display(), line_no + 1))?;
        values.push(value);
    }
    Ok(values)
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

fn ratio(numer: u64, denom: u64) -> f64 {
    if denom == 0 {
        0.0
    } else {
        numer as f64 / denom as f64
    }
}

impl ExpectedDiagnosticCandidate {
    fn to_json(&self) -> Value {
        json!({
            "type": self.candidate_type,
            "owner": self.owner,
            "name_contains": self.name_contains,
            "file_contains": self.file_contains,
        })
    }
}

impl ExpectedCapability {
    fn matches(&self, cap: &Capability) -> bool {
        if let Some(ref expected) = self.kind {
            if !cap.kind.as_str().eq_ignore_ascii_case(expected) {
                return false;
            }
        }
        if let Some(ref expected) = self.lang {
            if !cap.lang.as_str().eq_ignore_ascii_case(expected) {
                return false;
            }
        }
        if let Some(ref expected) = self.candidate_type {
            if !context::candidate_type(cap).eq_ignore_ascii_case(expected) {
                return false;
            }
        }
        if let Some(ref expected) = self.class {
            if cap.class.as_deref() != Some(expected.as_str()) {
                return false;
            }
        }
        if let Some(ref expected) = self.method {
            if cap.method != *expected {
                return false;
            }
        }
        if let Some(ref expected) = self.http_method {
            if cap.http.as_ref().map(|http| http.method.as_str()) != Some(expected.as_str()) {
                return false;
            }
        }
        if let Some(ref expected) = self.http_path {
            if cap.http.as_ref().map(|http| http.path.as_str()) != Some(expected.as_str()) {
                return false;
            }
        }
        if let Some(ref expected) = self.rpc_service {
            if cap.rpc.as_ref().map(|rpc| rpc.service.as_str()) != Some(expected.as_str()) {
                return false;
            }
        }
        if let Some(ref expected) = self.rpc_method {
            if cap.rpc.as_ref().map(|rpc| rpc.rpc.as_str()) != Some(expected.as_str()) {
                return false;
            }
        }
        if let Some(ref expected) = self.signature_contains {
            if !cap.signature.contains(expected) {
                return false;
            }
        }
        if let Some(ref expected) = self.file_contains {
            if !cap.file.contains(expected) {
                return false;
            }
        }
        if let Some(expected) = self.is_reference {
            if context::is_external(cap) != expected {
                return false;
            }
        }
        if !self.tags_contains.is_empty()
            && !self
                .tags_contains
                .iter()
                .all(|expected| cap.tags.iter().any(|tag| tag == expected))
        {
            return false;
        }
        if !self.annotations_contains.is_empty()
            && !self.annotations_contains.iter().all(|expected| {
                cap.annotations
                    .iter()
                    .any(|annotation| annotation == expected)
            })
        {
            return false;
        }
        true
    }

    fn to_json(&self) -> Value {
        json!({
            "kind": self.kind,
            "lang": self.lang,
            "type": self.candidate_type,
            "class": self.class,
            "method": self.method,
            "http_method": self.http_method,
            "http_path": self.http_path,
            "rpc_service": self.rpc_service,
            "rpc_method": self.rpc_method,
            "signature_contains": self.signature_contains,
            "file_contains": self.file_contains,
            "is_reference": self.is_reference,
            "tags_contains": self.tags_contains,
            "annotations_contains": self.annotations_contains,
        })
    }
}
