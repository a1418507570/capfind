//! Proto parser — lightweight extraction of `service` / `rpc` capabilities.
//!
//! This is the first v0.2 slice for RPC discovery. It recognizes regular proto
//! service blocks, rpc signatures, and simple `google.api.http` annotations.

use capfind_core::{Capability, HttpInfo, Kind, Lang, RpcInfo};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;

static PACKAGE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?m)^\s*package\s+([A-Za-z_][\w.]*)\s*;"#).unwrap());

static SERVICE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?m)\bservice\s+([A-Za-z_][\w]*)\s*\{"#).unwrap());

static RPC_SIG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?m)\brpc\s+([A-Za-z_][\w]*)\s*\(\s*(?:stream\s+)?([.A-Za-z_][\w.]*)\s*\)\s*returns\s*\(\s*(?:stream\s+)?([.A-Za-z_][\w.]*)\s*\)"#,
    )
    .unwrap()
});

static HTTP_ANNOTATION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?s)\b(get|post|put|delete|patch)\s*:\s*"([^"]+)""#).unwrap());

/// Parse a single `.proto` file and return all detected RPC capabilities.
pub fn parse_proto_file(path: &Path, src: &str) -> Vec<Capability> {
    let file = path.to_string_lossy().to_string();
    let package = PACKAGE_RE
        .captures(src)
        .and_then(|captures| captures.get(1))
        .map(|package| package.as_str().to_string())
        .unwrap_or_default();
    let mut caps = Vec::new();

    for service in SERVICE_RE.captures_iter(src) {
        let whole = service.get(0).unwrap();
        let service_name = service.get(1).unwrap().as_str();
        let service_open = whole.end() - 1;
        let Some(service_close) = find_matching_brace(src, service_open) else {
            continue;
        };
        let service_body_start = service_open + 1;
        let service_body = &src[service_body_start..service_close];

        for rpc in RPC_SIG_RE.captures_iter(service_body) {
            let rpc_whole = rpc.get(0).unwrap();
            let rpc_name = rpc.get(1).unwrap().as_str();
            let req = rpc.get(2).unwrap().as_str();
            let rsp = rpc.get(3).unwrap().as_str();
            let global_start = service_body_start + rpc_whole.start();
            let (global_end, rpc_block) =
                rpc_body(src, service_body, service_body_start, rpc_whole.end());
            let http = rpc_block.and_then(extract_http_annotation);
            let signature = format!("rpc {rpc_name}({req}) returns ({rsp})");
            let mut annotations = vec![signature.clone()];
            if let Some(ref http) = http {
                annotations.push(format!("{} {}", http.method, http.path));
            }

            caps.push(Capability {
                id: 0,
                kind: Kind::RpcMethod,
                lang: Lang::Proto,
                module: String::new(),
                package: package.clone(),
                class: Some(service_name.to_string()),
                method: rpc_name.to_string(),
                signature,
                annotations,
                http,
                rpc: Some(RpcInfo {
                    service: service_name.to_string(),
                    rpc: rpc_name.to_string(),
                    req: req.to_string(),
                    rsp: rsp.to_string(),
                    proto_file: Some(file.clone()),
                }),
                doc: None,
                tags: vec![],
                file: file.clone(),
                line: byte_offset_to_line(src, global_start),
                byte_range: (global_start as u32, global_end as u32),
                terms: vec![],
            });
        }
    }

    caps.sort_by_key(|cap| cap.byte_range.0);
    caps
}

fn rpc_body<'a>(
    src: &'a str,
    service_body: &'a str,
    service_body_start: usize,
    rpc_sig_end: usize,
) -> (usize, Option<&'a str>) {
    let tail = &service_body[rpc_sig_end..];
    let trimmed = tail.trim_start();
    let whitespace = tail.len() - trimmed.len();
    let body_start = service_body_start + rpc_sig_end + whitespace;

    if trimmed.starts_with('{') {
        if let Some(body_end) = find_matching_brace(src, body_start) {
            return (body_end + 1, Some(&src[body_start..=body_end]));
        }
    }

    if trimmed.starts_with(';') {
        return (body_start + 1, None);
    }

    (service_body_start + rpc_sig_end, None)
}

fn extract_http_annotation(block: &str) -> Option<HttpInfo> {
    let captures = HTTP_ANNOTATION_RE.captures(block)?;
    Some(HttpInfo {
        method: captures.get(1).unwrap().as_str().to_ascii_uppercase(),
        path: captures.get(2).unwrap().as_str().to_string(),
        consumes: None,
        produces: None,
    })
}

fn find_matching_brace(src: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in src[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn byte_offset_to_line(src: &str, byte_offset: usize) -> u32 {
    src[..byte_offset.min(src.len())].matches('\n').count() as u32 + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_rpc_methods_and_http_annotation() {
        let src = r#"
syntax = "proto3";
package demo.mdm.v1;

service MdmService {
  rpc QueryMdm(QueryMdmRequest) returns (QueryMdmResponse) {
    option (google.api.http) = {
      post: "/v1/mdm/query"
      body: "*"
    };
  }

  rpc GetMdm(GetMdmRequest) returns (GetMdmResponse);
}
"#;
        let caps = parse_proto_file(Path::new("proto/mdm.proto"), src);
        assert_eq!(caps.len(), 2);
        assert_eq!(caps[0].kind, Kind::RpcMethod);
        assert_eq!(caps[0].lang, Lang::Proto);
        assert_eq!(caps[0].package, "demo.mdm.v1");
        assert_eq!(caps[0].class.as_deref(), Some("MdmService"));
        assert_eq!(caps[0].method, "QueryMdm");
        assert_eq!(caps[0].rpc.as_ref().unwrap().req, "QueryMdmRequest");
        assert_eq!(caps[0].rpc.as_ref().unwrap().rsp, "QueryMdmResponse");
        assert_eq!(caps[0].http.as_ref().unwrap().method, "POST");
        assert_eq!(caps[0].http.as_ref().unwrap().path, "/v1/mdm/query");
        assert_eq!(caps[0].line, 6);
        assert_eq!(caps[1].method, "GetMdm");
        assert!(caps[1].http.is_none());
    }

    #[test]
    fn parses_streaming_rpc_types() {
        let src = r#"
syntax = "proto3";

service EventService {
  rpc Watch(stream WatchRequest) returns (stream WatchResponse);
}
"#;
        let caps = parse_proto_file(Path::new("event.proto"), src);
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].method, "Watch");
        assert_eq!(caps[0].rpc.as_ref().unwrap().req, "WatchRequest");
        assert_eq!(caps[0].rpc.as_ref().unwrap().rsp, "WatchResponse");
    }
}
