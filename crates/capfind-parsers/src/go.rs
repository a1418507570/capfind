//! Go parser — regex-based extraction of common HTTP route registrations.
//!
//! # Coverage
//!
//! This first slice intentionally focuses on route registration calls that are
//! common in Gin, Echo, Hertz, net/http, and gorilla/mux style code:
//!
//! - `router.GET("/path", handler)` / `router.POST(...)`
//! - `v1 := router.Group("/v1")` + `v1.GET("/path", handler)`
//! - `r.Get("/path", handler)` / `r.Post(...)` for chi-style routers
//! - `http.HandleFunc("/path", handler)`
//! - `mux.HandleFunc("/path", handler).Methods("GET")`
//!
//! It does not try to be a full Go AST parser yet. That remains a v0.2 track.

use capfind_core::{Capability, HttpInfo, Kind, Lang};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;

static ROUTE_CALL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?m)\b(?:(?P<recv>[A-Za-z_][\w]*)\.)?(?P<method>GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS|Get|Post|Put|Delete|Patch|Head|Options|Any|ANY|HandleFunc)\s*\(\s*"(?P<path>[^"]+)"\s*,\s*(?P<handler>[A-Za-z_][\w.]*)"#,
    )
    .unwrap()
});

static GORILLA_METHODS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?s)\b(?:(?P<recv>[A-Za-z_][\w]*)\.)?HandleFunc\s*\(\s*"(?P<path>[^"]+)"\s*,\s*(?P<handler>[A-Za-z_][\w.]*)\s*\)\s*\.Methods\s*\((?P<methods>[^)]*)\)"#,
    )
    .unwrap()
});

static GROUP_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?m)\b(?P<var>[A-Za-z_][\w]*)\s*:=\s*(?P<parent>[A-Za-z_][\w]*)\.Group\s*\(\s*"(?P<prefix>[^"]+)"\s*\)"#,
    )
    .unwrap()
});

static STRING_LITERAL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#""([^"]+)""#).unwrap());

/// Parse a single `.go` file and return all detected capabilities.
pub fn parse_go_file(path: &Path, src: &str) -> Vec<Capability> {
    let file = path.to_string_lossy().to_string();
    let groups = collect_group_prefixes(src);
    let mut caps = Vec::new();

    for captures in GORILLA_METHODS_RE.captures_iter(src) {
        let whole = captures.get(0).unwrap();
        let receiver = captures.name("recv").map(|m| m.as_str());
        let path = captures.name("path").unwrap().as_str();
        let handler = captures.name("handler").unwrap().as_str();
        let methods = captures.name("methods").unwrap().as_str();
        let http_method = extract_methods(methods).unwrap_or_else(|| "ANY".to_string());
        let full_path = qualify_route_path(receiver, path, &groups);
        caps.push(route_capability(
            &file,
            src,
            whole.start(),
            whole.end(),
            whole.as_str(),
            &http_method,
            &full_path,
            handler,
        ));
    }

    for captures in ROUTE_CALL_RE.captures_iter(src) {
        let whole = captures.get(0).unwrap();
        let route_kind = captures.name("method").unwrap().as_str();
        if route_kind == "HandleFunc" && has_methods_suffix(src, whole.end()) {
            continue;
        }

        let receiver = captures.name("recv").map(|m| m.as_str());
        let path = captures.name("path").unwrap().as_str();
        let handler = captures.name("handler").unwrap().as_str();
        let http_method = normalize_method(route_kind);
        let full_path = qualify_route_path(receiver, path, &groups);
        caps.push(route_capability(
            &file,
            src,
            whole.start(),
            whole.end(),
            whole.as_str(),
            &http_method,
            &full_path,
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

fn collect_group_prefixes(src: &str) -> HashMap<String, String> {
    let mut groups: HashMap<String, String> = HashMap::new();
    for captures in GROUP_RE.captures_iter(src) {
        let var = captures.name("var").unwrap().as_str();
        let parent = captures.name("parent").unwrap().as_str();
        let prefix = captures.name("prefix").unwrap().as_str();
        let full_prefix = if let Some(parent_prefix) = groups.get(parent) {
            join_paths(parent_prefix, prefix)
        } else {
            normalize_path(prefix)
        };
        groups.insert(var.to_string(), full_prefix);
    }
    groups
}

fn qualify_route_path(
    receiver: Option<&str>,
    path: &str,
    groups: &HashMap<String, String>,
) -> String {
    receiver
        .and_then(|name| groups.get(name))
        .map(|prefix| join_paths(prefix, path))
        .unwrap_or_else(|| normalize_path(path))
}

fn join_paths(prefix: &str, path: &str) -> String {
    let prefix = normalize_path(prefix);
    let path = normalize_path(path);
    if prefix == "/" {
        return path;
    }
    if path == "/" {
        return prefix;
    }
    format!(
        "{}/{}",
        prefix.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn normalize_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
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
    let tail = &src[offset.min(src.len())..src.len().min(offset + 128)];
    let tail = tail.trim_start();
    if tail.starts_with(".Methods") {
        return true;
    }
    tail.strip_prefix(')')
        .map(|rest| rest.trim_start().starts_with(".Methods"))
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

    #[test]
    fn parses_group_prefixes_and_chi_style_routes() {
        let src = r#"
package api

func routes(router *gin.Engine, r chi.Router) {
    v1 := router.Group("/v1")
    admin := v1.Group("admin")
    v1.GET("/mdm/query", queryMdm)
    admin.POST("/assets", controller.CreateAsset)
    r.Get("/chi/articles/{id}", getArticle)
}
"#;
        let caps = parse_go_file(Path::new("api/routes.go"), src);
        assert_eq!(caps.len(), 3);
        assert_eq!(caps[0].http.as_ref().unwrap().method, "GET");
        assert_eq!(caps[0].http.as_ref().unwrap().path, "/v1/mdm/query");
        assert_eq!(caps[1].http.as_ref().unwrap().method, "POST");
        assert_eq!(caps[1].http.as_ref().unwrap().path, "/v1/admin/assets");
        assert_eq!(caps[1].class.as_deref(), Some("controller"));
        assert_eq!(caps[2].http.as_ref().unwrap().method, "GET");
        assert_eq!(caps[2].http.as_ref().unwrap().path, "/chi/articles/{id}");
        assert_eq!(caps[2].method, "getArticle");
    }
}
