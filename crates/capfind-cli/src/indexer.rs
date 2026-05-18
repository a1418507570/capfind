//! Index builder: walks repo, parses files, tokenizes capabilities, writes index.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use anyhow::Result;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use rayon::prelude::*;

use capfind_core::{
    load_index, Capability, Field, FileStat, IndexBody, Kind, Lang, Posting, TermRef,
};
use capfind_search::{tokenize, tokenize_path};

/// Build the full index from parsed capabilities.
///
/// Assigns IDs, tokenizes all fields, builds vocab + postings + avgdl.
pub fn build_index(caps: Vec<Capability>) -> IndexBody {
    build_index_with_file_stats(caps, HashMap::new())
}

pub fn build_index_with_file_stats(
    mut caps: Vec<Capability>,
    file_stats: HashMap<String, FileStat>,
) -> IndexBody {
    let mut vocab: Vec<String> = Vec::new();
    let mut vocab_map: HashMap<String, u32> = HashMap::new();
    let mut postings: HashMap<u32, Vec<Posting>> = HashMap::new();

    // Assign IDs and tokenize.
    for (i, cap) in caps.iter_mut().enumerate() {
        cap.id = i as u32;
        let mut terms: Vec<TermRef> = Vec::new();

        // Tokenize each field and build term refs.
        let field_tokens: Vec<(Field, Vec<String>)> = vec![
            (
                Field::HttpPath,
                cap.http
                    .as_ref()
                    .map(|h| tokenize_path(&h.path))
                    .unwrap_or_default(),
            ),
            (
                Field::ClassName,
                cap.class.as_ref().map(|c| tokenize(c)).unwrap_or_default(),
            ),
            (Field::MethodName, tokenize(&cap.method)),
            (Field::AnnotationValue, {
                let mut t = cap
                    .annotations
                    .iter()
                    .flat_map(|a| tokenize(a))
                    .collect::<Vec<_>>();
                t.extend(cap.tags.iter().flat_map(|tag| tokenize(tag)));
                t
            }),
            (
                Field::Doc,
                cap.doc.as_ref().map(|d| tokenize(d)).unwrap_or_default(),
            ),
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
                terms.push(TermRef {
                    term_id: tid,
                    field: *field,
                    tf,
                });
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
    let avgdl = if caps.is_empty() {
        1.0
    } else {
        total_terms as f32 / caps.len() as f32
    };

    IndexBody {
        capabilities: caps,
        vocab,
        postings,
        file_stats,
        avgdl,
    }
}

pub struct IndexBuildResult {
    pub body: IndexBody,
    pub parsed_files: usize,
    pub reused_files: usize,
}

#[derive(Clone)]
struct SourceUnit {
    rel_path: String,
    content: String,
    stat: FileStat,
}

/// Walk the repo and parse all supported files in parallel.
pub fn scan_and_parse(repo_root: &Path) -> Result<Vec<Capability>> {
    let units = scan_source_units(repo_root)?;
    Ok(parse_units(&units)
        .into_iter()
        .chain(scan_java_external_apis_from_units(&units))
        .collect())
}

pub fn index_repo(repo_root: &Path, rehash: bool) -> Result<IndexBuildResult> {
    let units = scan_source_units(repo_root)?;
    let file_stats = units
        .iter()
        .map(|unit| (unit.rel_path.clone(), unit.stat.clone()))
        .collect::<HashMap<_, _>>();
    let old_body = if rehash {
        None
    } else {
        load_index(&repo_root.join(".capfind/index.cfi"))
            .ok()
            .map(|(_, body)| body)
    };

    let unchanged = old_body
        .as_ref()
        .map(|body| {
            file_stats
                .iter()
                .filter_map(|(path, stat)| {
                    (body.file_stats.get(path) == Some(stat)).then_some(path.clone())
                })
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();

    let mut caps = old_body
        .map(|body| {
            body.capabilities
                .into_iter()
                .filter(|cap| unchanged.contains(&cap.file) && !is_external_capability(cap))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let changed_units = units
        .iter()
        .filter(|unit| !unchanged.contains(&unit.rel_path))
        .cloned()
        .collect::<Vec<_>>();
    let parsed_files = changed_units.len();
    let reused_files = unchanged.len();

    caps.extend(parse_units(&changed_units));
    caps.extend(scan_java_external_apis_from_units(&units));

    Ok(IndexBuildResult {
        body: build_index_with_file_stats(caps, file_stats),
        parsed_files,
        reused_files,
    })
}

fn scan_source_units(repo_root: &Path) -> Result<Vec<SourceUnit>> {
    use ignore::WalkBuilder;

    let capfind_ignore = Arc::new(load_capfindignore(repo_root)?);
    let walker = WalkBuilder::new(repo_root)
        .hidden(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .threads(num_cpus())
        .build_parallel();

    let (tx, rx) = crossbeam_channel::unbounded::<SourceUnit>();
    let root = repo_root.to_path_buf();
    std::thread::spawn(move || {
        walker.run(|| {
            let tx = tx.clone();
            let root = root.clone();
            let capfind_ignore = Arc::clone(&capfind_ignore);
            Box::new(move |entry| {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => return ignore::WalkState::Continue,
                };
                if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                    return ignore::WalkState::Continue;
                }
                let path = entry.path();
                if is_capfind_ignored(capfind_ignore.as_ref().as_ref(), path, false)
                    || !is_supported_index_input(path)
                    || is_skipped_source_path(path)
                {
                    return ignore::WalkState::Continue;
                }

                let is_local_jar = path.extension().and_then(|e| e.to_str()) == Some("jar");
                if !is_local_jar
                    && std::fs::metadata(path)
                        .map(|metadata| metadata.len() > 2 * 1024 * 1024)
                        .unwrap_or(false)
                {
                    return ignore::WalkState::Continue;
                }
                let content = if is_local_jar {
                    read_jar_class_entries(path).join("\n")
                } else {
                    match std::fs::read_to_string(path) {
                        Ok(c) => c,
                        Err(_) => return ignore::WalkState::Continue,
                    }
                };
                let rel_path = path
                    .strip_prefix(&root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();
                let Some(stat) = file_stat(path, &content) else {
                    return ignore::WalkState::Continue;
                };
                let _ = tx.send(SourceUnit {
                    rel_path,
                    content,
                    stat,
                });
                ignore::WalkState::Continue
            })
        });
    });

    Ok(rx.into_iter().collect())
}

fn is_supported_index_input(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    matches!(ext, "java" | "go" | "proto" | "jar")
        || matches!(file_name, "pom.xml" | "build.gradle" | "build.gradle.kts")
}

fn is_skipped_source_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains("/target/")
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
}

fn file_stat(path: &Path, content: &str) -> Option<FileStat> {
    let metadata = std::fs::metadata(path).ok()?;
    let mtime_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or_default();
    Some(FileStat {
        mtime_ns,
        size: metadata.len(),
        blake3: *blake3::hash(content.as_bytes()).as_bytes(),
    })
}

fn parse_units(units: &[SourceUnit]) -> Vec<Capability> {
    units
        .par_iter()
        .flat_map(|unit| {
            let path = std::path::Path::new(&unit.rel_path);
            let mut caps = capfind_parsers::parse_file(path, &unit.content);
            let module = extract_module(&unit.rel_path);
            let package = extract_package(&unit.content);
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
        .collect()
}

fn scan_java_external_apis_from_units(units: &[SourceUnit]) -> Vec<Capability> {
    let pairs = units
        .iter()
        .map(|unit| (unit.rel_path.clone(), unit.content.clone()))
        .collect::<Vec<_>>();
    scan_java_external_apis(&pairs)
}

fn is_external_capability(cap: &Capability) -> bool {
    cap.tags.iter().any(|tag| tag == "external")
}

fn scan_java_external_apis(pairs: &[(String, String)]) -> Vec<Capability> {
    let project_packages = collect_project_packages(pairs);
    let mut seen = HashSet::new();
    let mut caps = Vec::new();

    for (rel_path, content) in pairs {
        let file_name = Path::new(rel_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if rel_path.ends_with(".java") {
            for cap in scan_external_imports(rel_path, content, &project_packages) {
                if seen.insert(cap.signature.clone()) {
                    caps.push(cap);
                }
            }
        } else if file_name == "pom.xml" {
            for cap in scan_maven_dependencies(rel_path, content) {
                if seen.insert(cap.signature.clone()) {
                    caps.push(cap);
                }
            }
        } else if file_name == "build.gradle" || file_name == "build.gradle.kts" {
            for cap in scan_gradle_dependencies(rel_path, content) {
                if seen.insert(cap.signature.clone()) {
                    caps.push(cap);
                }
            }
        } else if rel_path.ends_with(".jar") {
            let cap = local_jar_capability(rel_path);
            if seen.insert(cap.signature.clone()) {
                caps.push(cap);
            }
            for cap in scan_jar_class_capabilities(rel_path, content) {
                if seen.insert(cap.signature.clone()) {
                    caps.push(cap);
                }
            }
        }
    }

    caps.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.method.cmp(&b.method))
    });
    caps
}

fn collect_project_packages(pairs: &[(String, String)]) -> Vec<String> {
    let mut packages = pairs
        .iter()
        .filter(|(path, _)| path.ends_with(".java"))
        .filter_map(|(_, content)| {
            let package = extract_package(content);
            (!package.is_empty()).then_some(package)
        })
        .collect::<Vec<_>>();
    packages.sort();
    packages.dedup();
    packages
}

fn scan_external_imports(
    rel_path: &str,
    content: &str,
    project_packages: &[String],
) -> Vec<Capability> {
    let mut caps = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with("import ") {
            continue;
        }
        let symbol = trimmed
            .trim_start_matches("import ")
            .trim_start_matches("static ")
            .trim_end_matches(';')
            .trim();
        if !is_external_import(symbol, project_packages) {
            continue;
        }
        let (package, class) = split_external_symbol(symbol);
        let method = class.clone().unwrap_or_else(|| package.clone());
        caps.push(external_capability(
            rel_path,
            idx as u32 + 1,
            package,
            class,
            method,
            format!("import {symbol}"),
            vec!["external_api".to_string(), "java_import".to_string()],
            "External Java API imported by project source".to_string(),
        ));
    }
    caps
}

fn is_external_import(symbol: &str, project_packages: &[String]) -> bool {
    if symbol.starts_with("java.") {
        return false;
    }
    !project_packages
        .iter()
        .any(|package| symbol == package || symbol.starts_with(&format!("{package}.")))
}

fn split_external_symbol(symbol: &str) -> (String, Option<String>) {
    let symbol = symbol.trim_end_matches(".*");
    if let Some((package, class)) = symbol.rsplit_once('.') {
        (package.to_string(), Some(class.to_string()))
    } else {
        (symbol.to_string(), None)
    }
}

fn scan_maven_dependencies(rel_path: &str, content: &str) -> Vec<Capability> {
    let mut caps = Vec::new();
    let mut search_from = 0usize;
    while let Some(start_rel) = content[search_from..].find("<dependency>") {
        let start = search_from + start_rel;
        let Some(end_rel) = content[start..].find("</dependency>") else {
            break;
        };
        let end = start + end_rel + "</dependency>".len();
        let block = &content[start..end];
        search_from = end;

        let group = tag_value(block, "groupId").unwrap_or_default();
        let artifact = tag_value(block, "artifactId").unwrap_or_default();
        let version = tag_value(block, "version").unwrap_or_default();
        let scope = tag_value(block, "scope").unwrap_or_default();
        if group.is_empty() || artifact.is_empty() || scope == "test" {
            continue;
        }
        let line = byte_offset_to_line(content, start);
        caps.push(dependency_capability(
            rel_path, line, "maven", &group, &artifact, &version,
        ));
    }
    caps
}

fn tag_value(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&close)? + start;
    Some(block[start..end].trim().to_string())
}

fn scan_gradle_dependencies(rel_path: &str, content: &str) -> Vec<Capability> {
    let mut caps = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let Some(config) = gradle_config(trimmed) else {
            continue;
        };
        let Some(coord) = first_quoted_value(trimmed) else {
            continue;
        };
        let parts = coord.split(':').collect::<Vec<_>>();
        if parts.len() < 2 {
            continue;
        }
        let version = parts.get(2).copied().unwrap_or_default();
        caps.push(dependency_capability(
            rel_path,
            idx as u32 + 1,
            config,
            parts[0],
            parts[1],
            version,
        ));
    }
    caps
}

fn gradle_config(line: &str) -> Option<&'static str> {
    [
        "implementation",
        "api",
        "compileOnly",
        "runtimeOnly",
        "compile",
        "providedCompile",
        "annotationProcessor",
    ]
    .into_iter()
    .find(|config| line.starts_with(*config))
}

fn first_quoted_value(line: &str) -> Option<&str> {
    let start = line.find(|ch| ch == '\'' || ch == '"')?;
    let quote = line.as_bytes()[start] as char;
    let rest = &line[start + 1..];
    let end = rest.find(quote)?;
    Some(&rest[..end])
}

fn dependency_capability(
    rel_path: &str,
    line: u32,
    source: &str,
    group: &str,
    artifact: &str,
    version: &str,
) -> Capability {
    let coordinate = if version.is_empty() {
        format!("{group}:{artifact}")
    } else {
        format!("{group}:{artifact}:{version}")
    };
    external_capability(
        rel_path,
        line,
        group.to_string(),
        Some(artifact.to_string()),
        artifact.to_string(),
        format!("{source} dependency {coordinate}"),
        vec!["external_dependency".to_string(), source.to_string()],
        format!("External jar dependency declared by {source}: {coordinate}"),
    )
}

fn local_jar_capability(rel_path: &str) -> Capability {
    let file_name = Path::new(rel_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(rel_path);
    let artifact = file_name.trim_end_matches(".jar");
    external_capability(
        rel_path,
        1,
        "local.jar".to_string(),
        Some(artifact.to_string()),
        artifact.to_string(),
        format!("local jar {file_name}"),
        vec!["external_dependency".to_string(), "local_jar".to_string()],
        format!("External local jar discovered in repository: {file_name}"),
    )
}

fn scan_jar_class_capabilities(rel_path: &str, content: &str) -> Vec<Capability> {
    content
        .lines()
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| {
            if let Some(method) = JarMethodEntry::parse(entry) {
                jar_method_capability(rel_path, method)
            } else if let Some(method) = SourceMethodEntry::parse(entry) {
                source_method_capability(rel_path, method)
            } else if let Some(method) = JavadocMethodEntry::parse(entry) {
                javadoc_method_capability(rel_path, method)
            } else {
                jar_class_capability(rel_path, entry)
            }
        })
        .collect()
}

fn jar_class_capability(rel_path: &str, class_name: &str) -> Capability {
    let (package, class) = split_external_symbol(class_name);
    let method = class.clone().unwrap_or_else(|| package.clone());
    external_capability(
        rel_path,
        1,
        package,
        class,
        method,
        format!("jar class {class_name}"),
        vec![
            "external_api".to_string(),
            "jar_class".to_string(),
            "local_jar".to_string(),
        ],
        format!("External API class discovered inside local jar: {class_name}"),
    )
}

fn jar_method_capability(rel_path: &str, method: JarMethodEntry<'_>) -> Capability {
    let (package, class) = split_external_symbol(method.class_name);
    let params = method.params.join(", ");
    let signature = format!(
        "jar method {}#{}({}): {}",
        method.class_name, method.method_name, params, method.return_type
    );
    let mut annotations = vec![
        "external_api".to_string(),
        "jar_method".to_string(),
        format!("{}#{}", method.class_name, method.method_name),
    ];
    if method.is_static {
        annotations.push("static".to_string());
    }
    external_capability(
        rel_path,
        1,
        package,
        class,
        method.method_name.to_string(),
        signature.clone(),
        annotations,
        format!("External API method discovered inside local jar: {signature}"),
    )
}

fn source_method_capability(rel_path: &str, method: SourceMethodEntry<'_>) -> Capability {
    documented_method_capability(
        rel_path,
        method.class_name,
        method.method_name,
        &method.params,
        method.return_type,
        method.doc,
        "source method",
        "source_method",
        "sources_jar",
        "sources jar",
    )
}

fn javadoc_method_capability(rel_path: &str, method: JavadocMethodEntry<'_>) -> Capability {
    documented_method_capability(
        rel_path,
        method.class_name,
        method.method_name,
        &method.params,
        method.return_type,
        method.doc,
        "javadoc method",
        "javadoc_method",
        "javadoc_jar",
        "javadoc jar",
    )
}

#[allow(clippy::too_many_arguments)]
fn documented_method_capability(
    rel_path: &str,
    class_name: &str,
    method_name: &str,
    params: &[&str],
    return_type: &str,
    method_doc: &str,
    signature_prefix: &str,
    method_tag: &str,
    jar_tag: &str,
    jar_label: &str,
) -> Capability {
    let (package, class) = split_external_symbol(class_name);
    let params_display = params.join(", ");
    let signature =
        format!("{signature_prefix} {class_name}#{method_name}({params_display}): {return_type}");
    let annotations = vec![
        "external_api".to_string(),
        method_tag.to_string(),
        jar_tag.to_string(),
        format!("{class_name}#{method_name}"),
    ];
    let doc = if method_doc.is_empty() {
        format!("External API method discovered inside {jar_label}: {signature}")
    } else {
        format!(
            "External API method discovered inside {jar_label}: {signature}. Documentation: {method_doc}"
        )
    };
    external_capability(
        rel_path,
        1,
        package,
        class,
        method_name.to_string(),
        signature,
        annotations,
        doc,
    )
}

struct JarMethodEntry<'a> {
    class_name: &'a str,
    method_name: &'a str,
    params: Vec<&'a str>,
    return_type: &'a str,
    is_static: bool,
}

