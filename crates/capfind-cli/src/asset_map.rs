//! Machine-readable code asset map for agents and dashboards.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use capfind_core::{Capability, IndexBody};
use regex::Regex;
use serde_json::{json, Value};

use crate::config;
use crate::context as ai_context;

pub const ASSET_MAP_SCHEMA_VERSION: &str = "capfind.asset_map.v1";

#[derive(Clone)]
struct Edge {
    from: String,
    to: String,
    relationship: String,
    confidence: &'static str,
    evidence: Option<Value>,
}

struct JavaCallEdge<'a> {
    source: &'a Capability,
    target: &'a Capability,
    relationship: &'static str,
    confidence: &'static str,
    evidence: Value,
}

struct JavaCallResolution<'a> {
    target: &'a Capability,
    confidence: &'static str,
    resolution: &'static str,
    resolved_class: Option<String>,
}

struct JavaDependencyEdge<'a> {
    source: &'a Capability,
    target: &'a Capability,
    target_node: String,
    confidence: &'static str,
    evidence: Value,
}

#[derive(Default)]
struct JavaGraphAnalysis<'a> {
    call_edges: Vec<JavaCallEdge<'a>>,
    dependency_edges: Vec<JavaDependencyEdge<'a>>,
    diagnostics: Vec<Value>,
}

enum JavaCallResolutionResult<'a> {
    Resolved(JavaCallResolution<'a>),
    Ambiguous {
        reason: &'static str,
        candidates: Vec<&'a Capability>,
    },
    Unresolved,
}

struct JavaInjection {
    kind: &'static str,
    type_name: String,
    name: String,
    line: u32,
    annotations: Vec<String>,
}

struct JavaDependencyResolution<'a> {
    target: &'a Capability,
    target_class: String,
    confidence: &'static str,
    resolution: &'static str,
}

enum JavaDependencyResolutionResult<'a> {
    Resolved(JavaDependencyResolution<'a>),
    Ambiguous {
        reason: &'static str,
        candidates: Vec<&'a Capability>,
    },
    Unresolved,
}

#[derive(Default)]
struct ModuleStats {
    capability_count: u64,
    external_assets: u64,
    by_type: BTreeMap<String, u64>,
    packages: BTreeMap<String, u64>,
    entrypoints: Vec<Value>,
}

#[derive(Default)]
struct OwnershipCatalog {
    modules: BTreeMap<String, String>,
    packages: BTreeMap<String, String>,
    external: BTreeMap<String, String>,
    services: BTreeMap<String, config::ServiceConfig>,
}

#[derive(Clone)]
struct OwnershipInfo {
    owner: String,
    source: &'static str,
    matched: Option<String>,
    kind: &'static str,
}

impl OwnershipInfo {
    fn to_json(&self) -> Value {
        json!({
            "owner": self.owner,
            "source": self.source,
            "matched": self.matched,
            "kind": self.kind,
        })
    }
}

#[allow(dead_code)]
pub fn asset_map_json(body: &IndexBody, module_filter: Option<&str>, limit: usize) -> Value {
    asset_map_json_inner(body, None, module_filter, limit)
}

pub fn asset_map_json_with_repo(
    body: &IndexBody,
    repo_root: &Path,
    module_filter: Option<&str>,
    limit: usize,
) -> Value {
    asset_map_json_inner(body, Some(repo_root), module_filter, limit)
}

fn asset_map_json_inner(
    body: &IndexBody,
    repo_root: Option<&Path>,
    module_filter: Option<&str>,
    limit: usize,
) -> Value {
    let ownership = ownership_catalog(repo_root);
    let caps = body
        .capabilities
        .iter()
        .filter(|cap| matches_module_filter(cap, module_filter))
        .collect::<Vec<_>>();
    let selected = selected_caps(&caps, limit);
    let modules = module_summaries(&caps, &ownership);
    let external = external_summary(&caps, &ownership);
    let graph = graph_json(&caps, &selected, repo_root, &ownership);

    json!({
        "schema_version": ASSET_MAP_SCHEMA_VERSION,
        "filters": {
            "module": module_filter,
            "limit": limit,
        },
        "totals": {
            "capabilities": caps.len(),
            "modules": distinct_count(caps.iter().map(|cap| module_name(cap))),
            "packages": distinct_count(caps.iter().filter_map(|cap| non_empty(&cap.package))),
            "external_assets": caps.iter().filter(|cap| ai_context::is_external(cap)).count(),
            "indexed_files": body.file_stats.len(),
        },
        "modules": modules,
        "external": external,
        "ownership": ownership_summary(&caps, &ownership),
        "services": service_registry_summary(&caps, &ownership),
        "graph": graph,
    })
}

fn ownership_catalog(repo_root: Option<&Path>) -> OwnershipCatalog {
    repo_root
        .and_then(|root| config::load(root).ok())
        .map(|cfg| OwnershipCatalog {
            modules: cfg.ownership.modules,
            packages: cfg.ownership.packages,
            external: cfg.ownership.external,
            services: cfg.services,
        })
        .unwrap_or_default()
}

fn matches_module_filter(cap: &Capability, module_filter: Option<&str>) -> bool {
    let Some(filter) = module_filter
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    cap.module == filter || cap.module.starts_with(filter) || cap.file.starts_with(filter)
}

fn selected_caps<'a>(caps: &[&'a Capability], limit: usize) -> Vec<&'a Capability> {
    let mut selected = caps.to_vec();
    selected.sort_by(|a, b| {
        type_rank(ai_context::candidate_type(a))
            .cmp(&type_rank(ai_context::candidate_type(b)))
            .then(a.module.cmp(&b.module))
            .then(a.package.cmp(&b.package))
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
            .then(a.id.cmp(&b.id))
    });
    selected.truncate(limit);
    selected
}

