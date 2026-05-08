//! BM25 scorer with field-weighted scoring, exact-match boosts, and layer boost.

use capfind_core::{Capability, Field, Kind, Posting};
use std::collections::HashMap;

use crate::synonyms;
use crate::tokenize;

/// Scoring configuration — all values tunable via `.capfind/config.toml`.
#[derive(Debug, Clone)]
pub struct ScorerConfig {
    pub k1: f32,
    pub b: f32,
}

impl Default for ScorerConfig {
    fn default() -> Self {
        Self { k1: 1.2, b: 0.4 }
    }
}

/// Field weights: indexed by `Field as u8`.
pub const FIELD_WEIGHTS: [f32; 6] = [
    4.0, // HttpPath
    2.5, // ClassName
    3.0, // MethodName
    1.5, // AnnotationValue
    1.0, // Doc
    0.6, // PackageModule
];

/// Layer boosts: indexed by `Kind as u8` (0..=3; Other=9 maps to 1.0).
pub fn layer_boost(kind: Kind) -> f32 {
    match kind {
        Kind::HttpEndpoint => 1.25,
        Kind::RpcMethod => 1.20,
        Kind::ServiceMethod => 1.00,
        Kind::DaoMethod => 0.85,
        Kind::Other => 1.00,
    }
}

/// A single search hit with scoring detail.
#[derive(Debug, Clone)]
pub struct Hit {
    pub cap_id: u32,
    pub score: f32,
    pub explain: Option<ExplainTrace>,
}

