//! BM25 scorer with field-weighted scoring, exact-match boosts, and layer boost.

use capfind_core::{Capability, Field, Kind, Posting};
use std::collections::{HashMap, HashSet};

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
#[allow(clippy::too_many_arguments)]
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
    let query_intent = QueryIntent::detect(query, &query_tokens);
    let mut weighted_term_map: HashMap<u32, f32> = HashMap::new(); // term_id -> strongest weight

    for token in &query_tokens {
        if let Some(&tid) = vocab_map.get(token.as_str()) {
            add_weighted_term(&mut weighted_term_map, tid, 1.0);
        }
        // Synonym expansion.
        for &syn in synonyms::expand(token) {
            if let Some(&tid) = vocab_map.get(syn) {
                add_weighted_term(&mut weighted_term_map, tid, synonyms::SYNONYM_WEIGHT);
            }
        }
    }
    for expansion in synonyms::expand_phrases(query) {
        for term in expansion.terms {
            if let Some(&tid) = vocab_map.get(term.term) {
                add_weighted_term(&mut weighted_term_map, tid, term.weight);
            }
        }
    }
    let weighted_terms: Vec<(u32, f32)> = weighted_term_map.into_iter().collect();

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
    let mut bm25_raws: Vec<f32> = if explain {
        vec![0.0; caps.len()]
    } else {
        vec![]
    };
    let mut boost_details: Vec<Vec<(&'static str, f32)>> = if explain {
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
            let mut seen = HashSet::new();
            for p in posts {
                seen.insert(p.cap_id);
            }
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
                let term_str = vocab.get(tid as usize).cloned().unwrap_or_default();
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
            let cap_terms_set: HashSet<u32> = cap.terms.iter().map(|t| t.term_id).collect();
            let all_present = weighted_terms
                .iter()
                .filter(|(_, w)| *w >= 1.0) // only original terms, not synonyms
                .all(|(tid, _)| cap_terms_set.contains(tid));
            if all_present {
                scores[i] *= 1.2;
                boost_log.push(("all-terms", 1.2));
            }
        }

        apply_semantic_intent_boosts(&query_intent, cap, vocab, &mut scores[i], &mut boost_log);

        if explain {
            term_details[i].sort_by(|a, b| b.weight.partial_cmp(&a.weight).unwrap());
            bm25_raws[i] = bm25_raw;
            boost_details[i] = boost_log;
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
                    bm25_raw: bm25_raws[i],
                    boosts: boost_details[i].clone(),
                    final_score: score,
                })
            } else {
                None
            },
        })
        .collect()
}

fn add_weighted_term(weighted_terms: &mut HashMap<u32, f32>, tid: u32, weight: f32) {
    weighted_terms
        .entry(tid)
        .and_modify(|existing| {
            if weight > *existing {
                *existing = weight;
            }
        })
        .or_insert(weight);
}

#[derive(Debug, Default)]
struct QueryIntent {
    wants_account_identity: bool,
    wants_capability_lookup: bool,
    explicit_ocr_or_verification: bool,
}

impl QueryIntent {
    fn detect(query: &str, query_tokens: &[String]) -> Self {
        let normalized = query
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>();
        let lower = query.to_ascii_lowercase();
        let has_account = contains_any(
            &normalized,
            &["账号", "账户", "账户信息", "账号姓名", "主体名称"],
        ) || token_contains_any(
            query_tokens,
            &[
                "account",
                "accountname",
                "accountinfo",
                "accountlist",
                "relatedaccount",
            ],
        );
        let has_legal_or_subject = contains_any(&normalized, &["法人", "主体", "企业", "公司"])
            || token_contains_any(
                query_tokens,
                &["legalperson", "legalpersonid", "enterprise", "company"],
            );
        let has_identity = contains_any(&normalized, &["身份证", "身份证号", "证件"])
            || token_contains_any(query_tokens, &["idcard", "identity", "identityno"]);
        let has_name = contains_any(&normalized, &["姓名", "名称", "主体名称"])
            || token_contains_any(
                query_tokens,
                &["name", "accountname", "customername", "enterprisename"],
            );
        let wants_capability_lookup =
            contains_any(
                &normalized,
                &[
                    "现有代码",
                    "现有接口",
                    "可支持",
                    "支持的接口",
                    "可复用",
                    "复用",
                    "接口",
                    "能力",
                ],
            ) || token_contains_any(query_tokens, &["api", "endpoint", "capability", "reuse"]);
        let explicit_ocr_or_verification = contains_any(
            &normalized,
            &["识别", "验真", "校验", "核验", "二要素", "认证", "ocr"],
        ) || contains_any(
            &lower,
            &[
                "ocr",
                "verify",
                "verification",
                "validate",
                "validation",
                "authenticate",
                "authentication",
            ],
        );

        Self {
            wants_account_identity: has_account
                && (has_legal_or_subject || has_identity || has_name),
            wants_capability_lookup,
            explicit_ocr_or_verification,
        }
    }
}