impl<'a> JarMethodEntry<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let mut parts = line.split('\t');
        if parts.next()? != "method" {
            return None;
        }
        let class_name = parts.next()?;
        let method_name = parts.next()?;
        let params_raw = parts.next()?;
        let return_type = parts.next()?;
        let is_static = parts.next() == Some("static");
        let params = if params_raw.is_empty() {
            Vec::new()
        } else {
            params_raw.split(',').collect()
        };
        Some(Self {
            class_name,
            method_name,
            params,
            return_type,
            is_static,
        })
    }
}

struct SourceMethodEntry<'a> {
    class_name: &'a str,
    method_name: &'a str,
    params: Vec<&'a str>,
    return_type: &'a str,
    doc: &'a str,
}

impl<'a> SourceMethodEntry<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        parse_documented_method_entry(line, "source_method").map(|entry| Self {
            class_name: entry.class_name,
            method_name: entry.method_name,
            params: entry.params,
            return_type: entry.return_type,
            doc: entry.doc,
        })
    }
}

struct JavadocMethodEntry<'a> {
    class_name: &'a str,
    method_name: &'a str,
    params: Vec<&'a str>,
    return_type: &'a str,
    doc: &'a str,
}

impl<'a> JavadocMethodEntry<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        parse_documented_method_entry(line, "javadoc_method").map(|entry| Self {
            class_name: entry.class_name,
            method_name: entry.method_name,
            params: entry.params,
            return_type: entry.return_type,
            doc: entry.doc,
        })
    }
}