fn module_summaries(caps: &[&Capability], ownership: &OwnershipCatalog) -> Vec<Value> {
    let mut stats = BTreeMap::<String, ModuleStats>::new();
    for cap in caps {
        let module = module_name(cap).to_string();
        let entry = stats.entry(module).or_default();
        entry.capability_count += 1;
        if ai_context::is_external(cap) {
            entry.external_assets += 1;
        }
        *entry
            .by_type
            .entry(ai_context::candidate_type(cap).to_string())
            .or_insert(0) += 1;
        if !cap.package.is_empty() {
            *entry.packages.entry(cap.package.clone()).or_insert(0) += 1;
        }
        if let Some(entrypoint) = entrypoint_json(cap) {
            entry.entrypoints.push(entrypoint);
        }
    }

    let mut modules = stats
        .into_iter()
        .map(|(name, mut stat)| {
            stat.entrypoints.truncate(12);
            json!({
                "name": name,
                "capability_count": stat.capability_count,
                "external_assets": stat.external_assets,
                "ownership": ownership_for_module(&name, ownership).to_json(),
                "service": service_for_module(&name, ownership),
                "by_type": stat.by_type,
                "top_packages": top_counts(stat.packages, 10),
                "entrypoints": stat.entrypoints,
            })
        })
        .collect::<Vec<_>>();
    modules.sort_by(|a, b| {
        b["capability_count"]
            .as_u64()
            .cmp(&a["capability_count"].as_u64())
            .then(a["name"].as_str().cmp(&b["name"].as_str()))
    });
    modules
}

fn external_summary(caps: &[&Capability], ownership: &OwnershipCatalog) -> Value {
    let mut dependencies = Vec::new();
    let mut api_usages = Vec::new();
    let mut jar_methods = Vec::new();

    for cap in caps.iter().filter(|cap| ai_context::is_external(cap)) {
        let item = cap_ref_json_with_ownership(cap, ownership);
        match ai_context::candidate_type(cap) {
            "external_dependency" => dependencies.push(item),
            "external_api_usage" => api_usages.push(item),
            "jar_method" | "sources_jar_method" | "javadoc_method" => jar_methods.push(item),
            _ => {}
        }
    }
    dependencies.truncate(20);
    api_usages.truncate(20);
    jar_methods.truncate(20);

    json!({
        "dependencies": dependencies,
        "api_usages": api_usages,
        "jar_methods": jar_methods,
        "ownership": external_ownership_summary(caps, ownership),
    })
}

fn graph_json(
    all_caps: &[&Capability],
    selected: &[&Capability],
    repo_root: Option<&Path>,
    ownership: &OwnershipCatalog,
) -> Value {
    let module_counts = count_strings(all_caps.iter().map(|cap| module_name(cap)));
    let package_counts = count_strings(all_caps.iter().filter_map(|cap| non_empty(&cap.package)));
    let class_counts = count_strings(all_caps.iter().filter_map(|cap| cap.class.as_deref()));
    let graph_ctx = GraphBuildContext {
        module_counts: &module_counts,
        package_counts: &package_counts,
        class_counts: &class_counts,
        ownership,
    };

    let mut nodes = Vec::new();
    let mut seen_nodes = BTreeSet::new();
    let mut edges = BTreeMap::<(String, String, String), Edge>::new();
    let mut diagnostics = Vec::new();

    for cap in selected {
        push_capability_graph(cap, &graph_ctx, &mut nodes, &mut seen_nodes, &mut edges);
    }

    if let Some(repo_root) = repo_root {
        let analysis = java_graph_analysis(repo_root, all_caps, selected);
        for call_edge in analysis.call_edges {
            let from = format!("cap:{}", call_edge.source.id);
            let to = push_capability_graph(
                call_edge.target,
                &graph_ctx,
                &mut nodes,
                &mut seen_nodes,
                &mut edges,
            );
            push_edge(
                &mut edges,
                from.clone(),
                to.clone(),
                call_edge.relationship,
                call_edge.confidence,
                Some(call_edge.evidence.clone()),
            );
            for relationship in semantic_call_relationships(&call_edge) {
                push_edge(
                    &mut edges,
                    from.clone(),
                    to.clone(),
                    relationship,
                    call_edge.confidence,
                    Some(semantic_call_evidence(&call_edge, relationship)),
                );
            }
        }
        for dependency_edge in analysis.dependency_edges {
            let from = format!("cap:{}", dependency_edge.source.id);
            push_capability_graph(
                dependency_edge.target,
                &graph_ctx,
                &mut nodes,
                &mut seen_nodes,
                &mut edges,
            );
            push_edge(
                &mut edges,
                from,
                dependency_edge.target_node,
                "depends_on",
                dependency_edge.confidence,
                Some(dependency_edge.evidence),
            );
        }
        diagnostics = analysis.diagnostics;
    }

    let mut relationship_counts = BTreeMap::<String, u64>::new();
    let edge_values = edges
        .into_values()
        .map(|edge| {
            *relationship_counts
                .entry(edge.relationship.clone())
                .or_insert(0) += 1;
            let mut value = json!({
                "from": edge.from,
                "to": edge.to,
                "relationship": edge.relationship,
                "confidence": edge.confidence,
            });
            if let Some(evidence) = edge.evidence {
                value["evidence"] = evidence;
            }
            value
        })
        .collect::<Vec<_>>();

    json!({
        "node_count": nodes.len(),
        "edge_count": edge_values.len(),
        "relationship_counts": relationship_counts,
        "nodes": nodes,
        "edges": edge_values,
        "diagnostics": diagnostics,
    })
}

