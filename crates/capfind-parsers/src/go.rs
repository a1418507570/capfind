//! Go parser — regex-based extraction of common HTTP route registrations.
//!
//! # Coverage
//!
//! This first slice intentionally focuses on route registration calls that are
//! common in Gin, Echo, Hertz, net/http, and gorilla/mux style code:
//!
//! - `router.GET("/path", handler)` / `router.POST(...)`
//! - `http.HandleFunc("/path", handler)`
//! - `mux.HandleFunc("/path", handler).Methods("GET")`
//!
//! It does not try to be a full Go AST parser yet. That remains a v0.2 track.

use capfind_core::{Capability, HttpInfo, Kind, Lang};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;

static ROUTE_CALL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?m)\b(?:[A-Za-z_][\w]*\.)?(GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS|Any|ANY|HandleFunc)\s*\(\s*"([^"]+)"\s*,\s*([A-Za-z_][\w.]*)"#,
    )
    .unwrap()
});

static GORILLA_METHODS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?s)\b(?:[A-Za-z_][\w]*\.)?HandleFunc\s*\(\s*"([^"]+)"\s*,\s*([A-Za-z_][\w.]*)\s*\)\s*\.Methods\s*\(([^)]*)\)"#,
    )
    .unwrap()
});

static STRING_LITERAL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#""([^"]+)""#).unwrap());

/// Parse a single `.go` file and return all detected capabilities.
pub fn parse_go_file(path: &Path, src: &str) -> Vec<Capability> {
    let file = path.to_string_lossy().to_string();
    let mut caps = Vec::new();

    for captures in GORILLA_METHODS_RE.captures_iter(src) {
        let whole = captures.get(0).unwrap();
        let path = captures.get(1).unwrap().as_str();
        let handler = captures.get(2).unwrap().as_str();
        let methods = captures.get(3).unwrap().as_str();
        let http_method = extract_methods(methods).unwrap_or_else(|| "ANY".to_string());
        caps.push(route_capability(
            &file,
            src,
            whole.start(),
            whole.end(),
            whole.as_str(),
            &http_method,
            path,
            handler,
        ));
    }

    for captures in ROUTE_CALL_RE.captures_iter(src) {
        let whole = captures.get(0).unwrap();
        let route_kind = captures.get(1).unwrap().as_str();
        if route_kind == "HandleFunc" && has_methods_suffix(src, whole.end()) {
            continue;
        }

        let path = captures.get(2).unwrap().as_str();
        let handler = captures.get(3).unwrap().as_str();
        let http_method = normalize_method(route_kind);
        caps.push(route_capability(
            &file,
            src,
            whole.start(),
            whole.end(),
            whole.as_str(),
            &http_method,
            path,
            handler,
        ));
    }

    caps.sort_by_key(|cap| cap.byte_range.0);
    caps
}

#[allow(clippy::too_many_arguments)]
fn route_capability(
    file: &str,
    src: &str,
    start: usize,
    end: usize,
    annotation: &str,
    http_method: &str,
    path: &str,
    handler: &str,
) -> Capability {
    let (class, method) = split_handler(handler);
    Capability {
        id: 0,
        kind: Kind::HttpEndpoint,
        lang: Lang::Go,
        module: String::new(),
        package: String::new(),
        class,
        method: method.to_string(),
        signature: format!("{}({})", method, handler),
        annotations: vec![annotation.to_string()],
        http: Some(HttpInfo {
            method: http_method.to_string(),
            path: path.to_string(),
            consumes: None,
            produces: None,
        }),
        rpc: None,
        doc: None,
        tags: vec![],
        file: file.to_string(),
        line: byte_offset_to_line(src, start),
        byte_range: (start as u32, end as u32),
        terms: vec![],
    }
}

fn normalize_method(method: &str) -> String {
    match method {
        "Any" | "ANY" | "HandleFunc" => "ANY".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

fn extract_methods(input: &str) -> Option<String> {
    let methods: Vec<String> = STRING_LITERAL_RE
        .captures_iter(input)
        .filter_map(|capture| capture.get(1))
        .map(|method| method.as_str().to_ascii_uppercase())
        .collect();
    if methods.is_empty() {
        None
    } else {
        Some(methods.join(","))
    }
}

fn split_handler(handler: &str) -> (Option<String>, &str) {
    if let Some((class, method)) = handler.rsplit_once('.') {
        (Some(class.to_string()), method)
    } else {
        (None, handler)
    }
}

fn has_methods_suffix(src: &str, offset: usize) -> bool {
    src[offset..]
        .lines()
        .next()
        .map(|line| line.contains(".Methods"))
        .unwrap_or(false)
}

fn byte_offset_to_line(src: &str, byte_offset: usize) -> u32 {
    src[..byte_offset.min(src.len())].matches('\n').count() as u32 + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_gin_style_routes() {
        let src = r#"
package api

func register(router *gin.Engine) {
    router.GET("/mdm/query", queryMdm)
    router.POST("/asset/create", controller.CreateAsset)
}
"#;
        let caps = parse_go_file(Path::new("api/routes.go"), src);
        assert_eq!(caps.len(), 2);
        assert_eq!(caps[0].lang, Lang::Go);
        assert_eq!(caps[0].http.as_ref().unwrap().method, "GET");
        assert_eq!(caps[0].http.as_ref().unwrap().path, "/mdm/query");
        assert_eq!(caps[0].method, "queryMdm");
        assert_eq!(caps[0].line, 5);
        assert_eq!(caps[1].class.as_deref(), Some("controller"));
        assert_eq!(caps[1].method, "CreateAsset");
    }

    #[test]
    fn parses_net_http_handle_func() {
        let src = r#"
package main

func main() {
    http.HandleFunc("/healthz", healthz)
}
"#;
        let caps = parse_go_file(Path::new("main.go"), src);
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].http.as_ref().unwrap().method, "ANY");
        assert_eq!(caps[0].http.as_ref().unwrap().path, "/healthz");
        assert_eq!(caps[0].method, "healthz");
    }

    #[test]
    fn parses_gorilla_methods_without_duplicate_any_route() {
        let src = r#"
package api

func routes(r *mux.Router) {
    r.HandleFunc("/articles/{id}", getArticle).Methods("GET", "HEAD")
}
"#;
        let caps = parse_go_file(Path::new("api/routes.go"), src);
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].http.as_ref().unwrap().method, "GET,HEAD");
        assert_eq!(caps[0].http.as_ref().unwrap().path, "/articles/{id}");
        assert_eq!(caps[0].method, "getArticle");
    }
}