struct DocumentedMethodEntry<'a> {
    class_name: &'a str,
    method_name: &'a str,
    params: Vec<&'a str>,
    return_type: &'a str,
    doc: &'a str,
}

fn parse_documented_method_entry<'a>(
    line: &'a str,
    prefix: &str,
) -> Option<DocumentedMethodEntry<'a>> {
    let mut parts = line.split('\t');
    if parts.next()? != prefix {
        return None;
    }
    let class_name = parts.next()?;
    let method_name = parts.next()?;
    let params_raw = parts.next()?;
    let return_type = parts.next()?;
    let doc = parts.next().unwrap_or_default();
    let params = if params_raw.is_empty() {
        Vec::new()
    } else {
        params_raw.split(',').collect()
    };
    Some(DocumentedMethodEntry {
        class_name,
        method_name,
        params,
        return_type,
        doc,
    })
}

fn read_jar_class_entries(path: &Path) -> Vec<String> {
    const MAX_JAR_BYTES: u64 = 64 * 1024 * 1024;
    if std::fs::metadata(path)
        .map(|metadata| metadata.len() > MAX_JAR_BYTES)
        .unwrap_or(true)
    {
        return Vec::new();
    }

    let entries = read_jar_class_entries_with_zip(path);
    if !entries.is_empty() {
        return entries;
    }

    std::fs::read(path)
        .map(|bytes| jar_class_entries_from_bytes(&bytes))
        .unwrap_or_default()
}