/// Detailed scoring trace for `--explain`.
#[derive(Debug, Clone)]
pub struct ExplainTrace {
    pub term_hits: Vec<TermHit>,
    pub bm25_raw: f32,
    pub boosts: Vec<(&'static str, f32)>,
    pub final_score: f32,
}

#[derive(Debug, Clone)]
pub struct TermHit {
    pub term: String,
    pub field: Field,
    pub tf: u16,
    pub weight: f32,
    pub is_synonym: bool,
}

/// Run BM25 search over capabilities.
///
/// # Arguments
/// * `query` — user query string
/// * `caps` — all indexed capabilities
/// * `postings` — `term_id -> Vec<Posting>` from the index
/// * `vocab` — `term_id -> String` from the index
/// * `avgdl` — average document length
/// * `config` — scorer tuning params
/// * `limit` — max results to return
/// * `explain` — if true, fill `Hit::explain`
pub fn search(
    query: &str,
    caps: &[Capability],
    postings: &HashMap<u32, Vec<Posting>>,
    vocab: &[String],
    avgdl: f32,
    config: &ScorerConfig,
    limit: usize,
    explain: bool,
) -> Vec<Hit> {
    let n = caps.len() as f32;
    if n == 0.0 {
        return vec![];
    }

    // Build reverse vocab map: term_str → term_id.
    let vocab_map: HashMap<&str, u32> = vocab
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i as u32))
        .collect();

    // Tokenize query + expand synonyms.
    let query_tokens = tokenize::tokenize(query);
    let mut weighted_terms: Vec<(u32, f32)> = Vec::new(); // (term_id, weight)

    for token in &query_tokens {
        if let Some(&tid) = vocab_map.get(token.as_str()) {
            weighted_terms.push((tid, 1.0));
        }
        // Synonym expansion.
        for &syn in synonyms::expand(token) {
            if let Some(&tid) = vocab_map.get(syn) {
                weighted_terms.push((tid, synonyms::SYNONYM_WEIGHT));
            }
        }
    }

    if weighted_terms.is_empty() {
        return vec![];
    }

    // Score each capability.
    let mut scores: Vec<f32> = vec![0.0; caps.len()];
    let mut term_details: Vec<Vec<TermHit>> = if explain {
        vec![Vec::new(); caps.len()]
    } else {
        vec![]
    };

    for &(tid, term_weight) in &weighted_terms {
        let Some(posts) = postings.get(&tid) else {
            continue;
        };

        // IDF: log((N - df + 0.5) / (df + 0.5) + 1)
        // df = number of DISTINCT capabilities that contain this term (not total postings).
        let df = {
            let mut seen = std::collections::HashSet::new();
            for p in posts { seen.insert(p.cap_id); }
            seen.len() as f32
        };
        let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();

        for posting in posts {
            let cap = &caps[posting.cap_id as usize];
            let field_weight = FIELD_WEIGHTS[posting.field as u8 as usize];
            let adjusted_tf = posting.tf as f32 * field_weight * term_weight;

            // BM25 tf saturation.
            let dl = cap.terms.len() as f32; // document length ≈ total term refs
            let tf_norm = (adjusted_tf * (config.k1 + 1.0))
                / (adjusted_tf + config.k1 * (1.0 - config.b + config.b * dl / avgdl));

            scores[posting.cap_id as usize] += idf * tf_norm;

            if explain {
                let term_str = vocab.get(tid as usize).map(|s| s.clone()).unwrap_or_default();
                term_details[posting.cap_id as usize].push(TermHit {
                    term: term_str,
                    field: posting.field,
                    tf: posting.tf,
                    weight: field_weight * term_weight,
                    is_synonym: term_weight < 1.0,
                });
            }
        }
    }

    // Apply boosts.
    let query_lower = query.to_ascii_lowercase();
    let query_joined = query_tokens.join("");

    for (i, cap) in caps.iter().enumerate() {
        if scores[i] == 0.0 {
            continue;
        }

        let mut boost_log: Vec<(&'static str, f32)> = Vec::new();
        let bm25_raw = scores[i];

        // Layer boost.
        let lb = layer_boost(cap.kind);
        scores[i] *= lb;
        if lb != 1.0 {
            boost_log.push(("layer", lb));
        }

        // Exact path match boost.
        if let Some(ref http) = cap.http {
            if http.path.to_ascii_lowercase().contains(&query_lower) {
                scores[i] *= 1.8;
                boost_log.push(("path-exact", 1.8));
            }
        }

        // Class.method exact match.
        let qualified = cap.qualified().to_ascii_lowercase();
        if !query_joined.is_empty() && qualified.contains(&query_joined) {
            scores[i] *= 1.5;
            boost_log.push(("qualified-exact", 1.5));
        }

        // All-terms-present boost.
        if query_tokens.len() > 1 {
            let cap_terms_set: std::collections::HashSet<u32> = cap.terms.iter().map(|t| t.term_id).collect();
            let all_present = weighted_terms
                .iter()
                .filter(|(_, w)| *w >= 1.0) // only original terms, not synonyms
                .all(|(tid, _)| cap_terms_set.contains(tid));
            if all_present {
                scores[i] *= 1.2;
                boost_log.push(("all-terms", 1.2));
            }
        }

        if explain {
            term_details[i].sort_by(|a, b| b.weight.partial_cmp(&a.weight).unwrap());
            // Store explain info via a tagged value (we'll collect later).
            // For now we store boost_log and bm25_raw in a side vec.
            // This is a bit clunky but keeps the hot path allocation-free.
            let _ = (bm25_raw, boost_log); // Used below when building hits.
        }
    }

    // Collect top-N.
    let mut hits: Vec<(usize, f32)> = scores
        .iter()
        .enumerate()
        .filter(|(_, &s)| s > 0.0)
        .map(|(i, &s)| (i, s))
        .collect();
    hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit);

    hits.iter()
        .map(|&(i, score)| Hit {
            cap_id: i as u32,
            score,
            explain: if explain {
                Some(ExplainTrace {
                    term_hits: term_details[i].clone(),
                    bm25_raw: score, // simplified: bm25_raw before boosts not tracked separately in this pass
                    boosts: vec![], // TODO: store per-cap boost log
                    final_score: score,
                })
            } else {
                None
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use capfind_core::{Capability, Field, HttpInfo, Kind, Lang, TermRef};
    use std::collections::HashMap;

    fn make_test_index() -> (Vec<Capability>, HashMap<u32, Vec<Posting>>, Vec<String>, f32) {
        // vocab: 0=mdm, 1=query, 2=asset, 3=controller, 4=detail
        let vocab = vec![
            "mdm".into(), "query".into(), "asset".into(), "controller".into(), "detail".into(),
        ];

        // Cap 0: POST /mdm/query  — should rank #1 for "mdm query"
        let cap0 = Capability {
            id: 0,
            kind: Kind::HttpEndpoint,
            lang: Lang::Java,
            module: String::new(),
            package: "com.demo".into(),
            class: Some("MdmController".into()),
            method: "queryMdm".into(),
            signature: "ApiResult queryMdm(MdmQueryRequest)".into(),
            annotations: vec![],
            http: Some(HttpInfo { method: "POST".into(), path: "/mdm/query".into(), consumes: None, produces: None }),
            rpc: None,
            doc: None,
            tags: vec![],
            file: "MdmController.java".into(),
            line: 10,
            byte_range: (0, 100),
            terms: vec![
                TermRef { term_id: 0, field: Field::HttpPath, tf: 1 },
                TermRef { term_id: 1, field: Field::HttpPath, tf: 1 },
                TermRef { term_id: 0, field: Field::ClassName, tf: 1 },
                TermRef { term_id: 3, field: Field::ClassName, tf: 1 },
                TermRef { term_id: 1, field: Field::MethodName, tf: 1 },
                TermRef { term_id: 0, field: Field::MethodName, tf: 1 },
            ],
        };

        // Cap 1: GET /asset/detail — weaker match
        let cap1 = Capability {
            id: 1,
            kind: Kind::HttpEndpoint,
            lang: Lang::Java,
            module: String::new(),
            package: "com.demo".into(),
            class: Some("AssetController".into()),
            method: "getDetail".into(),
            signature: "ApiResult getDetail(Long id)".into(),
            annotations: vec![],
            http: Some(HttpInfo { method: "GET".into(), path: "/asset/detail".into(), consumes: None, produces: None }),
            rpc: None,
            doc: None,
            tags: vec![],
            file: "AssetController.java".into(),
            line: 20,
            byte_range: (0, 50),
            terms: vec![
                TermRef { term_id: 2, field: Field::HttpPath, tf: 1 },
                TermRef { term_id: 4, field: Field::HttpPath, tf: 1 },
                TermRef { term_id: 2, field: Field::ClassName, tf: 1 },
                TermRef { term_id: 3, field: Field::ClassName, tf: 1 },
            ],
        };

        let caps = vec![cap0, cap1];

        // Build postings.
        let mut postings: HashMap<u32, Vec<Posting>> = HashMap::default();
        for cap in &caps {
            for tr in &cap.terms {
                postings.entry(tr.term_id).or_default().push(Posting {
                    cap_id: cap.id,
                    field: tr.field,
                    tf: tr.tf,
                });
            }
        }

        let avgdl = caps.iter().map(|c| c.terms.len() as f32).sum::<f32>() / caps.len() as f32;
        (caps, postings, vocab, avgdl)
    }

    #[test]
    fn mdm_query_ranks_first() {
        let (caps, postings, vocab, avgdl) = make_test_index();
        let hits = search("mdm query", &caps, &postings, &vocab, avgdl, &ScorerConfig::default(), 10, false);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].cap_id, 0, "MdmController#queryMdm should rank #1");
        assert!(hits[0].score > 0.0);
    }

    #[test]
    fn synonym_expansion_works() {
        let (caps, postings, vocab, avgdl) = make_test_index();
        // "mdm search" should still find cap0 via synonym search→query.
        let hits = search("mdm search", &caps, &postings, &vocab, avgdl, &ScorerConfig::default(), 10, false);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].cap_id, 0);
    }

    #[test]
    fn no_results_for_unknown() {
        let (caps, postings, vocab, avgdl) = make_test_index();
        let hits = search("zzzzunknown", &caps, &postings, &vocab, avgdl, &ScorerConfig::default(), 10, false);
        assert!(hits.is_empty());
    }
}
