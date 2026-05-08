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
            println!("{} {}  ·  {}  ·  score {:.1}",
                method_colored, http.path, kind_str, hit.score);
        } else {
            println!("{}  ·  {}  ·  score {:.1}",
                cap.qualified(), kind_str, hit.score);
        }

        // Line 2: qualified method + signature.
        println!("  {}({})",
            cap.qualified(),
            short_params(&cap.signature));

        // Line 3: file:line
        println!("  {}:{}", cap.file, cap.line);

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

    let results: Vec<_> = hits.iter().map(|h| {
        let cap = &caps[h.cap_id as usize];
        let mut obj = json!({
            "id": cap.id,
            "score": (h.score * 10.0).round() / 10.0,
            "kind": cap.kind.as_str(),
            "lang": cap.lang.as_str(),
            "class": cap.class,
            "method": cap.method,
            "signature": cap.signature,
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
            });
        }
        obj
    }).collect();

    let output = json!({
        "query": query,
        "took_ms": elapsed.as_millis(),
        "results": results,
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
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