fn read_jar_class_entries_with_zip(path: &Path) -> Vec<String> {
    const JAR_CLASS_ENTRY_LIMIT: usize = 2_000;
    const MAX_CLASS_BYTES: u64 = 8 * 1024 * 1024;

    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return Vec::new();
    };

    let is_sources_jar = is_sources_jar(path);
    let is_javadoc_jar = is_javadoc_jar(path);
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for idx in 0..archive.len() {
        if entries.len() >= JAR_CLASS_ENTRY_LIMIT {
            break;
        }
        let Ok(mut file) = archive.by_index(idx) else {
            continue;
        };
        let entry_name = file.name().to_string();
        if is_sources_jar && entry_name.ends_with(".java") {
            if file.size() > MAX_CLASS_BYTES {
                continue;
            }
            let mut src = String::new();
            if file.read_to_string(&mut src).is_err() {
                continue;
            }
            for method in source_methods_from_java_source(&entry_name, &src) {
                if seen.insert(format!("source:{method}")) {
                    entries.push(method);
                }
            }
            continue;
        }
        if is_javadoc_jar && entry_name.ends_with(".html") {
            if file.size() > MAX_CLASS_BYTES {
                continue;
            }
            let mut html = String::new();
            if file.read_to_string(&mut html).is_err() {
                continue;
            }
            for method in javadoc_methods_from_html(&entry_name, &html) {
                if seen.insert(format!("javadoc:{method}")) {
                    entries.push(method);
                }
            }
            continue;
        }

        let Some(class_name) = class_name_from_jar_entry(&entry_name) else {
            continue;
        };
        if seen.insert(format!("class:{class_name}")) {
            entries.push(class_name.clone());
        }
        if file.size() > MAX_CLASS_BYTES {
            continue;
        }
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            continue;
        }
        for method in jar_methods_from_class_bytes(&class_name, &bytes) {
            if seen.insert(format!("method:{method}")) {
                entries.push(method);
            }
        }
    }
    entries
}

fn is_sources_jar(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.contains("-sources") || name.ends_with("sources.jar"))
        .unwrap_or(false)
}

fn is_javadoc_jar(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.contains("-javadoc") || name.ends_with("javadoc.jar"))
        .unwrap_or(false)
}

fn jar_class_entries_from_bytes(bytes: &[u8]) -> Vec<String> {
    const CENTRAL_DIR_SIG: &[u8; 4] = b"PK\x01\x02";
    const CENTRAL_DIR_HEADER_LEN: usize = 46;
    const JAR_CLASS_ENTRY_LIMIT: usize = 2_000;

    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut offset = 0usize;

    while offset + CENTRAL_DIR_HEADER_LEN <= bytes.len() && entries.len() < JAR_CLASS_ENTRY_LIMIT {
        if &bytes[offset..offset + 4] != CENTRAL_DIR_SIG {
            offset += 1;
            continue;
        }

        let compression = u16::from_le_bytes([bytes[offset + 10], bytes[offset + 11]]);
        let compressed_size = u32::from_le_bytes([
            bytes[offset + 20],
            bytes[offset + 21],
            bytes[offset + 22],
            bytes[offset + 23],
        ]) as usize;
        let name_len = u16::from_le_bytes([bytes[offset + 28], bytes[offset + 29]]) as usize;
        let extra_len = u16::from_le_bytes([bytes[offset + 30], bytes[offset + 31]]) as usize;
        let comment_len = u16::from_le_bytes([bytes[offset + 32], bytes[offset + 33]]) as usize;
        let local_offset = u32::from_le_bytes([
            bytes[offset + 42],
            bytes[offset + 43],
            bytes[offset + 44],
            bytes[offset + 45],
        ]) as usize;
        let name_start = offset + CENTRAL_DIR_HEADER_LEN;
        let name_end = name_start.saturating_add(name_len);
        if name_end > bytes.len() {
            offset += 4;
            continue;
        }

        if let Ok(entry_name) = std::str::from_utf8(&bytes[name_start..name_end]) {
            if entry_name.ends_with(".java") || entry_name.ends_with(".html") {
                if compression == 0 {
                    if let Some(entry_bytes) =
                        stored_zip_entry(bytes, local_offset, compressed_size)
                    {
                        if let Ok(text) = std::str::from_utf8(entry_bytes) {
                            let methods = if entry_name.ends_with(".java") {
                                source_methods_from_java_source(entry_name, text)
                            } else {
                                javadoc_methods_from_html(entry_name, text)
                            };
                            for method in methods {
                                if seen.insert(format!("doc:{method}")) {
                                    entries.push(method);
                                }
                            }
                        }
                    }
                }
                offset = name_end
                    .saturating_add(extra_len)
                    .saturating_add(comment_len)
                    .min(bytes.len());
                continue;
            }
            if let Some(class_name) = class_name_from_jar_entry(entry_name) {
                if seen.insert(format!("class:{class_name}")) {
                    entries.push(class_name.clone());
                }
                if compression == 0 {
                    if let Some(class_bytes) =
                        stored_zip_entry(bytes, local_offset, compressed_size)
                    {
                        for method in jar_methods_from_class_bytes(&class_name, class_bytes) {
                            if seen.insert(format!("method:{method}")) {
                                entries.push(method);
                            }
                        }
                    }
                }
            }
        }

        offset = name_end
            .saturating_add(extra_len)
            .saturating_add(comment_len)
            .min(bytes.len());
    }

    entries
}