fn push_capability_graph(
    cap: &Capability,
    ctx: &GraphBuildContext<'_>,
    nodes: &mut Vec<Value>,
    seen_nodes: &mut BTreeSet<String>,
    edges: &mut BTreeMap<(String, String, String), Edge>,
) -> String {
    let module = module_name(cap);
    let module_id = format!("module:{module}");
    push_node(
        nodes,
        seen_nodes,
        json!({
            "id": module_id,
            "kind": "module",
            "name": module,
            "capability_count": ctx.module_counts.get(module).copied().unwrap_or_default(),
            "ownership": ownership_for_module(module, ctx.ownership).to_json(),
            "service": service_for_module(module, ctx.ownership),
        }),
    );

    let mut parent_id = module_id.clone();
    if let Some(package) = non_empty(&cap.package) {
        let package_id = format!("package:{package}");
        push_node(
            nodes,
            seen_nodes,
            json!({
                "id": package_id,
                "kind": "package",
                "name": package,
                "module": module,
                "capability_count": ctx.package_counts.get(package).copied().unwrap_or_default(),
                "ownership": ownership_for_package(package, module, ctx.ownership).to_json(),
                "service": service_for_package(package, module, ctx.ownership),
            }),
        );
        push_edge(
            edges,
            module_id.clone(),
            package_id.clone(),
            "contains_package",
            "structural",
            None,
        );
        parent_id = package_id;
    }

    if let Some(class) = cap.class.as_deref() {
        let class_id = format!("class:{class}");
        push_node(
            nodes,
            seen_nodes,
            json!({
                "id": class_id,
                "kind": "class",
                "name": class,
                "module": module,
                "package": cap.package,
                "capability_count": ctx.class_counts.get(class).copied().unwrap_or_default(),
                "external": ai_context::is_external(cap),
                "framework": framework_name(cap),
                "ownership": ownership_for_cap(cap, ctx.ownership).to_json(),
                "service": service_for_cap(cap, ctx.ownership),
            }),
        );
        push_edge(
            edges,
            parent_id.clone(),
            class_id.clone(),
            "contains_class",
            "structural",
            None,
        );
        parent_id = class_id;
    }

    let cap_id = format!("cap:{}", cap.id);
    push_node(
        nodes,
        seen_nodes,
        json!({
            "id": cap_id,
            "kind": "capability",
            "capability_id": cap.id,
            "type": ai_context::candidate_type(cap),
            "name": display_name(cap),
            "module": module,
            "package": cap.package,
            "file": cap.file,
            "line": cap.line,
            "entrypoint": entrypoint_json(cap),
            "external": ai_context::is_external(cap),
            "framework": framework_name(cap),
            "ownership": ownership_for_cap(cap, ctx.ownership).to_json(),
            "service": service_for_cap(cap, ctx.ownership),
        }),
    );
    push_edge(
        edges,
        parent_id,
        cap_id.clone(),
        "declares_capability",
        "structural",
        None,
    );
    cap_id
}

fn push_edge(
    edges: &mut BTreeMap<(String, String, String), Edge>,
    from: String,
    to: String,
    relationship: &str,
    confidence: &'static str,
    evidence: Option<Value>,
) {
    let relationship = relationship.to_string();
    edges
        .entry((from.clone(), to.clone(), relationship.clone()))
        .or_insert(Edge {
            from,
            to,
            relationship,
            confidence,
            evidence,
        });
}

fn java_graph_analysis<'a>(
    repo_root: &Path,
    all_caps: &[&'a Capability],
    selected: &[&'a Capability],
) -> JavaGraphAnalysis<'a> {
    let call_re = Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(")
        .expect("valid Java call regex");
    let mut source_cache = BTreeMap::<String, Option<String>>::new();
    let implementation_index = java_implementation_index(repo_root, all_caps, &mut source_cache);
    let mut analysis = JavaGraphAnalysis::default();
    let mut seen = BTreeSet::<(u32, u32, u32)>::new();
    let mut seen_dependencies = BTreeSet::<(u32, String, String, String)>::new();

    for source in selected
        .iter()
        .copied()
        .filter(|cap| is_java_project_cap(cap))
    {
        let source_text = cached_source(repo_root, &source.file, &mut source_cache);
        let Some(source_text) = source_text.as_deref() else {
            continue;
        };
        for injection in java_injections(source_text, source.class.as_deref()) {
            if !seen_dependencies.insert((
                source.id,
                injection.kind.to_string(),
                injection.type_name.clone(),
                injection.name.clone(),
            )) {
                continue;
            }
            match resolve_dependency_target(
                all_caps,
                source,
                &injection.type_name,
                &implementation_index,
            ) {
                JavaDependencyResolutionResult::Resolved(resolution) => {
                    analysis.dependency_edges.push(JavaDependencyEdge {
                        source,
                        target: resolution.target,
                        target_node: format!("class:{}", resolution.target_class),
                        confidence: resolution.confidence,
                        evidence: json!({
                            "kind": injection.kind,
                            "file": source.file,
                            "line": injection.line,
                            "dependency_name": injection.name,
                            "declared_type": injection.type_name,
                            "resolved_declaring_class": resolution.target_class,
                            "resolution": resolution.resolution,
                            "annotations": injection.annotations,
                            "source_capability_id": source.id,
                            "target_capability_id": resolution.target.id,
                        }),
                    });
                }
                JavaDependencyResolutionResult::Ambiguous { reason, candidates } => {
                    analysis.diagnostics.push(json!({
                        "kind": "ambiguous_dependency_target",
                        "reason": reason,
                        "file": source.file,
                        "line": injection.line,
                        "dependency_name": injection.name,
                        "declared_type": injection.type_name,
                        "annotations": injection.annotations,
                        "source_capability_id": source.id,
                        "source": cap_ref_json(source),
                        "candidates": candidate_refs(&candidates),
                    }));
                }
                JavaDependencyResolutionResult::Unresolved => {}
            }
        }
        let Some((body_offset, body)) = method_body(source_text, source) else {
            continue;
        };
        let variable_types = java_variable_types(source_text);

        for captures in call_re.captures_iter(body) {
            let qualifier = captures.get(1).map_or("", |m| m.as_str());
            let method = captures.get(2).map_or("", |m| m.as_str());
            let Some(target_class) = resolve_qualifier_type(source, qualifier, &variable_types)
            else {
                continue;
            };
            let absolute_call_offset = body_offset + captures.get(0).map_or(0, |m| m.start());
            let line = byte_offset_to_line(source_text, absolute_call_offset);
            match resolve_call_target(
                all_caps,
                source,
                &target_class,
                method,
                &implementation_index,
            ) {
                JavaCallResolutionResult::Resolved(resolution) => {
                    if !seen.insert((source.id, resolution.target.id, line)) {
                        continue;
                    }
                    analysis.call_edges.push(JavaCallEdge {
                        source,
                        target: resolution.target,
                        relationship: call_relationship(resolution.target),
                        confidence: resolution.confidence,
                        evidence: json!({
                            "kind": "java_qualified_call",
                            "file": source.file,
                            "line": line,
                            "call": format!("{qualifier}.{method}"),
                            "resolved_type": target_class,
                            "resolved_declaring_class": resolution.resolved_class,
                            "resolution": resolution.resolution,
                            "target_type": ai_context::candidate_type(resolution.target),
                            "target_framework": framework_name(resolution.target),
                            "source_capability_id": source.id,
                            "target_capability_id": resolution.target.id,
                        }),
                    });
                }
                JavaCallResolutionResult::Ambiguous { reason, candidates } => {
                    analysis.diagnostics.push(json!({
                        "kind": "ambiguous_call_target",
                        "reason": reason,
                        "file": source.file,
                        "line": line,
                        "call": format!("{qualifier}.{method}"),
                        "resolved_type": target_class,
                        "method": method,
                        "source_capability_id": source.id,
                        "source": cap_ref_json(source),
                        "candidates": candidate_refs(&candidates),
                    }));
                }
                JavaCallResolutionResult::Unresolved => {}
            }
        }
    }

    analysis.diagnostics.sort_by_key(|item| item.to_string());
    analysis
}

fn java_implementation_index(
    repo_root: &Path,
    all_caps: &[&Capability],
    source_cache: &mut BTreeMap<String, Option<String>>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut index = BTreeMap::<String, BTreeSet<String>>::new();
    for cap in all_caps
        .iter()
        .copied()
        .filter(|cap| is_java_project_cap(cap))
    {
        let source_text = cached_source(repo_root, &cap.file, source_cache);
        let Some(source_text) = source_text.as_deref() else {
            continue;
        };
        for class_info in java_class_infos(source_text) {
            for interface in class_info.interfaces {
                index
                    .entry(interface)
                    .or_default()
                    .insert(class_info.class_name.clone());
            }
        }
    }
    index
}

struct JavaClassInfo {
    class_name: String,
    interfaces: Vec<String>,
}

fn java_class_infos(source_text: &str) -> Vec<JavaClassInfo> {
    let class_re =
        Regex::new(r"(?s)\b(?:class|record)\s+([A-Z][A-Za-z0-9_]*)[^{;]*?\bimplements\s+([^{]+)\{")
            .expect("valid Java class implements regex");
    class_re
        .captures_iter(source_text)
        .filter_map(|captures| {
            let class_name = captures.get(1)?.as_str().to_string();
            let interfaces = captures
                .get(2)
                .map(|m| implemented_interfaces(m.as_str()))
                .unwrap_or_default();
            (!interfaces.is_empty()).then_some(JavaClassInfo {
                class_name,
                interfaces,
            })
        })
        .collect()
}

fn implemented_interfaces(raw: &str) -> Vec<String> {
    raw.split(',')
        .filter_map(|part| {
            let name = simple_type(part.trim());
            (!name.is_empty()).then_some(name.to_string())
        })
        .collect()
}

fn cached_source(
    repo_root: &Path,
    rel_path: &str,
    cache: &mut BTreeMap<String, Option<String>>,
) -> Option<String> {
    if !cache.contains_key(rel_path) {
        let path = repo_root.join(rel_path);
        let source = std::fs::read_to_string(path).ok();
        cache.insert(rel_path.to_string(), source);
    }
    cache.get(rel_path).cloned().flatten()
}

fn method_body<'a>(source_text: &'a str, cap: &Capability) -> Option<(usize, &'a str)> {
    let search_from = (cap.byte_range.1 as usize).min(source_text.len());
    let open_rel = source_text[search_from..].find('{')?;
    let open = search_from + open_rel;
    let mut depth = 0usize;
    for (offset, ch) in source_text[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let body_start = open + 1;
                    let body_end = open + offset;
                    return source_text
                        .get(body_start..body_end)
                        .map(|body| (body_start, body));
                }
            }
            _ => {}
        }
    }
    None
}

