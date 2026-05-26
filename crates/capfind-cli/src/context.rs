//! Low-token AI context schema for MCP / agent integrations.

use std::time::Duration;

use capfind_core::{Capability, Kind};
use capfind_search::Hit;
use serde_json::{json, Value};

pub const CONTEXT_SCHEMA_VERSION: &str = "capfind.context.v1";

pub fn context_json(task: &str, hits: &[&Hit], caps: &[Capability], elapsed: Duration) -> Value {
    let top_score = hits.first().map(|hit| hit.score).unwrap_or(0.0).max(0.1);
    let candidates = hits
        .iter()
        .map(|hit| {
            let cap = &caps[hit.cap_id as usize];
            candidate_json(cap, hit.score, top_score)
        })
        .collect::<Vec<_>>();
    let decision = context_decision(&candidates);
    json!({
        "schema_version": CONTEXT_SCHEMA_VERSION,
        "task": task,
        "took_ms": elapsed.as_millis(),
        "decision": decision,
        "has_candidates": !candidates.is_empty(),
        "next_actions": next_actions(decision),
        "candidates": candidates,
    })
}

pub fn candidate_json(cap: &Capability, score: f32, top_score: f32) -> Value {
    let ctype = candidate_type(cap);
    json!({
        "id": cap.id,
        "type": ctype,
        "name": candidate_name(cap),
        "owner": owner_or_source(cap),
        "does": does_text(cap),
        "helps_with": helps_with(cap, ctype),
        "entrypoints": entrypoints(cap),
        "relationship": relationship(cap, ctype),
        "evidence": [evidence(cap)],
        "callability": callability(ctype),
        "confidence": confidence(score, top_score),
        "meta": metadata(cap),
    })
}

pub fn candidate_type(cap: &Capability) -> &'static str {
    if is_external(cap) {
        if has_marker(cap, "jar_method") {
            "jar_method"
        } else if has_marker(cap, "jar_class") {
            "jar_class"
        } else if has_marker(cap, "source_method") {
            "sources_jar_method"
        } else if has_marker(cap, "javadoc_method") {
            "javadoc_method"
        } else if has_marker(cap, "external_dependency") {
            "external_dependency"
        } else if has_marker(cap, "java_import") {
            "external_api_usage"
        } else {
            "external_api"
        }
    } else {
        match cap.kind {
            Kind::HttpEndpoint => "http_endpoint",
            Kind::RpcMethod => "rpc_method",
            Kind::ServiceMethod => "service_method",
            Kind::DaoMethod => "dao_method",
            Kind::Other => "internal_capability",
        }
    }
}

pub fn is_external(cap: &Capability) -> bool {
    cap.tags.iter().any(|tag| tag == "external")
        || cap
            .annotations
            .iter()
            .any(|annotation| annotation.starts_with("external_"))
}

fn context_decision(candidates: &[Value]) -> &'static str {
    let Some(best) = candidates.first() else {
        return "no_match";
    };
    let ctype = best.get("type").and_then(Value::as_str).unwrap_or_default();
    let confidence = best
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or_default();
    if matches!(
        ctype,
        "http_endpoint" | "rpc_method" | "service_method" | "dao_method" | "internal_capability"
    ) && confidence >= 0.75
    {
        "block_duplicate_candidate"
    } else {
        "inform"
    }
}

fn next_actions(decision: &str) -> Vec<&'static str> {
    match decision {
        "block_duplicate_candidate" => vec![
            "inspect_candidate_entrypoint",
            "prefer_reuse_or_extension_if_task_matches",
            "record_adoption_after_generation",
        ],
        "inform" => vec![
            "decide_direct_call_vs_wrapper",
            "inspect_candidate_if_needed",
            "record_adoption_after_generation",
        ],
        _ => vec![
            "continue_with_domain_verification",
            "run_capfind_doctor_if_unexpected",
        ],
    }
}

fn candidate_name(cap: &Capability) -> String {
    if let Some(ref http) = cap.http {
        return format!("{} {}", http.method, http.path);
    }
    if let Some(ref rpc) = cap.rpc {
        return format!("{}.{}", rpc.service, rpc.rpc);
    }
    if let Some(ref class) = cap.class {
        format!("{class}#{}", cap.method)
    } else {
        cap.method.clone()
    }
}

fn owner_or_source(cap: &Capability) -> String {
    if let Some(ref class) = cap.class {
        return class.clone();
    }
    if !cap.package.is_empty() {
        return cap.package.clone();
    }
    if !cap.module.is_empty() {
        return cap.module.clone();
    }
    cap.file.clone()
}