fn stored_zip_entry(bytes: &[u8], local_offset: usize, compressed_size: usize) -> Option<&[u8]> {
    const LOCAL_FILE_SIG: &[u8; 4] = b"PK\x03\x04";
    const LOCAL_FILE_HEADER_LEN: usize = 30;

    if local_offset + LOCAL_FILE_HEADER_LEN > bytes.len() {
        return None;
    }
    if &bytes[local_offset..local_offset + 4] != LOCAL_FILE_SIG {
        return None;
    }
    let name_len =
        u16::from_le_bytes([bytes[local_offset + 26], bytes[local_offset + 27]]) as usize;
    let extra_len =
        u16::from_le_bytes([bytes[local_offset + 28], bytes[local_offset + 29]]) as usize;
    let data_start = local_offset
        .saturating_add(LOCAL_FILE_HEADER_LEN)
        .saturating_add(name_len)
        .saturating_add(extra_len);
    let data_end = data_start.saturating_add(compressed_size);
    (data_end <= bytes.len()).then_some(&bytes[data_start..data_end])
}

fn javadoc_methods_from_html(entry_name: &str, html: &str) -> Vec<String> {
    let Some(class_name) = javadoc_class_name(entry_name) else {
        return Vec::new();
    };
    html.split("member-signature")
        .skip(1)
        .filter_map(|section| javadoc_method_from_section(&class_name, section))
        .collect()
}

fn javadoc_method_from_section(class_name: &str, section: &str) -> Option<String> {
    let return_type = html_class_text(section, "return-type")?;
    let method_name = html_class_text(section, "element-name")?;
    let params = html_class_text(section, "parameters")
        .unwrap_or_default()
        .trim_matches(|ch| ch == '(' || ch == ')')
        .to_string();
    let params = parse_javadoc_params(&params).join(",");
    let doc = html_class_text(section, "block").unwrap_or_default();
    Some(format!(
        "javadoc_method\t{}\t{}\t{}\t{}\t{}",
        class_name,
        method_name,
        params,
        return_type,
        sanitize_entry_field(&doc)
    ))
}

fn javadoc_class_name(entry_name: &str) -> Option<String> {
    if entry_name.ends_with("package-summary.html")
        || entry_name.ends_with("module-summary.html")
        || entry_name.ends_with("index.html")
        || entry_name.contains("/doc-files/")
    {
        return None;
    }
    Some(
        entry_name
            .trim_end_matches(".html")
            .replace('/', ".")
            .replace('\\', "."),
    )
}

fn html_class_text(section: &str, class_name: &str) -> Option<String> {
    let marker = format!("class=\"{class_name}\"");
    let start = section.find(&marker)? + marker.len();
    let after_marker = &section[start..];
    let open = after_marker.find('>')? + 1;
    let after_open = &after_marker[open..];
    let close = after_open.find('<')?;
    let value = strip_html_tags(&after_open[..close]);
    (!value.is_empty()).then_some(value)
}

fn strip_html_tags(input: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    decode_html_entities(out.trim())
}

fn decode_html_entities(input: &str) -> String {
    input
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
}

fn parse_javadoc_params(params: &str) -> Vec<String> {
    params
        .split(',')
        .filter_map(|param| {
            let tokens = param.split_whitespace().collect::<Vec<_>>();
            if tokens.len() >= 2 {
                Some(tokens[..tokens.len() - 1].join(" "))
            } else if tokens.len() == 1 && !tokens[0].is_empty() {
                Some(tokens[0].to_string())
            } else {
                None
            }
        })
        .collect()
}

fn source_methods_from_java_source(entry_name: &str, src: &str) -> Vec<String> {
    let class_name = source_class_name(entry_name, src);
    let mut entries = Vec::new();
    let mut in_doc = false;
    let mut doc_lines = Vec::new();
    let mut pending_doc = String::new();

    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("/**") {
            in_doc = true;
            doc_lines.clear();
            push_clean_doc_line(trimmed, &mut doc_lines);
            if trimmed.contains("*/") {
                in_doc = false;
                pending_doc = doc_lines.join(" ");
            }
            continue;
        }
        if in_doc {
            push_clean_doc_line(trimmed, &mut doc_lines);
            if trimmed.contains("*/") {
                in_doc = false;
                pending_doc = doc_lines.join(" ");
            }
            continue;
        }
        if let Some(method) = parse_source_method_decl(trimmed) {
            entries.push(format!(
                "source_method\t{}\t{}\t{}\t{}\t{}",
                class_name,
                method.name,
                method.params.join(","),
                method.return_type,
                sanitize_entry_field(&pending_doc)
            ));
            pending_doc.clear();
        } else if !trimmed.is_empty() && !trimmed.starts_with('@') {
            pending_doc.clear();
        }
    }

    entries
}

fn source_class_name(entry_name: &str, src: &str) -> String {
    let class = Path::new(entry_name)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Unknown");
    let package = extract_package(src);
    if package.is_empty() {
        entry_name
            .trim_end_matches(".java")
            .replace('/', ".")
            .replace('\\', ".")
    } else {
        format!("{package}.{class}")
    }
}

struct SourceMethodDecl {
    name: String,
    params: Vec<String>,
    return_type: String,
}

fn parse_source_method_decl(line: &str) -> Option<SourceMethodDecl> {
    if !(line.starts_with("public ") || line.starts_with("protected ")) {
        return None;
    }
    if line.contains(" class ") || line.contains(" interface ") || line.contains(" enum ") {
        return None;
    }
    let open = line.find('(')?;
    let close = line[open..].find(')')? + open;
    let before = line[..open].split_whitespace().collect::<Vec<_>>();
    if before.len() < 2 {
        return None;
    }
    let name = before.last()?.to_string();
    let return_type = before
        .iter()
        .rev()
        .skip(1)
        .find(|token| !is_java_modifier(token))?
        .to_string();
    let params = parse_source_params(&line[open + 1..close]);
    Some(SourceMethodDecl {
        name,
        params,
        return_type,
    })
}