fn java_variable_types(source_text: &str) -> BTreeMap<String, String> {
    let var_re =
        Regex::new(r"\b([A-Z][A-Za-z0-9_$.]*(?:<[^;=(){}]+>)?(?:\[\])?)\s+([a-z][A-Za-z0-9_]*)\b")
            .expect("valid Java variable regex");
    let mut variables = BTreeMap::new();
    for captures in var_re.captures_iter(source_text) {
        let Some(raw_type) = captures.get(1).map(|m| m.as_str()) else {
            continue;
        };
        let Some(name) = captures.get(2).map(|m| m.as_str()) else {
            continue;
        };
        if matches!(
            name,
            "return" | "new" | "class" | "interface" | "enum" | "record"
        ) {
            continue;
        }
        variables.insert(name.to_string(), simple_type(raw_type).to_string());
    }
    variables
}

fn java_injections(source_text: &str, class_name: Option<&str>) -> Vec<JavaInjection> {
    let field_re = Regex::new(
        r"\b(?:private|protected|public)?\s*(?:final\s+)?([A-Z][A-Za-z0-9_$.]*(?:<[^;=(){}]+>)?)\s+([a-z][A-Za-z0-9_]*)\s*(?:=[^;]*)?;",
    )
    .expect("valid Java field injection regex");
    let annotation_prefix_re =
        Regex::new(r"^(?:@\w+(?:\([^)]*\))?\s*)+").expect("valid Java annotation prefix regex");
    let mut injections = Vec::new();
    let mut pending_annotations = Vec::<String>::new();
    let mut byte_offset = 0usize;
    for line in source_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('@') {
            pending_annotations.extend(java_annotation_names(trimmed));
        }
        let inline_annotations = java_annotation_names(trimmed);
        let all_annotations = merged_annotations(&pending_annotations, &inline_annotations);
        let cleaned = annotation_prefix_re.replace(trimmed, "");
        if has_injection_annotation(&all_annotations) {
            if let Some(captures) = field_re.captures(&cleaned) {
                if let (Some(raw_type), Some(name)) = (captures.get(1), captures.get(2)) {
                    injections.push(JavaInjection {
                        kind: "java_field_injection",
                        type_name: simple_type(raw_type.as_str()).to_string(),
                        name: name.as_str().to_string(),
                        line: byte_offset_to_line(source_text, byte_offset),
                        annotations: all_annotations,
                    });
                }
            }
        }
        if !trimmed.starts_with('@') && !trimmed.is_empty() {
            pending_annotations.clear();
        }
        byte_offset += line.len() + 1;
    }

    if let Some(class_name) = class_name
        .map(simple_type)
        .filter(|value| !value.is_empty())
    {
        injections.extend(java_constructor_injections(source_text, class_name));
    }
    injections
}

fn java_constructor_injections(source_text: &str, class_name: &str) -> Vec<JavaInjection> {
    let constructor_re = Regex::new(&format!(
        r"(?s)\b(?:public|protected|private)?\s*{}\s*\(([^)]*)\)",
        regex::escape(class_name)
    ))
    .expect("valid Java constructor regex");
    let mut injections = Vec::new();
    for captures in constructor_re.captures_iter(source_text) {
        let Some(params) = captures.get(1) else {
            continue;
        };
        let line = captures
            .get(0)
            .map(|m| byte_offset_to_line(source_text, m.start()))
            .unwrap_or_default();
        for param in java_parameters(params.as_str()) {
            let Some((type_name, name, annotations)) = java_parameter_type_name(&param) else {
                continue;
            };
            injections.push(JavaInjection {
                kind: "java_constructor_injection",
                type_name,
                name,
                line,
                annotations,
            });
        }
    }
    injections
}