fn apply_semantic_intent_boosts(
    intent: &QueryIntent,
    cap: &Capability,
    vocab: &[String],
    score: &mut f32,
    boost_log: &mut Vec<(&'static str, f32)>,
) {
    if !intent.wants_account_identity {
        return;
    }

    let terms = capability_term_set(cap, vocab);
    let has_account = term_set_contains_any(
        &terms,
        &[
            "account",
            "acct",
            "accountno",
            "accountnumber",
            "accountid",
            "accountname",
            "accountinfo",
            "accountlist",
            "relatedaccount",
            "relatedaccountlist",
        ],
    );
    let has_legal = term_set_contains_any(
        &terms,
        &[
            "legalperson",
            "legalpersonid",
            "legalpersonidcard",
            "legal",
            "person",
        ],
    );
    let has_name = term_set_contains_any(
        &terms,
        &[
            "name",
            "accountname",
            "customername",
            "enterprisename",
            "companyname",
            "personname",
            "subjectname",
        ],
    );
    let has_ocr_or_verify = term_set_contains_any(
        &terms,
        &[
            "ocr",
            "verify",
            "verification",
            "validate",
            "validation",
            "authenticate",
            "authentication",
            "certification",
        ],
    );

    if has_account && has_legal {
        *score *= 1.55;
        boost_log.push(("account-legal-intent", 1.55));
    } else if has_account {
        *score *= 1.25;
        boost_log.push(("account-intent", 1.25));
    }

    if has_account && has_name {
        *score *= 1.15;
        boost_log.push(("account-name-intent", 1.15));
    }

    if intent.wants_capability_lookup && matches!(cap.kind, Kind::ServiceMethod) && has_account {
        *score *= 1.25;
        boost_log.push(("service-reuse-intent", 1.25));
    }

    if !intent.explicit_ocr_or_verification && has_ocr_or_verify && !has_account {
        *score *= 0.55;
        boost_log.push(("ocr-verify-demotion", 0.55));
    }
}

fn capability_term_set<'a>(cap: &Capability, vocab: &'a [String]) -> HashSet<&'a str> {
    cap.terms
        .iter()
        .filter_map(|term| vocab.get(term.term_id as usize).map(String::as_str))
        .collect()
}

fn term_set_contains_any(terms: &HashSet<&str>, needles: &[&str]) -> bool {
    needles.iter().any(|needle| terms.contains(needle))
}