fn parse_source_params(params: &str) -> Vec<String> {
    params
        .split(',')
        .filter_map(|param| {
            let tokens = param
                .split_whitespace()
                .filter(|token| !token.starts_with('@') && !is_java_modifier(token))
                .collect::<Vec<_>>();
            if tokens.len() >= 2 {
                Some(tokens[..tokens.len() - 1].join(" "))
            } else {
                None
            }
        })
        .collect()
}

fn is_java_modifier(token: &str) -> bool {
    matches!(
        token,
        "public"
            | "protected"
            | "private"
            | "static"
            | "final"
            | "abstract"
            | "synchronized"
            | "native"
            | "strictfp"
            | "default"
    )
}

fn push_clean_doc_line(line: &str, out: &mut Vec<String>) {
    let cleaned = line
        .trim()
        .trim_start_matches("/**")
        .trim_start_matches('*')
        .trim_end_matches("*/")
        .trim();
    if !cleaned.is_empty() {
        out.push(cleaned.to_string());
    }
}

fn sanitize_entry_field(value: &str) -> String {
    value.replace(['\t', '\n', '\r'], " ")
}

fn class_name_from_jar_entry(entry_name: &str) -> Option<String> {
    if !entry_name.ends_with(".class")
        || entry_name.starts_with("META-INF/")
        || entry_name.contains('$')
    {
        return None;
    }

    let class_path = entry_name.trim_end_matches(".class");
    if class_path.ends_with("module-info") || class_path.ends_with("package-info") {
        return None;
    }

    Some(class_path.replace('/', "."))
}

fn jar_methods_from_class_bytes(class_name: &str, bytes: &[u8]) -> Vec<String> {
    let Some(methods) = parse_class_methods(bytes) else {
        return Vec::new();
    };
    methods
        .into_iter()
        .map(|method| {
            let params = method.params.join(",");
            let static_flag = if method.is_static { "\tstatic" } else { "" };
            format!(
                "method\t{}\t{}\t{}\t{}{}",
                class_name, method.name, params, method.return_type, static_flag
            )
        })
        .collect()
}

struct ParsedClassMethod {
    name: String,
    params: Vec<String>,
    return_type: String,
    is_static: bool,
}

#[derive(Clone)]
enum ConstantPoolEntry {
    Utf8(String),
}

fn parse_class_methods(bytes: &[u8]) -> Option<Vec<ParsedClassMethod>> {
    let mut reader = ClassReader::new(bytes);
    if reader.read_u4()? != 0xCAFEBABE {
        return None;
    }
    reader.skip(4)?; // minor + major
    let cp_count = reader.read_u2()? as usize;
    let mut cp = vec![None; cp_count];
    let mut i = 1usize;
    while i < cp_count {
        let tag = reader.read_u1()?;
        match tag {
            1 => {
                let len = reader.read_u2()? as usize;
                let value = std::str::from_utf8(reader.read_bytes(len)?)
                    .ok()?
                    .to_string();
                cp[i] = Some(ConstantPoolEntry::Utf8(value));
            }
            3 | 4 => reader.skip(4)?,
            5 | 6 => {
                reader.skip(8)?;
                i += 1;
            }
            7 | 8 | 16 | 19 | 20 => reader.skip(2)?,
            9 | 10 | 11 | 12 | 17 | 18 => reader.skip(4)?,
            15 => reader.skip(3)?,
            _ => return None,
        }
        i += 1;
    }

    reader.skip(6)?; // access_flags + this_class + super_class
    let interfaces = reader.read_u2()? as usize;
    reader.skip(interfaces * 2)?;
    skip_members(&mut reader)?; // fields

    let method_count = reader.read_u2()? as usize;
    let mut methods = Vec::new();
    for _ in 0..method_count {
        let access = reader.read_u2()?;
        let name_index = reader.read_u2()? as usize;
        let descriptor_index = reader.read_u2()? as usize;
        skip_attributes(&mut reader)?;

        if !is_indexable_method(access) {
            continue;
        }
        let name = cp_utf8(&cp, name_index)?;
        if name == "<clinit>" {
            continue;
        }
        let descriptor = cp_utf8(&cp, descriptor_index)?;
        let (params, return_type) = parse_method_descriptor(descriptor)?;
        methods.push(ParsedClassMethod {
            name: name.to_string(),
            params,
            return_type,
            is_static: access & 0x0008 != 0,
        });
    }

    Some(methods)
}

fn is_indexable_method(access: u16) -> bool {
    let public_or_protected = access & 0x0005 != 0;
    let private = access & 0x0002 != 0;
    let bridge = access & 0x0040 != 0;
    let synthetic = access & 0x1000 != 0;
    public_or_protected && !private && !bridge && !synthetic
}

fn skip_members(reader: &mut ClassReader<'_>) -> Option<()> {
    let count = reader.read_u2()? as usize;
    for _ in 0..count {
        reader.skip(6)?; // access_flags + name_index + descriptor_index
        skip_attributes(reader)?;
    }
    Some(())
}

fn skip_attributes(reader: &mut ClassReader<'_>) -> Option<()> {
    let count = reader.read_u2()? as usize;
    for _ in 0..count {
        reader.skip(2)?; // attribute_name_index
        let len = reader.read_u4()? as usize;
        reader.skip(len)?;
    }
    Some(())
}

fn cp_utf8(cp: &[Option<ConstantPoolEntry>], index: usize) -> Option<&str> {
    match cp.get(index)? {
        Some(ConstantPoolEntry::Utf8(value)) => Some(value.as_str()),
        _ => None,
    }
}

fn parse_method_descriptor(descriptor: &str) -> Option<(Vec<String>, String)> {
    let bytes = descriptor.as_bytes();
    if bytes.first().copied()? != b'(' {
        return None;
    }
    let mut idx = 1usize;
    let mut params = Vec::new();
    while idx < bytes.len() && bytes[idx] != b')' {
        params.push(parse_descriptor_type(descriptor, &mut idx)?);
    }
    idx += 1;
    let return_type = parse_descriptor_type(descriptor, &mut idx)?;
    Some((params, return_type))
}