fn java_parameters(params: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut start = 0usize;
    let mut generic_depth = 0usize;
    for (offset, ch) in params.char_indices() {
        match ch {
            '<' => generic_depth += 1,
            '>' => generic_depth = generic_depth.saturating_sub(1),
            ',' if generic_depth == 0 => {
                values.push(params[start..offset].trim().to_string());
                start = offset + ch.len_utf8();
            }
            _ => {}
        }
    }
    let tail = params[start..].trim();
    if !tail.is_empty() {
        values.push(tail.to_string());
    }
    values
}

fn java_parameter_type_name(param: &str) -> Option<(String, String, Vec<String>)> {
    let annotations = java_annotation_names(param);
    let annotation_re = Regex::new(r"@\w+(?:\([^)]*\))?").expect("valid Java annotation regex");
    let cleaned = annotation_re.replace_all(param, "");
    let cleaned = cleaned
        .replace("final ", "")
        .replace('\n', " ")
        .trim()
        .to_string();
    let param_re =
        Regex::new(r"([A-Z][A-Za-z0-9_$.]*(?:<[^;=(){}]+>)?(?:\[\])?)\s+([a-z][A-Za-z0-9_]*)$")
            .expect("valid Java parameter regex");
    let captures = param_re.captures(&cleaned)?;
    Some((
        simple_type(captures.get(1)?.as_str()).to_string(),
        captures.get(2)?.as_str().to_string(),
        annotations,
    ))
}

fn java_annotation_names(text: &str) -> Vec<String> {
    let annotation_re = Regex::new(r"@(?:[\w.]+\.)?([A-Za-z_][A-Za-z0-9_]*)")
        .expect("valid Java annotation name regex");
    annotation_re
        .captures_iter(text)
        .filter_map(|captures| captures.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

fn merged_annotations(left: &[String], right: &[String]) -> Vec<String> {
    let mut values = BTreeSet::new();
    for value in left.iter().chain(right.iter()) {
        values.insert(value.clone());
    }
    values.into_iter().collect()
}

fn has_injection_annotation(annotations: &[String]) -> bool {
    annotations.iter().any(|annotation| {
        matches!(
            annotation.as_str(),
            "Autowired" | "Resource" | "Inject" | "DubboReference"
        )
    })
}

fn resolve_qualifier_type(
    source: &Capability,
    qualifier: &str,
    variable_types: &BTreeMap<String, String>,
) -> Option<String> {
    if qualifier == "this" {
        return source.class.as_deref().map(simple_type).map(str::to_string);
    }
    if starts_uppercase(qualifier) {
        return Some(simple_type(qualifier).to_string());
    }
    variable_types.get(qualifier).cloned()
}

fn resolve_call_target<'a>(
    all_caps: &[&'a Capability],
    source: &Capability,
    target_class: &str,
    method: &str,
    implementation_index: &BTreeMap<String, BTreeSet<String>>,
) -> JavaCallResolutionResult<'a> {
    let class_matches = all_caps
        .iter()
        .copied()
        .filter(|target| target.id != source.id)
        .filter(|target| target.method == method)
        .filter(|target| {
            target
                .class
                .as_deref()
                .is_some_and(|class| simple_type(class) == target_class)
        })
        .collect::<Vec<_>>();
    if class_matches.len() == 1 {
        return JavaCallResolutionResult::Resolved(JavaCallResolution {
            target: class_matches[0],
            confidence: "high",
            resolution: "declaring_class",
            resolved_class: class_matches[0].class.clone(),
        });
    } else if class_matches.len() > 1 {
        return JavaCallResolutionResult::Ambiguous {
            reason: "ambiguous_declaring_class_method",
            candidates: class_matches,
        };
    }

    let implemented_classes = implementation_index.get(target_class);
    let implementation_matches = all_caps
        .iter()
        .copied()
        .filter(|target| target.id != source.id)
        .filter(|target| target.method == method)
        .filter(|target| {
            let Some(class) = target.class.as_deref() else {
                return false;
            };
            implemented_classes.is_some_and(|classes| classes.contains(simple_type(class)))
        })
        .collect::<Vec<_>>();
    if implementation_matches.len() == 1 {
        return JavaCallResolutionResult::Resolved(JavaCallResolution {
            target: implementation_matches[0],
            confidence: "high",
            resolution: "implements_interface",
            resolved_class: implementation_matches[0].class.clone(),
        });
    } else if implementation_matches.len() > 1 {
        return JavaCallResolutionResult::Ambiguous {
            reason: "ambiguous_interface_implementations",
            candidates: implementation_matches,
        };
    }

    let method_matches = all_caps
        .iter()
        .copied()
        .filter(|target| target.id != source.id)
        .filter(|target| target.method == method)
        .collect::<Vec<_>>();
    if method_matches.len() == 1 {
        JavaCallResolutionResult::Resolved(JavaCallResolution {
            target: method_matches[0],
            confidence: "medium",
            resolution: "unique_method_name",
            resolved_class: method_matches[0].class.clone(),
        })
    } else if method_matches.len() > 1 {
        JavaCallResolutionResult::Ambiguous {
            reason: "ambiguous_method_name",
            candidates: method_matches,
        }
    } else {
        JavaCallResolutionResult::Unresolved
    }
}

fn resolve_dependency_target<'a>(
    all_caps: &[&'a Capability],
    source: &Capability,
    target_class: &str,
    implementation_index: &BTreeMap<String, BTreeSet<String>>,
) -> JavaDependencyResolutionResult<'a> {
    let class_matches = all_caps
        .iter()
        .copied()
        .filter(|target| target.id != source.id)
        .filter(|target| {
            target
                .class
                .as_deref()
                .is_some_and(|class| simple_type(class) == target_class)
        })
        .collect::<Vec<_>>();
    if !class_matches.is_empty() {
        let files = class_matches
            .iter()
            .map(|cap| cap.file.as_str())
            .collect::<BTreeSet<_>>();
        if files.len() == 1 {
            let target = representative_capability(&class_matches);
            let target_class = target
                .class
                .as_deref()
                .map(simple_type)
                .unwrap_or(target_class);
            return JavaDependencyResolutionResult::Resolved(JavaDependencyResolution {
                target,
                target_class: target_class.to_string(),
                confidence: "high",
                resolution: "declaring_class",
            });
        }
        return JavaDependencyResolutionResult::Ambiguous {
            reason: "ambiguous_declaring_class",
            candidates: class_matches,
        };
    }

    let Some(implemented_classes) = implementation_index.get(target_class) else {
        return JavaDependencyResolutionResult::Unresolved;
    };
    let implementation_matches = all_caps
        .iter()
        .copied()
        .filter(|target| target.id != source.id)
        .filter(|target| {
            target
                .class
                .as_deref()
                .is_some_and(|class| implemented_classes.contains(simple_type(class)))
        })
        .collect::<Vec<_>>();
    if implementation_matches.is_empty() {
        JavaDependencyResolutionResult::Unresolved
    } else if implemented_classes.len() == 1 {
        let target = representative_capability(&implementation_matches);
        JavaDependencyResolutionResult::Resolved(JavaDependencyResolution {
            target,
            target_class: target
                .class
                .as_deref()
                .map(simple_type)
                .unwrap_or(target_class)
                .to_string(),
            confidence: "high",
            resolution: "implements_interface",
        })
    } else {
        JavaDependencyResolutionResult::Ambiguous {
            reason: "ambiguous_interface_implementations",
            candidates: implementation_matches,
        }
    }
}