fn token_contains_any(tokens: &[String], needles: &[&str]) -> bool {
    tokens
        .iter()
        .any(|token| needles.iter().any(|needle| token == needle))
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use capfind_core::{Capability, Field, HttpInfo, Kind, Lang, TermRef};
    use std::collections::HashMap;

    fn make_test_index() -> (
        Vec<Capability>,
        HashMap<u32, Vec<Posting>>,
        Vec<String>,
        f32,
    ) {
        // vocab: 0=mdm, 1=query, 2=asset, 3=controller, 4=detail
        let vocab = vec![
            "mdm".into(),
            "query".into(),
            "asset".into(),
            "controller".into(),
            "detail".into(),
            "legalperson".into(),
            "legal".into(),
            "person".into(),
            "idcard".into(),
            "accountname".into(),
            "account".into(),
            "name".into(),
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
            http: Some(HttpInfo {
                method: "POST".into(),
                path: "/mdm/query".into(),
                consumes: None,
                produces: None,
            }),
            rpc: None,
            doc: None,
            tags: vec![],
            file: "MdmController.java".into(),
            line: 10,
            byte_range: (0, 100),
            terms: vec![
                TermRef {
                    term_id: 0,
                    field: Field::HttpPath,
                    tf: 1,
                },
                TermRef {
                    term_id: 1,
                    field: Field::HttpPath,
                    tf: 1,
                },
                TermRef {
                    term_id: 0,
                    field: Field::ClassName,
                    tf: 1,
                },
                TermRef {
                    term_id: 3,
                    field: Field::ClassName,
                    tf: 1,
                },
                TermRef {
                    term_id: 1,
                    field: Field::MethodName,
                    tf: 1,
                },
                TermRef {
                    term_id: 0,
                    field: Field::MethodName,
                    tf: 1,
                },
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
            http: Some(HttpInfo {
                method: "GET".into(),
                path: "/asset/detail".into(),
                consumes: None,
                produces: None,
            }),
            rpc: None,
            doc: None,
            tags: vec![],
            file: "AssetController.java".into(),
            line: 20,
            byte_range: (0, 50),
            terms: vec![
                TermRef {
                    term_id: 2,
                    field: Field::HttpPath,
                    tf: 1,
                },
                TermRef {
                    term_id: 4,
                    field: Field::HttpPath,
                    tf: 1,
                },
                TermRef {
                    term_id: 2,
                    field: Field::ClassName,
                    tf: 1,
                },
                TermRef {
                    term_id: 3,
                    field: Field::ClassName,
                    tf: 1,
                },
            ],
        };

        let cap2 = Capability {
            id: 2,
            kind: Kind::HttpEndpoint,
            lang: Lang::Java,
            module: String::new(),
            package: "com.demo".into(),
            class: Some("LegalPersonController".into()),
            method: "getLegalPersonIdCardAccountName".into(),
            signature: "LegalPersonAccountNameResponse getLegalPersonIdCardAccountName(LegalPersonIdCardQuery request)".into(),
            annotations: vec![],
            http: Some(HttpInfo {
                method: "GET".into(),
                path: "/legal-person/id-card/account-name".into(),
                consumes: None,
                produces: None,
            }),
            rpc: None,
            doc: None,
            tags: vec![],
            file: "LegalPersonController.java".into(),
            line: 30,
            byte_range: (0, 80),
            terms: vec![
                TermRef {
                    term_id: 5,
                    field: Field::HttpPath,
                    tf: 1,
                },
                TermRef {
                    term_id: 8,
                    field: Field::HttpPath,
                    tf: 1,
                },
                TermRef {
                    term_id: 9,
                    field: Field::HttpPath,
                    tf: 1,
                },
                TermRef {
                    term_id: 5,
                    field: Field::ClassName,
                    tf: 1,
                },
                TermRef {
                    term_id: 6,
                    field: Field::MethodName,
                    tf: 1,
                },
                TermRef {
                    term_id: 7,
                    field: Field::MethodName,
                    tf: 1,
                },
                TermRef {
                    term_id: 8,
                    field: Field::MethodName,
                    tf: 1,
                },
                TermRef {
                    term_id: 10,
                    field: Field::MethodName,
                    tf: 1,
                },
                TermRef {
                    term_id: 11,
                    field: Field::MethodName,
                    tf: 1,
                },
            ],
        };

        let caps = vec![cap0, cap1, cap2];

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

    fn build_index_from_caps(
        mut caps: Vec<Capability>,
    ) -> (
        Vec<Capability>,
        HashMap<u32, Vec<Posting>>,
        Vec<String>,
        f32,
    ) {
        let mut vocab: Vec<String> = Vec::new();
        let mut vocab_map: HashMap<String, u32> = HashMap::new();
        let mut postings: HashMap<u32, Vec<Posting>> = HashMap::default();

        for (i, cap) in caps.iter_mut().enumerate() {
            cap.id = i as u32;
            let field_tokens: Vec<(Field, Vec<String>)> = vec![
                (
                    Field::HttpPath,
                    cap.http
                        .as_ref()
                        .map(|h| tokenize::tokenize_path(&h.path))
                        .unwrap_or_default(),
                ),
                (
                    Field::ClassName,
                    cap.class
                        .as_ref()
                        .map(|class| tokenize::tokenize(class))
                        .unwrap_or_default(),
                ),
                (Field::MethodName, tokenize::tokenize(&cap.method)),
                (
                    Field::AnnotationValue,
                    cap.annotations
                        .iter()
                        .chain(cap.tags.iter())
                        .flat_map(|value| tokenize::tokenize(value))
                        .collect(),
                ),
                (
                    Field::Doc,
                    cap.doc
                        .as_ref()
                        .map(|doc| tokenize::tokenize(doc))
                        .unwrap_or_default(),
                ),
                (Field::PackageModule, {
                    let mut tokens = tokenize::tokenize(&cap.package);
                    tokens.extend(tokenize::tokenize(&cap.module));
                    tokens
                }),
            ];

            let mut terms = Vec::new();
            for (field, tokens) in field_tokens {
                let mut tf_map: HashMap<String, u16> = HashMap::default();
                for token in tokens {
                    *tf_map.entry(token).or_default() += 1;
                }
                for (token, tf) in tf_map {
                    let tid = *vocab_map.entry(token.clone()).or_insert_with(|| {
                        let id = vocab.len() as u32;
                        vocab.push(token);
                        id
                    });
                    terms.push(TermRef {
                        term_id: tid,
                        field,
                        tf,
                    });
                    postings.entry(tid).or_default().push(Posting {
                        cap_id: cap.id,
                        field,
                        tf,
                    });
                }
            }
            cap.terms = terms;
        }

        let avgdl = caps.iter().map(|cap| cap.terms.len() as f32).sum::<f32>() / caps.len() as f32;
        (caps, postings, vocab, avgdl)
    }

    fn java_capability(
        kind: Kind,
        class: &str,
        method: &str,
        signature: &str,
        http: Option<(&str, &str)>,
        doc: Option<&str>,
    ) -> Capability {
        Capability {
            id: 0,
            kind,
            lang: Lang::Java,
            module: String::new(),
            package: "com.demo".into(),
            class: Some(class.into()),
            method: method.into(),
            signature: signature.into(),
            annotations: vec![],
            http: http.map(|(method, path)| HttpInfo {
                method: method.into(),
                path: path.into(),
                consumes: None,
                produces: None,
            }),
            rpc: None,
            doc: doc.map(str::to_string),
            tags: vec![],
            file: format!("{class}.java"),
            line: 1,
            byte_range: (0, 80),
            terms: vec![],
        }
    }

    #[test]
    fn mdm_query_ranks_first() {
        let (caps, postings, vocab, avgdl) = make_test_index();
        let hits = search(
            "mdm query",
            &caps,
            &postings,
            &vocab,
            avgdl,
            &ScorerConfig::default(),
            10,
            false,
        );
        assert!(!hits.is_empty());
        assert_eq!(hits[0].cap_id, 0, "MdmController#queryMdm should rank #1");
        assert!(hits[0].score > 0.0);
    }

    #[test]
    fn synonym_expansion_works() {
        let (caps, postings, vocab, avgdl) = make_test_index();
        // "mdm search" should still find cap0 via synonym search→query.
        let hits = search(
            "mdm search",
            &caps,
            &postings,
            &vocab,
            avgdl,
            &ScorerConfig::default(),
            10,
            false,
        );
        assert!(!hits.is_empty());
        assert_eq!(hits[0].cap_id, 0);
    }

    #[test]
    fn chinese_business_query_expands_to_code_terms() {
        let (caps, postings, vocab, avgdl) = make_test_index();
        let hits = search(
            "获取法人身份证账号姓名",
            &caps,
            &postings,
            &vocab,
            avgdl,
            &ScorerConfig::default(),
            10,
            true,
        );
        assert!(!hits.is_empty());
        assert_eq!(hits[0].cap_id, 2);
        let explain = hits[0].explain.as_ref().unwrap();
        assert!(explain
            .term_hits
            .iter()
            .any(|hit| hit.term == "legalperson" && hit.is_synonym));
        assert!(explain
            .term_hits
            .iter()
            .any(|hit| hit.term == "accountname" && hit.is_synonym));
    }

    #[test]
    fn account_legal_person_intent_ranks_account_chain_over_ocr() {
        let (caps, postings, vocab, avgdl) = build_index_from_caps(vec![
            java_capability(
                Kind::HttpEndpoint,
                "IdCardController",
                "idCardOcrAndVerify",
                "IdCardVerifyResponse idCardOcrAndVerify(IdCardImageRequest request)",
                Some(("GET", "/idCard/idCardOcrAndVerify")),
                None,
            ),
            java_capability(
                Kind::HttpEndpoint,
                "IdCardController",
                "verifyNameAndNo",
                "VerifyResponse verifyNameAndNo(IdCardVerifyRequest request)",
                Some(("GET", "/idCard/verifyNameAndNo")),
                None,
            ),
            java_capability(
                Kind::ServiceMethod,
                "CustomerServiceWrapper",
                "selectAccountListByLegalPersonId",
                "List<AccountInfo> selectAccountListByLegalPersonId(String legalPersonId)",
                None,
                Some("returns AccountInfo legalPersonId accountName relatedAccountList"),
            ),
            java_capability(
                Kind::ServiceMethod,
                "AsyncCallService",
                "getLegalPersonIdAsync",
                "CompletableFuture<String> getLegalPersonIdAsync(AccountInfo accountInfo)",
                None,
                Some("loads legalPersonId accountName"),
            ),
            java_capability(
                Kind::HttpEndpoint,
                "CheckToolController",
                "getLogs",
                "List<AccountInfo> getLogs(AccountInfo query)",
                Some(("GET", "/checkTool/v1/getLogs")),
                Some("AccountInfo legalPersonId accountName relatedAccountList"),
            ),
        ]);
        let hits = search(
            "从现有代码里找到可支持的接口 获取法人身份证 账号 姓名",
            &caps,
            &postings,
            &vocab,
            avgdl,
            &ScorerConfig::default(),
            10,
            true,
        );

        assert!(!hits.is_empty());
        assert_eq!(
            caps[hits[0].cap_id as usize].method,
            "selectAccountListByLegalPersonId"
        );
        let account_rank = hits
            .iter()
            .position(|hit| caps[hit.cap_id as usize].method == "selectAccountListByLegalPersonId")
            .unwrap();
        let ocr_rank = hits
            .iter()
            .position(|hit| caps[hit.cap_id as usize].method == "idCardOcrAndVerify")
            .unwrap();
        let verify_rank = hits
            .iter()
            .position(|hit| caps[hit.cap_id as usize].method == "verifyNameAndNo")
            .unwrap();
        assert!(account_rank < ocr_rank);
        assert!(account_rank < verify_rank);

        let account_explain = hits[account_rank].explain.as_ref().unwrap();
        assert!(account_explain
            .boosts
            .iter()
            .any(|(name, _)| *name == "account-legal-intent"));
        assert!(account_explain
            .boosts
            .iter()
            .any(|(name, _)| *name == "service-reuse-intent"));

        let ocr_explain = hits[ocr_rank].explain.as_ref().unwrap();
        assert!(ocr_explain
            .boosts
            .iter()
            .any(|(name, _)| *name == "ocr-verify-demotion"));
    }

    #[test]
    fn no_results_for_unknown() {
        let (caps, postings, vocab, avgdl) = make_test_index();
        let hits = search(
            "zzzzunknown",
            &caps,
            &postings,
            &vocab,
            avgdl,
            &ScorerConfig::default(),
            10,
            false,
        );
        assert!(hits.is_empty());
    }

    #[test]
    fn explain_includes_raw_score_and_boosts() {
        let (caps, postings, vocab, avgdl) = make_test_index();
        let hits = search(
            "mdm query",
            &caps,
            &postings,
            &vocab,
            avgdl,
            &ScorerConfig::default(),
            10,
            true,
        );
        let explain = hits[0].explain.as_ref().unwrap();
        assert!(explain.bm25_raw > 0.0);
        assert!(explain.final_score >= explain.bm25_raw);
        assert!(explain.boosts.iter().any(|(name, _)| *name == "layer"));
        assert!(explain.term_hits.iter().any(|hit| hit.term == "mdm"));
    }
}