fn parse_descriptor_type(descriptor: &str, idx: &mut usize) -> Option<String> {
    let bytes = descriptor.as_bytes();
    let mut array_depth = 0usize;
    while *idx < bytes.len() && bytes[*idx] == b'[' {
        array_depth += 1;
        *idx += 1;
    }
    let base = match *bytes.get(*idx)? as char {
        'B' => {
            *idx += 1;
            "byte".to_string()
        }
        'C' => {
            *idx += 1;
            "char".to_string()
        }
        'D' => {
            *idx += 1;
            "double".to_string()
        }
        'F' => {
            *idx += 1;
            "float".to_string()
        }
        'I' => {
            *idx += 1;
            "int".to_string()
        }
        'J' => {
            *idx += 1;
            "long".to_string()
        }
        'S' => {
            *idx += 1;
            "short".to_string()
        }
        'Z' => {
            *idx += 1;
            "boolean".to_string()
        }
        'V' => {
            *idx += 1;
            "void".to_string()
        }
        'L' => {
            *idx += 1;
            let start = *idx;
            while *idx < bytes.len() && bytes[*idx] != b';' {
                *idx += 1;
            }
            let ty = descriptor[start..*idx].replace('/', ".");
            *idx += 1;
            ty
        }
        _ => return None,
    };
    Some(format!("{}{}", base, "[]".repeat(array_depth)))
}

struct ClassReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ClassReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn read_u1(&mut self) -> Option<u8> {
        let value = *self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(value)
    }

    fn read_u2(&mut self) -> Option<u16> {
        let bytes = self.read_bytes(2)?;
        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_u4(&mut self) -> Option<u32> {
        let bytes = self.read_bytes(4)?;
        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        let bytes = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(bytes)
    }

    fn skip(&mut self, len: usize) -> Option<()> {
        self.read_bytes(len).map(|_| ())
    }
}

fn external_capability(
    rel_path: &str,
    line: u32,
    package: String,
    class: Option<String>,
    method: String,
    signature: String,
    annotations: Vec<String>,
    doc: String,
) -> Capability {
    Capability {
        id: 0,
        kind: Kind::Other,
        lang: Lang::Java,
        module: extract_module(rel_path),
        package,
        class,
        method,
        signature,
        annotations,
        http: None,
        rpc: None,
        doc: Some(doc),
        tags: vec!["external".to_string()],
        file: rel_path.to_string(),
        line,
        byte_range: (0, 0),
        terms: vec![],
    }
}

fn byte_offset_to_line(src: &str, byte_offset: usize) -> u32 {
    src[..byte_offset.min(src.len())].matches('\n').count() as u32 + 1
}

fn load_capfindignore(repo_root: &Path) -> Result<Option<Gitignore>> {
    let path = repo_root.join(".capfindignore");
    if !path.exists() {
        return Ok(None);
    }

    let mut builder = GitignoreBuilder::new(repo_root);
    if let Some(err) = builder.add(&path) {
        return Err(err.into());
    }
    Ok(Some(builder.build()?))
}