fn representative_capability<'a>(caps: &[&'a Capability]) -> &'a Capability {
    caps.iter()
        .copied()
        .min_by(|a, b| {
            type_rank(ai_context::candidate_type(a))
                .cmp(&type_rank(ai_context::candidate_type(b)))
                .then(a.method.cmp(&b.method))
                .then(a.id.cmp(&b.id))
        })
        .expect("representative capability requires non-empty candidates")
}

fn call_relationship(target: &Capability) -> &'static str {
    if ai_context::is_external(target) {
        "calls_external_api"
    } else if target.rpc.is_some() || has_framework(target, "framework:dubbo_service") {
        "calls_rpc"
    } else if has_framework(target, "framework:feign_client") {
        "calls_http_client"
    } else if has_framework(target, "framework:mybatis_xml") {
        "calls_data_mapper"
    } else {
        "calls"
    }
}

fn semantic_call_relationships(edge: &JavaCallEdge<'_>) -> Vec<&'static str> {
    let source_type = ai_context::candidate_type(edge.source);
    let target_type = ai_context::candidate_type(edge.target);
    let mut relationships = Vec::new();
    if matches!(source_type, "http_endpoint" | "rpc_method")
        && !ai_context::is_external(edge.target)
    {
        relationships.push("exposes");
    }
    if source_type == "service_method"
        && (ai_context::is_external(edge.target)
            || matches!(target_type, "http_endpoint" | "rpc_method" | "dao_method")
            || framework_name(edge.target).is_some())
    {
        relationships.push("wraps");
    }
    relationships
}

fn semantic_call_evidence(edge: &JavaCallEdge<'_>, relationship: &str) -> Value {
    let mut evidence = edge.evidence.clone();
    evidence["kind"] = json!("java_semantic_call");
    evidence["derived_from"] = json!(edge.relationship);
    evidence["semantic_relationship"] = json!(relationship);
    evidence
}

fn is_java_project_cap(cap: &Capability) -> bool {
    cap.lang.as_str() == "Java" && !ai_context::is_external(cap) && cap.byte_range.1 > 0
}

fn simple_type(raw: &str) -> &str {
    let raw = raw.trim();
    let raw = raw.split('<').next().unwrap_or(raw);
    let raw = raw.trim_end_matches("[]");
    raw.rsplit(['.', '$']).next().unwrap_or(raw)
}

fn starts_uppercase(value: &str) -> bool {
    value
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_uppercase())
}

fn byte_offset_to_line(source_text: &str, byte_offset: usize) -> u32 {
    let end = byte_offset.min(source_text.len());
    source_text[..end].matches('\n').count() as u32 + 1
}

fn push_node(nodes: &mut Vec<Value>, seen: &mut BTreeSet<String>, node: Value) {
    let Some(id) = node.get("id").and_then(Value::as_str) else {
        return;
    };
    if seen.insert(id.to_string()) {
        nodes.push(node);
    }
}

fn entrypoint_json(cap: &Capability) -> Option<Value> {
    if let Some(ref http) = cap.http {
        Some(json!({
            "kind": "http",
            "value": format!("{} {}", http.method, http.path),
            "id": cap.id,
            "file": cap.file,
            "line": cap.line,
        }))
    } else {
        cap.rpc.as_ref().map(|rpc| {
            json!({
                "kind": "rpc",
                "value": format!("{}.{}", rpc.service, rpc.rpc),
                "id": cap.id,
                "file": cap.file,
                "line": cap.line,
            })
        })
    }
}

fn framework_name(cap: &Capability) -> Option<&'static str> {
    if has_framework(cap, "framework:feign_client") {
        Some("feign_client")
    } else if has_framework(cap, "framework:dubbo_service") {
        Some("dubbo_service")
    } else if has_framework(cap, "framework:mybatis_xml") {
        Some("mybatis_xml")
    } else if has_framework(cap, "framework:jax_rs") {
        Some("jax_rs")
    } else {
        None
    }
}

fn has_framework(cap: &Capability, marker: &str) -> bool {
    cap.tags.iter().any(|tag| tag == marker) || cap.annotations.iter().any(|ann| ann == marker)
}

fn ownership_for_cap(cap: &Capability, ownership: &OwnershipCatalog) -> OwnershipInfo {
    if ai_context::is_external(cap) {
        if let Some((matched, owner)) = longest_external_match(cap, &ownership.external) {
            return OwnershipInfo {
                owner: owner.to_string(),
                source: "config.external",
                matched: Some(matched.to_string()),
                kind: "external_owner",
            };
        }
        return OwnershipInfo {
            owner: cap
                .class
                .as_deref()
                .or_else(|| non_empty(&cap.package))
                .unwrap_or("external")
                .to_string(),
            source: "external_symbol",
            matched: None,
            kind: "external_reference",
        };
    }

    if let Some((matched, owner)) = longest_prefix_match(&ownership.packages, &cap.package) {
        return OwnershipInfo {
            owner: owner.to_string(),
            source: "config.packages",
            matched: Some(matched.to_string()),
            kind: "package_owner",
        };
    }

    let module = module_name(cap);
    if let Some((matched, owner)) = longest_prefix_match(&ownership.modules, module) {
        return OwnershipInfo {
            owner: owner.to_string(),
            source: "config.modules",
            matched: Some(matched.to_string()),
            kind: "module_owner",
        };
    }

    OwnershipInfo {
        owner: cap
            .class
            .as_deref()
            .or_else(|| non_empty(&cap.package))
            .unwrap_or(module)
            .to_string(),
        source: if cap.class.is_some() {
            "class"
        } else if !cap.package.is_empty() {
            "package"
        } else {
            "module_path"
        },
        matched: None,
        kind: "inferred_owner",
    }
}

