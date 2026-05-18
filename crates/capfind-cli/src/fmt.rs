//! Output formatting: compact terminal + JSON.

use std::time::Duration;

use capfind_core::Capability;
use capfind_search::Hit;

/// Print compact 3-line-per-result format to stdout.
pub fn print_compact(hits: &[&Hit], caps: &[Capability], no_color: bool) {
    use is_terminal::IsTerminal;
    let color = !no_color && std::io::stdout().is_terminal();

    for hit in hits {
        let cap = &caps[hit.cap_id as usize];
        let kind_str = cap.kind.as_str();

        // Line 1: HTTP method + path (or kind) + score
        if let Some(ref http) = cap.http {
            let method_colored = if color {
                colorize_method(&http.method)
            } else {
                http.method.clone()
            };
            println!(
                "{} {}  ·  {}  ·  score {:.1}",
                method_colored, http.path, kind_str, hit.score
            );
        } else {
            println!(
                "{}  ·  {}  ·  score {:.1}",
                cap.qualified(),
                kind_str,
                hit.score
            );
        }

        // Line 2: qualified method + signature.
        println!("  {}({})", cap.qualified(), short_params(&cap.signature));

        // Line 3: file:line
        println!("  {}:{}", cap.file, cap.line);
        if is_reference(cap) {
            if let Some(ref doc) = cap.doc {
                println!("  reference: {}", doc);
            }
        }

        if let Some(ref explain) = hit.explain {
            println!(
                "  explain: bm25 {:.2} -> final {:.2}",
                explain.bm25_raw, explain.final_score
            );
            if !explain.boosts.is_empty() {
                let boosts = explain
                    .boosts
                    .iter()
                    .map(|(name, factor)| format!("{}×{:.2}", name, factor))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("           boosts: {}", boosts);
            }
            if !explain.term_hits.is_empty() {
                let terms = explain
                    .term_hits
                    .iter()
                    .take(6)
                    .map(|t| format!("{}@{:?} tf={} w={:.2}", t.term, t.field, t.tf, t.weight))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("           terms: {}", terms);
            }
        }

        println!();
    }
}

/// Print JSON output.
pub fn print_json(
    query: &str,
    hits: &[&Hit],
    caps: &[Capability],
    elapsed: Duration,
) -> anyhow::Result<()> {
    use serde_json::json;

    let results: Vec<_> = hits
        .iter()
        .map(|h| {
            let cap = &caps[h.cap_id as usize];
            let mut obj = json!({
                "id": cap.id,
                "score": (h.score * 10.0).round() / 10.0,
                "kind": cap.kind.as_str(),
                "lang": cap.lang.as_str(),
                "class": cap.class,
                "method": cap.method,
                "signature": cap.signature,
                "annotations": cap.annotations,
                "tags": cap.tags,
                "doc": cap.doc,
                "is_reference": is_reference(cap),
                "file": cap.file,
                "line": cap.line,
            });
            if let Some(ref http) = cap.http {
                obj["http"] = json!({
                    "method": http.method,
                    "path": http.path,
                });
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
            if let Some(ref explain) = h.explain {
                obj["explain"] = json!({
                    "bm25_raw": explain.bm25_raw,
                    "boosts": explain.boosts.iter().map(|(name, factor)| json!({
                        "name": name,
                        "factor": factor,
                    })).collect::<Vec<_>>(),
                    "term_hits": explain.term_hits.iter().map(|t| json!({
                        "term": t.term,
                        "field": format!("{:?}", t.field),
                        "tf": t.tf,
                        "weight": t.weight,
                        "is_synonym": t.is_synonym,
                    })).collect::<Vec<_>>(),
                    "final_score": explain.final_score,
                });
            }
            obj
        })
        .collect();

    let output = json!({
        "query": query,
        "took_ms": elapsed.as_millis(),
        "results": results,
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub const AGENT_SCHEMA_VERSION: &str = "capfind.agent.v1";

/// Print agent-friendly JSON preflight output.
pub fn print_agent_json(
    task: &str,
    hits: &[&Hit],
    caps: &[Capability],
    elapsed: Duration,
    fail_on_candidates: bool,
) -> anyhow::Result<()> {
    use serde_json::json;

    let candidates: Vec<_> = hits
        .iter()
        .map(|h| {
            let cap = &caps[h.cap_id as usize];
            let mut obj = json!({
                "id": cap.id,
                "score": (h.score * 10.0).round() / 10.0,
                "kind": cap.kind.as_str(),
                "lang": cap.lang.as_str(),
                "class": cap.class,
                "method": cap.method,
                "signature": cap.signature,
                "annotations": cap.annotations,
                "tags": cap.tags,
                "doc": cap.doc,
                "is_reference": is_reference(cap),
                "file": cap.file,
                "line": cap.line,
            });
            if let Some(ref http) = cap.http {
                obj["http"] = json!({
                    "method": http.method,
                    "path": http.path,
                });
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
        })
        .collect();

    let has_candidates = !candidates.is_empty();
    let recommendation = if has_candidates {
        "review_existing_capability_before_implementing"
    } else {
        "no_similar_capability_found"
    };
    let agent_hint = if has_candidates {
        "Review the candidates and cited file:line locations before creating new code. Prefer reuse or extension when appropriate."
    } else {
        "No indexed capability matched this task. It may be safe to implement, but verify domain context first."
    };
    let next_actions = if has_candidates {
        vec![
            "open_candidate_file_lines",
            "prefer_reuse_or_extension",
            "ask_user_before_creating_duplicate_capability",
        ]
    } else {
        vec![
            "verify_domain_context",
            "continue_implementation_if_no_conflict",
        ]
    };

    let output = json!({
        "schema_version": AGENT_SCHEMA_VERSION,
        "task": task,
        "took_ms": elapsed.as_millis(),
        "has_candidates": has_candidates,
        "recommendation": recommendation,
        "agent_hint": agent_hint,
        "exit_policy": {
            "fail_on_candidates": fail_on_candidates,
            "candidate_exit_code": if fail_on_candidates { 2 } else { 0 },
            "no_candidate_exit_code": 0,
        },
        "next_actions": next_actions,
        "candidates": candidates,
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn is_reference(cap: &Capability) -> bool {
    cap.tags.iter().any(|tag| tag == "external")
        || cap
            .annotations
            .iter()
            .any(|annotation| annotation.starts_with("external_"))
}

fn colorize_method(method: &str) -> String {
    use owo_colors::OwoColorize;
    match method {
        "GET" => format!("{}", "GET".green()),
        "POST" => format!("{}", "POST".yellow()),
        "PUT" => format!("{}", "PUT".blue()),
        "DELETE" => format!("{}", "DELETE".red()),
        "PATCH" => format!("{}", "PATCH".cyan()),
        _ => method.to_string(),
    }
}

/// Extract params portion from a signature like "ReturnType method(Param1, Param2)"
fn short_params(sig: &str) -> &str {
    if let Some(start) = sig.find('(') {
        if let Some(end) = sig.rfind(')') {
            return &sig[start + 1..end];
        }
    }
    ""
}