fn is_capfind_ignored(ignore: Option<&Gitignore>, path: &Path, is_dir: bool) -> bool {
    ignore
        .map(|ignore| ignore.matched(path, is_dir).is_ignore())
        .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const CONTROLLER: &str = r#"
package com.demo;

import org.springframework.web.bind.annotation.*;

@RestController
public class DemoController {
    @GetMapping("/demo")
    public String demo() {
        return "ok";
    }
}
"#;

    #[test]
    fn capfindignore_excludes_matching_sources() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".capfindignore"), "ignored/**\n").unwrap();
        fs::create_dir_all(root.join("included")).unwrap();
        fs::create_dir_all(root.join("ignored")).unwrap();
        fs::write(root.join("included/IncludedController.java"), CONTROLLER).unwrap();
        fs::write(root.join("ignored/IgnoredController.java"), CONTROLLER).unwrap();

        let caps = scan_and_parse(root).unwrap();
        let endpoints = caps
            .iter()
            .filter(|cap| cap.kind == Kind::HttpEndpoint)
            .collect::<Vec<_>>();

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].file, "included/IncludedController.java");
    }

    #[test]
    fn capfindignore_negation_can_reinclude_sources() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join(".capfindignore"),
            "*.java\n!KeepController.java\n",
        )
        .unwrap();
        fs::write(root.join("SkipController.java"), CONTROLLER).unwrap();
        fs::write(root.join("KeepController.java"), CONTROLLER).unwrap();

        let caps = scan_and_parse(root).unwrap();
        let endpoints = caps
            .iter()
            .filter(|cap| cap.kind == Kind::HttpEndpoint)
            .collect::<Vec<_>>();

        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].file, "KeepController.java");
    }

    fn fake_jar_with_classes(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut central = Vec::new();
        for (entry, data) in entries {
            let local_offset = bytes.len() as u32;
            bytes.extend(b"PK\x03\x04");
            bytes.extend([20, 0, 0, 0]); // version + flags
            bytes.extend(0u16.to_le_bytes()); // stored
            bytes.extend([0u8; 8]); // time/date/crc
            bytes.extend((data.len() as u32).to_le_bytes());
            bytes.extend((data.len() as u32).to_le_bytes());
            bytes.extend((entry.len() as u16).to_le_bytes());
            bytes.extend(0u16.to_le_bytes());
            bytes.extend(entry.as_bytes());
            bytes.extend_from_slice(data);

            let mut header = vec![0u8; 46];
            header[0..4].copy_from_slice(b"PK\x01\x02");
            header[10..12].copy_from_slice(&0u16.to_le_bytes());
            header[20..24].copy_from_slice(&(data.len() as u32).to_le_bytes());
            header[24..28].copy_from_slice(&(data.len() as u32).to_le_bytes());
            header[28..30].copy_from_slice(&(entry.len() as u16).to_le_bytes());
            header[42..46].copy_from_slice(&local_offset.to_le_bytes());
            central.extend(header);
            central.extend(entry.as_bytes());
        }
        bytes.extend(central);
        bytes
    }

    fn fake_class_with_methods(methods: &[(&str, &str, u16)]) -> Vec<u8> {
        let mut cp = vec![
            (1u8, "com/vendor/ClientApi".to_string()),
            (7u8, "\u{1}".to_string()),
            (1u8, "java/lang/Object".to_string()),
            (7u8, "\u{3}".to_string()),
        ];
        for (name, descriptor, _) in methods {
            cp.push((1, (*name).to_string()));
            cp.push((1, (*descriptor).to_string()));
        }

        let mut bytes = Vec::new();
        bytes.extend(0xCAFEBABEu32.to_be_bytes());
        bytes.extend(0u16.to_be_bytes());
        bytes.extend(52u16.to_be_bytes());
        bytes.extend(((cp.len() + 1) as u16).to_be_bytes());
        for (tag, value) in &cp {
            bytes.push(*tag);
            if *tag == 1 {
                bytes.extend((value.len() as u16).to_be_bytes());
                bytes.extend(value.as_bytes());
            } else {
                bytes.extend((value.as_bytes()[0] as u16).to_be_bytes());
            }
        }
        bytes.extend(0x0021u16.to_be_bytes()); // public + super
        bytes.extend(2u16.to_be_bytes()); // this_class
        bytes.extend(4u16.to_be_bytes()); // super_class
        bytes.extend(0u16.to_be_bytes()); // interfaces
        bytes.extend(0u16.to_be_bytes()); // fields
        bytes.extend((methods.len() as u16).to_be_bytes());
        for (idx, (_, _, access)) in methods.iter().enumerate() {
            let name_index = 5 + idx * 2;
            let descriptor_index = name_index + 1;
            bytes.extend((*access).to_be_bytes());
            bytes.extend((name_index as u16).to_be_bytes());
            bytes.extend((descriptor_index as u16).to_be_bytes());
            bytes.extend(0u16.to_be_bytes()); // attributes
        }
        bytes.extend(0u16.to_be_bytes()); // class attributes
        bytes
    }

    #[test]
    fn scans_external_dependencies_and_imported_apis() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src/main/java/com/demo")).unwrap();
        fs::create_dir_all(root.join("libs")).unwrap();
        fs::write(
            root.join("src/main/java/com/demo/UsesExternal.java"),
            r#"
package com.demo;

import com.demo.internal.LocalService;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.util.List;
import org.springframework.web.client.RestTemplate;

public class UsesExternal {}
"#,
        )
        .unwrap();
        fs::write(
            root.join("pom.xml"),
            r#"
<project>
  <dependencies>
    <dependency>
      <groupId>com.fasterxml.jackson.core</groupId>
      <artifactId>jackson-databind</artifactId>
      <version>2.17.0</version>
    </dependency>
    <dependency>
      <groupId>junit</groupId>
      <artifactId>junit</artifactId>
      <version>4.13.2</version>
      <scope>test</scope>
    </dependency>
  </dependencies>
</project>
"#,
        )
        .unwrap();
        fs::write(
            root.join("build.gradle"),
            "implementation 'org.apache.commons:commons-lang3:3.14.0'\n",
        )
        .unwrap();
        fs::write(
            root.join("libs/vendor-api-1.0.jar"),
            fake_jar_with_classes(&[
                (
                    "com/vendor/ClientApi.class",
                    fake_class_with_methods(&[
                        (
                            "query",
                            "(Lcom/vendor/QueryRequest;I)Lcom/vendor/QueryResponse;",
                            0x0001,
                        ),
                        ("helper", "()V", 0x0002),
                    ]),
                ),
                ("com/vendor/internal/ClientApi$Builder.class", Vec::new()),
                ("module-info.class", Vec::new()),
            ]),
        )
        .unwrap();
        fs::write(
            root.join("libs/vendor-api-1.0-sources.jar"),
            fake_jar_with_classes(&[(
                "com/vendor/ClientApi.java",
                br#"
package com.vendor;

public class ClientApi {
    /**
     * Query vendor profile by request and limit.
     */
    public QueryResponse query(QueryRequest request, int limit) { return null; }
}
"#
                .to_vec(),
            )]),
        )
        .unwrap();
        fs::write(
            root.join("libs/vendor-api-1.0-javadoc.jar"),
            fake_jar_with_classes(&[(
                "com/vendor/ClientApi.html",
                br#"
<html><body>
<section class="detail" id="query(com.vendor.QueryRequest,int)">
<h3>query</h3>
<div class="member-signature">
  <span class="return-type">QueryResponse</span>
  <span class="element-name">query</span>
  <span class="parameters">(QueryRequest request, int limit)</span>
</div>
<div class="block">Query vendor profile from published Javadoc.</div>
</section>
</body></html>
"#
                .to_vec(),
            )]),
        )
        .unwrap();

        let caps = scan_and_parse(root).unwrap();
        let signatures = caps
            .iter()
            .map(|cap| cap.signature.as_str())
            .collect::<Vec<_>>();

        assert!(signatures.contains(&"import com.fasterxml.jackson.databind.ObjectMapper"));
        assert!(signatures.contains(&"import org.springframework.web.client.RestTemplate"));
        assert!(signatures
            .contains(&"maven dependency com.fasterxml.jackson.core:jackson-databind:2.17.0"));
        assert!(signatures
            .contains(&"implementation dependency org.apache.commons:commons-lang3:3.14.0"));
        assert!(signatures.contains(&"local jar vendor-api-1.0.jar"));
        assert!(signatures.contains(&"jar class com.vendor.ClientApi"));
        assert!(signatures.contains(&
            "jar method com.vendor.ClientApi#query(com.vendor.QueryRequest, int): com.vendor.QueryResponse"
        ));
        assert!(signatures.contains(
            &"source method com.vendor.ClientApi#query(QueryRequest, int): QueryResponse"
        ));
        assert!(caps.iter().any(|cap| {
            cap.signature
                == "source method com.vendor.ClientApi#query(QueryRequest, int): QueryResponse"
                && cap
                    .doc
                    .as_deref()
                    .unwrap_or_default()
                    .contains("Query vendor profile")
        }));
        assert!(signatures.contains(
            &"javadoc method com.vendor.ClientApi#query(QueryRequest, int): QueryResponse"
        ));
        assert!(caps.iter().any(|cap| {
            cap.signature
                == "javadoc method com.vendor.ClientApi#query(QueryRequest, int): QueryResponse"
                && cap
                    .doc
                    .as_deref()
                    .unwrap_or_default()
                    .contains("published Javadoc")
        }));
        assert!(!signatures.contains(&"jar method com.vendor.ClientApi#helper(): void"));
        assert!(!signatures.contains(&"jar class com.vendor.internal.ClientApi$Builder"));
        assert!(!signatures.contains(&"jar class module-info"));
        assert!(!signatures.contains(&"import com.demo.internal.LocalService"));
        assert!(!signatures.contains(&"import java.util.List"));
        assert!(!signatures.contains(&"maven dependency junit:junit:4.13.2"));
    }
}