fn ownership_for_module(module: &str, ownership: &OwnershipCatalog) -> OwnershipInfo {
    if let Some((matched, owner)) = longest_prefix_match(&ownership.modules, module) {
        OwnershipInfo {
            owner: owner.to_string(),
            source: "config.modules",
            matched: Some(matched.to_string()),
            kind: "module_owner",
        }
    } else {
        OwnershipInfo {
            owner: module.to_string(),
            source: "module_path",
            matched: None,
            kind: "inferred_module",
        }
    }
}

fn ownership_for_package(
    package: &str,
    module: &str,
    ownership: &OwnershipCatalog,
) -> OwnershipInfo {
    if let Some((matched, owner)) = longest_prefix_match(&ownership.packages, package) {
        OwnershipInfo {
            owner: owner.to_string(),
            source: "config.packages",
            matched: Some(matched.to_string()),
            kind: "package_owner",
        }
    } else {
        ownership_for_module(module, ownership)
    }
}

fn service_registry_summary(caps: &[&Capability], ownership: &OwnershipCatalog) -> Value {
    let mut rows = ownership
        .services
        .iter()
        .map(|(id, service)| {
            let mut capability_count = 0u64;
            let mut entrypoint_count = 0u64;
            let mut external_assets = 0u64;
            for cap in caps {
                if service_matches_cap(service, cap) {
                    capability_count += 1;
                    if cap.http.is_some() || cap.rpc.is_some() {
                        entrypoint_count += 1;
                    }
                    if ai_context::is_external(cap) {
                        external_assets += 1;
                    }
                }
            }
            let mut item = service_config_json(id, service, "config.services", None);
            item["capability_count"] = json!(capability_count);
            item["entrypoint_count"] = json!(entrypoint_count);
            item["external_assets"] = json!(external_assets);
            item
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b["capability_count"]
            .as_u64()
            .cmp(&a["capability_count"].as_u64())
            .then(a["id"].as_str().cmp(&b["id"].as_str()))
    });
    json!({
        "configured": ownership.services.len(),
        "items": rows,
    })
}

fn service_for_cap(cap: &Capability, ownership: &OwnershipCatalog) -> Value {
    if ai_context::is_external(cap) {
        return Value::Null;
    }
    service_match_json(module_name(cap), &cap.package, ownership).unwrap_or(Value::Null)
}

fn service_for_module(module: &str, ownership: &OwnershipCatalog) -> Value {
    service_match_json(module, "", ownership).unwrap_or(Value::Null)
}

fn service_for_package(package: &str, module: &str, ownership: &OwnershipCatalog) -> Value {
    service_match_json(module, package, ownership).unwrap_or(Value::Null)
}

fn service_match_json(module: &str, package: &str, ownership: &OwnershipCatalog) -> Option<Value> {
    let mut best = None::<(usize, String, Value)>;
    for (id, service) in &ownership.services {
        if let Some(prefix) = service.module.as_deref() {
            if module_prefix_matches(module, prefix) {
                let score = prefix.len() + 1_000;
                let value = service_config_json(
                    id,
                    service,
                    "config.services.module",
                    Some(format!("module:{prefix}")),
                );
                let should_replace = match best.as_ref() {
                    Some((best_score, _, _)) => score > *best_score,
                    None => true,
                };
                if should_replace {
                    best = Some((score, id.clone(), value));
                }
            }
        }
        if let Some(prefix) = service.package.as_deref() {
            if package_prefix_matches(package, prefix) {
                let score = prefix.len();
                let value = service_config_json(
                    id,
                    service,
                    "config.services.package",
                    Some(format!("package:{prefix}")),
                );
                let should_replace = match best.as_ref() {
                    Some((best_score, _, _)) => score > *best_score,
                    None => true,
                };
                if should_replace {
                    best = Some((score, id.clone(), value));
                }
            }
        }
    }
    best.map(|(_, _, value)| value)
}

fn service_config_json(
    id: &str,
    service: &config::ServiceConfig,
    source: &str,
    matched: Option<String>,
) -> Value {
    json!({
        "id": id,
        "name": service.name.as_deref().unwrap_or(id),
        "module": service.module.as_deref(),
        "package": service.package.as_deref(),
        "owner": service.owner.as_deref(),
        "tier": service.tier.as_deref(),
        "slo": service.slo.as_deref(),
        "runbook": service.runbook.as_deref(),
        "tags": &service.tags,
        "source": source,
        "matched": matched,
    })
}

fn service_matches_cap(service: &config::ServiceConfig, cap: &Capability) -> bool {
    service
        .module
        .as_deref()
        .is_some_and(|prefix| module_prefix_matches(module_name(cap), prefix))
        || service
            .package
            .as_deref()
            .is_some_and(|prefix| package_prefix_matches(&cap.package, prefix))
}

fn module_prefix_matches(module: &str, prefix: &str) -> bool {
    module == prefix
        || module
            .strip_prefix(prefix)
            .is_some_and(|tail| tail.starts_with('/') || tail.is_empty())
}

fn package_prefix_matches(package: &str, prefix: &str) -> bool {
    package == prefix
        || package
            .strip_prefix(prefix)
            .is_some_and(|tail| tail.starts_with('.') || tail.is_empty())
}

fn longest_prefix_match<'a>(
    rules: &'a BTreeMap<String, String>,
    value: &str,
) -> Option<(&'a str, &'a str)> {
    rules
        .iter()
        .filter(|(pattern, _)| prefix_matches(value, pattern))
        .max_by_key(|(pattern, _)| pattern.len())
        .map(|(pattern, owner)| (pattern.as_str(), owner.as_str()))
}

