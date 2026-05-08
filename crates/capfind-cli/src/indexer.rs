//! Index builder: walks repo, parses files, tokenizes capabilities, writes index.

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use anyhow::Result;
use rayon::prelude::*;

use capfind_core::{Capability, Field, IndexBody, Posting, TermRef};
use capfind_search::{tokenize, tokenize_path, FIELD_WEIGHTS};

/// Build the full index from parsed capabilities.
///
/// Assigns IDs, tokenizes all fields, builds vocab + postings + avgdl.
pub fn build_index(mut caps: Vec<Capability>) -> IndexBody {
    let mut vocab: Vec<String> = Vec::new();
    let mut vocab_map: HashMap<String, u32> = HashMap::new();
    let mut postings: HashMap<u32, Vec<Posting>> = HashMap::new();

    // Assign IDs and tokenize.
    for (i, cap) in caps.iter_mut().enumerate() {
        cap.id = i as u32;
        let mut terms: Vec<TermRef> = Vec::new();

        // Tokenize each field and build term refs.
        let field_tokens: Vec<(Field, Vec<String>)> = vec![
            (Field::HttpPath, cap.http.as_ref().map(|h| tokenize_path(&h.path)).unwrap_or_default()),
            (Field::ClassName, cap.class.as_ref().map(|c| tokenize(c)).unwrap_or_default()),
            (Field::MethodName, tokenize(&cap.method)),
            (Field::AnnotationValue, cap.annotations.iter().flat_map(|a| tokenize(a)).collect()),
            (Field::Doc, cap.doc.as_ref().map(|d| tokenize(d)).unwrap_or_default()),
            (Field::PackageModule, {
                let mut t = tokenize(&cap.package);
                t.extend(tokenize(&cap.module));
                t
            }),
        ];

        for (field, tokens) in &field_tokens {
            // Count term frequency per (term, field).
            let mut tf_map: HashMap<&str, u16> = HashMap::new();
            for tok in tokens {
                *tf_map.entry(tok.as_str()).or_insert(0) += 1;
            }
            for (tok, tf) in tf_map {
                let tid = *vocab_map.entry(tok.to_string()).or_insert_with(|| {
                    let id = vocab.len() as u32;
                    vocab.push(tok.to_string());
                    id
                });
                terms.push(TermRef { term_id: tid, field: *field, tf });
                postings.entry(tid).or_default().push(Posting {
                    cap_id: cap.id,
                    field: *field,
                    tf,
                });
            }
        }

        cap.terms = terms;
    }

    // Compute avgdl (average document length = avg term refs per cap).
    let total_terms: usize = caps.iter().map(|c| c.terms.len()).sum();
    let avgdl = if caps.is_empty() { 1.0 } else { total_terms as f32 / caps.len() as f32 };

    IndexBody {
        capabilities: caps,
        vocab,
        postings,
        file_stats: HashMap::new(),
        avgdl,
    }
}

/// Walk the repo and parse all supported files in parallel.
pub fn scan_and_parse(repo_root: &Path) -> Result<Vec<Capability>> {
    use ignore::WalkBuilder;

    let walker = WalkBuilder::new(repo_root)
        .hidden(true)        // skip hidden dirs
        .git_ignore(true)    // respect .gitignore
        .git_global(false)
        .git_exclude(true)
        .max_filesize(Some(2 * 1024 * 1024)) // 2MB max
        .threads(num_cpus())
        .build_parallel();

    let (tx, rx) = crossbeam_channel::unbounded::<(String, String)>();

    // Spawn walker threads that push (relative_path, content) pairs.
    let root = repo_root.to_path_buf();
    std::thread::spawn(move || {
        walker.run(|| {
            let tx = tx.clone();
            let root = root.clone();
            Box::new(move |entry| {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => return ignore::WalkState::Continue,
                };
                if !entry.file_type().map_or(false, |ft| ft.is_file()) {
                    return ignore::WalkState::Continue;
                }
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if !matches!(ext, "java" | "go" | "proto") {
                    return ignore::WalkState::Continue;
                }
                // Skip common non-source directories.
                let path_str = path.to_string_lossy();
                if path_str.contains("/target/")
                    || path_str.contains("/build/")
                    || path_str.contains("/node_modules/")
                    || path_str.contains("/vendor/")
                    || path_str.contains("/.gradle/")
                    || path_str.contains("/generated/")
                    || path_str.ends_with("Test.java")
                    || path_str.ends_with("Tests.java")
                    || path_str.contains("/src/test/")
                    || path_str.ends_with("_test.go")
                    || path_str.ends_with(".pb.go")
                {
                    return ignore::WalkState::Continue;
                }
                // Read content.
                let content = match std::fs::read_to_string(path) {
                    Ok(c) => c,
                    Err(_) => return ignore::WalkState::Continue, // binary or unreadable
                };
                let rel = path.strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();
                let _ = tx.send((rel, content));
                ignore::WalkState::Continue
            })
        });
    });

    // Parse in parallel using rayon.
    let pairs: Vec<(String, String)> = rx.into_iter().collect();
    let caps: Vec<Capability> = pairs
        .into_par_iter()
        .flat_map(|(rel_path, content)| {
            let path = std::path::Path::new(&rel_path);
            let mut caps = capfind_parsers::parse_file(path, &content);
            // Fill in module from directory structure.
            let module = extract_module(&rel_path);
            let package = extract_package(&content);
            for cap in &mut caps {
                if cap.module.is_empty() {
                    cap.module = module.clone();
                }
                if cap.package.is_empty() {
                    cap.package = package.clone();
                }
            }
            caps
        })
        .collect();

    Ok(caps)
}

/// Extract module name from relative path (first meaningful directory).
fn extract_module(rel_path: &str) -> String {
    // e.g. "review-main-account/review-http-server/.../Foo.java" → "review-main-account/review-http-server"
    let parts: Vec<&str> = rel_path.split('/').collect();
    if parts.len() > 2 {
        // Take first two directory levels as module.
        format!("{}/{}", parts[0], parts[1])
    } else if parts.len() > 1 {
        parts[0].to_string()
    } else {
        String::new()
    }
}

/// Extract Java package declaration.
fn extract_package(content: &str) -> String {
    for line in content.lines().take(10) {
        let trimmed = line.trim();
        if trimmed.starts_with("package ") {
            return trimmed
                .trim_start_matches("package ")
                .trim_end_matches(';')
                .trim()
                .to_string();
        }
    }
    String::new()
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}