fn does_text(cap: &Capability) -> String {
    if let Some(ref http) = cap.http {
        return format!("handles {} {}", http.method, http.path);
    }
    if let Some(ref rpc) = cap.rpc {
        return format!("defines RPC {}.{}", rpc.service, rpc.rpc);
    }
    if let Some(ref doc) = cap.doc {
        return shorten(doc, 140);
    }
    shorten(&cap.signature, 140)
}

fn helps_with(cap: &Capability, ctype: &str) -> &'static str {
    match ctype {
        "http_endpoint" => "reuse or extend an existing HTTP capability",
        "rpc_method" => "call or extend an existing RPC contract",
        "service_method" => "reuse internal business logic",
        "dao_method" => "reuse data-access behavior",
        "jar_method" | "sources_jar_method" | "javadoc_method" => {
            "call an available dependency API or wrap it locally"
        }
        "external_dependency" => "confirm a dependency is already available",
        "external_api_usage" => "follow an API already used by project code",
        "jar_class" => "discover an available dependency class",
        _ if is_external(cap) => "discover referenced external API surface",
        _ => "reuse an indexed project capability",
    }
}

fn entrypoints(cap: &Capability) -> Vec<Value> {
    if let Some(ref http) = cap.http {
        return vec![json!({"kind": "http", "value": format!("{} {}", http.method, http.path)})];
    }
    if let Some(ref rpc) = cap.rpc {
        return vec![json!({"kind": "rpc", "value": format!("{}.{}", rpc.service, rpc.rpc)})];
    }
    vec![json!({"kind": "signature", "value": shorten(&cap.signature, 180)})]
}

fn relationship(cap: &Capability, ctype: &str) -> Value {
    json!({
        "scope": if is_external(cap) { "external" } else { "project" },
        "layer": ctype,
        "used_by_project": is_external_usage(cap),
        "wrapped_by_project": matches!(ctype, "external_api_usage"),
    })
}

fn evidence(cap: &Capability) -> Value {
    json!({
        "file": cap.file,
        "line": cap.line,
        "reason": evidence_reason(cap),
    })
}

fn evidence_reason(cap: &Capability) -> &'static str {
    if has_marker(cap, "external_dependency") {
        "dependency declared by project"
    } else if has_marker(cap, "java_import") {
        "project source imports this API"
    } else if has_marker(cap, "jar_method") {
        "method discovered inside local jar"
    } else if has_marker(cap, "source_method") {
        "method documented in sources jar"
    } else if has_marker(cap, "javadoc_method") {
        "method documented in javadoc jar"
    } else if cap.http.is_some() {
        "HTTP entrypoint definition"
    } else if cap.rpc.is_some() {
        "RPC contract definition"
    } else {
        "indexed capability definition"
    }
}

fn callability(ctype: &str) -> &'static str {
    match ctype {
        "jar_method" | "sources_jar_method" | "javadoc_method" | "external_api_usage" => {
            "direct_or_wrap"
        }
        "external_dependency" | "jar_class" => "inspect_then_call",
        _ => "direct",
    }
}

fn metadata(cap: &Capability) -> Value {
    let mut meta = serde_json::Map::new();
    for marker in &cap.annotations {
        if marker == "static" {
            meta.insert("static".to_string(), json!(true));
        } else if marker == "deprecated" {
            meta.insert("deprecated".to_string(), json!(true));
        } else if let Some(access) = marker.strip_prefix("access:") {
            meta.insert("access".to_string(), json!(access));
        } else if let Some(throws) = marker.strip_prefix("throws:") {
            let values = throws
                .split(',')
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();
            meta.insert("throws".to_string(), json!(values));
        } else if let Some(generic) = marker.strip_prefix("generic:") {
            meta.insert("generic".to_string(), json!(generic));
        }
    }
    Value::Object(meta)
}

fn confidence(score: f32, top_score: f32) -> f32 {
    let relative = if top_score <= 0.0 {
        0.0
    } else {
        (score / top_score).clamp(0.0, 1.0)
    };
    ((relative * 100.0).round() / 100.0).max(0.01)
}

fn is_external_usage(cap: &Capability) -> bool {
    has_marker(cap, "java_import") || has_marker(cap, "external_dependency")
}

fn has_marker(cap: &Capability, marker: &str) -> bool {
    cap.tags.iter().any(|tag| tag == marker)
        || cap
            .annotations
            .iter()
            .any(|annotation| annotation == marker)
}

fn shorten(input: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in input.chars().take(max_chars) {
        out.push(ch);
    }
    if input.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}