fn longest_external_match<'a>(
    cap: &Capability,
    rules: &'a BTreeMap<String, String>,
) -> Option<(&'a str, &'a str)> {
    let class = cap.class.as_deref().unwrap_or_default();
    rules
        .iter()
        .filter(|(pattern, _)| {
            prefix_matches(&cap.package, pattern)
                || prefix_matches(class, pattern)
                || cap.signature.contains(pattern.as_str())
        })
        .max_by_key(|(pattern, _)| pattern.len())
        .map(|(pattern, owner)| (pattern.as_str(), owner.as_str()))
}

fn prefix_matches(value: &str, pattern: &str) -> bool {
    !pattern.is_empty()
        && (value == pattern
            || value
                .strip_prefix(pattern)
                .is_some_and(|rest| rest.starts_with('/') || rest.starts_with('.')))
}

fn ownership_summary(caps: &[&Capability], ownership: &OwnershipCatalog) -> Value {
    let mut by_owner = BTreeMap::<String, OwnerStats>::new();
    for cap in caps {
        let info = ownership_for_cap(cap, ownership);
        let entry = by_owner.entry(info.owner.clone()).or_default();
        entry.assets += 1;
        if ai_context::is_external(cap) {
            entry.external_assets += 1;
        }
        if entry.source.is_none() {
            entry.source = Some(info.source.to_string());
        }
        if entry.kind.is_none() {
            entry.kind = Some(info.kind.to_string());
        }
        entry.modules.insert(module_name(cap).to_string());
        if !cap.package.is_empty() {
            entry.packages.insert(cap.package.clone());
        }
    }

    let mut owners = by_owner
        .into_iter()
        .map(|(owner, stat)| {
            json!({
                "owner": owner,
                "assets": stat.assets,
                "external_assets": stat.external_assets,
                "source": stat.source,
                "kind": stat.kind,
                "modules": set_values(&stat.modules),
                "packages": set_values(&stat.packages),
            })
        })
        .collect::<Vec<_>>();
    owners.sort_by(|a, b| {
        b["assets"]
            .as_u64()
            .cmp(&a["assets"].as_u64())
            .then(a["owner"].as_str().cmp(&b["owner"].as_str()))
    });

    json!({
        "configured": {
            "modules": ownership.modules.len(),
            "packages": ownership.packages.len(),
            "external": ownership.external.len(),
        },
        "owner_count": owners.len(),
        "owners": owners,
    })
}

#[derive(Default)]
struct OwnerStats {
    assets: u64,
    external_assets: u64,
    source: Option<String>,
    kind: Option<String>,
    modules: BTreeSet<String>,
    packages: BTreeSet<String>,
}

struct GraphBuildContext<'a> {
    module_counts: &'a BTreeMap<String, u64>,
    package_counts: &'a BTreeMap<String, u64>,
    class_counts: &'a BTreeMap<String, u64>,
    ownership: &'a OwnershipCatalog,
}

fn external_ownership_summary(caps: &[&Capability], ownership: &OwnershipCatalog) -> Value {
    let mut by_owner = BTreeMap::<String, u64>::new();
    for cap in caps.iter().filter(|cap| ai_context::is_external(cap)) {
        let info = ownership_for_cap(cap, ownership);
        *by_owner.entry(info.owner).or_default() += 1;
    }
    let mut owners = by_owner
        .into_iter()
        .map(|(owner, count)| json!({"owner": owner, "count": count}))
        .collect::<Vec<_>>();
    owners.sort_by(|a, b| {
        b["count"]
            .as_u64()
            .cmp(&a["count"].as_u64())
            .then(a["owner"].as_str().cmp(&b["owner"].as_str()))
    });
    json!({
        "configured_rules": ownership.external.len(),
        "owners": owners,
    })
}

fn set_values<T: ToString>(values: &BTreeSet<T>) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

fn cap_ref_json(cap: &Capability) -> Value {
    json!({
        "id": cap.id,
        "type": ai_context::candidate_type(cap),
        "name": display_name(cap),
        "owner": cap.class.as_deref().unwrap_or(&cap.package),
        "file": cap.file,
        "line": cap.line,
    })
}

fn cap_ref_json_with_ownership(cap: &Capability, ownership: &OwnershipCatalog) -> Value {
    let mut value = cap_ref_json(cap);
    value["ownership"] = ownership_for_cap(cap, ownership).to_json();
    value
}

fn candidate_refs(candidates: &[&Capability]) -> Vec<Value> {
    let mut values = candidates
        .iter()
        .copied()
        .map(cap_ref_json)
        .collect::<Vec<_>>();
    values.sort_by(|a, b| {
        a["type"]
            .as_str()
            .cmp(&b["type"].as_str())
            .then(a["name"].as_str().cmp(&b["name"].as_str()))
            .then(a["file"].as_str().cmp(&b["file"].as_str()))
            .then(a["line"].as_u64().cmp(&b["line"].as_u64()))
    });
    values.truncate(8);
    values
}

fn display_name(cap: &Capability) -> String {
    if let Some(ref http) = cap.http {
        return format!("{} {}", http.method, http.path);
    }
    if let Some(ref rpc) = cap.rpc {
        return format!("{}.{}", rpc.service, rpc.rpc);
    }
    cap.qualified()
}

fn module_name(cap: &Capability) -> &str {
    if cap.module.is_empty() {
        "(root)"
    } else {
        &cap.module
    }
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn distinct_count<'a>(values: impl Iterator<Item = &'a str>) -> usize {
    values.collect::<BTreeSet<_>>().len()
}

fn count_strings<'a>(values: impl Iterator<Item = &'a str>) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for value in values {
        *counts.entry(value.to_string()).or_insert(0) += 1;
    }
    counts
}

fn top_counts(counts: BTreeMap<String, u64>, limit: usize) -> Vec<Value> {
    let mut counts = counts.into_iter().collect::<Vec<_>>();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    counts
        .into_iter()
        .take(limit)
        .map(|(name, count)| json!({"name": name, "count": count}))
        .collect()
}

fn type_rank(ctype: &str) -> u8 {
    match ctype {
        "http_endpoint" => 0,
        "rpc_method" => 1,
        "service_method" => 2,
        "dao_method" => 3,
        "internal_capability" => 4,
        "external_api_usage" => 5,
        "external_dependency" => 6,
        "jar_method" | "sources_jar_method" | "javadoc_method" => 7,
        "jar_class" => 8,
        _ => 9,
    }
}
