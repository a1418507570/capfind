//! capfind-parsers: language-specific parsers that produce `Vec<Capability>`.
//!
//! This crate is organized as sub-modules per language. Only the `java` module
//! is active in v0.1; `go` and `proto` land in v0.2.

#![forbid(unsafe_code)]

pub mod java;
// pub mod go;   // v0.2
// pub mod proto; // v0.2

use capfind_core::{Capability, Lang};
use std::path::Path;

/// Dispatch to the appropriate parser based on file extension.
pub fn parse_file(path: &Path, src: &str) -> Vec<Capability> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("java") => java::parse_java_file(path, src),
        // Some("go")   => go::parse_go_file(path, src),
        // Some("proto") => proto::parse_proto_file(path, src),
        _ => vec![],
    }
}

/// Which languages this build supports.
pub fn supported_langs() -> &'static [Lang] {
    &[Lang::Java]
}
