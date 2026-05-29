//! Synonym expansion for query terms.
//!
//! Default synonym groups cover common coding verbs. Users extend via
//! `.capfind/config.toml`. Synonyms are only applied query-side (not during
//! indexing) to avoid postings bloat.

use once_cell::sync::Lazy;
use std::collections::{BTreeSet, HashMap};

use crate::tokenize;

#[derive(Debug, Clone, Copy)]
pub struct PhraseTerm {
    pub term: &'static str,
    pub weight: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct PhraseExpansion {
    pub phrase: &'static str,
    pub terms: &'static [PhraseTerm],
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynonymRule {
    pub trigger: String,
    pub normalized_trigger: String,
    pub trigger_tokens: Vec<String>,
    pub terms: Vec<String>,
    pub weight: f32,
}

const fn phrase_term(term: &'static str, weight: f32) -> PhraseTerm {
    PhraseTerm { term, weight }
}

impl SynonymRule {
    pub fn new(trigger: impl Into<String>, terms: Vec<String>, weight: f32) -> Self {
        let trigger = trigger.into();
        let normalized_trigger = normalize_phrase(&trigger);
        let trigger_tokens = tokenize::tokenize(&trigger);
        let terms = normalize_terms(terms);
        Self {
            trigger,
            normalized_trigger,
            trigger_tokens,
            terms,
            weight,
        }
    }

    pub fn matches_query(&self, query: &str, query_tokens: &[String]) -> bool {
        if self.terms.is_empty() {
            return false;
        }
        let normalized_query = normalize_phrase(query);
        if !self.normalized_trigger.is_empty()
            && normalized_query.contains(self.normalized_trigger.as_str())
        {
            return true;
        }
        self.trigger_tokens
            .iter()
            .any(|trigger| query_tokens.iter().any(|token| token == trigger))
    }
}

pub fn normalize_phrase(input: &str) -> String {
    input
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn normalize_terms(terms: Vec<String>) -> Vec<String> {
    let mut normalized = BTreeSet::new();
    for term in terms {
        let compact = normalize_phrase(&term);
        if compact.len() > 1 {
            normalized.insert(compact);
        }
        for token in tokenize::tokenize(&term) {
            normalized.insert(token);
        }
    }
    normalized.into_iter().collect()
}

/// The default synonym table.
static DEFAULT_SYNONYMS: Lazy<HashMap<&'static str, &'static [&'static str]>> = Lazy::new(|| {
    let mut m = HashMap::default();
    m.insert("query", &["search", "find", "list", "fetch", "lookup"][..]);
    m.insert("search", &["query", "find", "list", "fetch", "lookup"][..]);
    m.insert("find", &["query", "search", "list", "fetch", "lookup"][..]);
    m.insert("list", &["query", "search", "find", "fetch", "lookup"][..]);
    m.insert("fetch", &["query", "search", "find", "list", "lookup"][..]);
    m.insert("lookup", &["query", "search", "find", "list", "fetch"][..]);
    m.insert("create", &["add", "insert", "register", "save"][..]);
    m.insert("add", &["create", "insert", "register", "save"][..]);
    m.insert("insert", &["create", "add", "register", "save"][..]);
    m.insert("register", &["create", "add", "insert", "save"][..]);
    m.insert("save", &["create", "add", "insert", "register"][..]);
    m.insert("update", &["modify", "edit", "change"][..]);
    m.insert("modify", &["update", "edit", "change"][..]);
    m.insert("edit", &["update", "modify", "change"][..]);
    m.insert("change", &["update", "modify", "edit"][..]);
    m.insert("delete", &["remove", "erase", "del"][..]);
    m.insert("remove", &["delete", "erase", "del"][..]);
    m.insert("batch", &["bulk", "many"][..]);
    m.insert("bulk", &["batch", "many"][..]);
    m.insert(
        "idcard",
        &[
            "identity",
            "identityno",
            "id",
            "card",
            "cert",
            "certificate",
        ][..],
    );
    m.insert("identity", &["idcard", "identityno", "id", "card"][..]);
    m.insert(
        "account",
        &[
            "acct",
            "accountno",
            "accountnumber",
            "accountid",
            "accountname",
            "accountinfo",
            "accountlist",
            "relatedaccount",
            "relatedaccountlist",
        ][..],
    );
    m.insert(
        "name",
        &[
            "username",
            "realname",
            "accountname",
            "customername",
            "enterprisename",
            "personname",
        ][..],
    );
    m.insert(
        "legalperson",
        &[
            "legalpersonid",
            "legalpersonidcard",
            "legal",
            "person",
            "company",
            "corporation",
        ][..],
    );
    m
});

static DEFAULT_PHRASE_SYNONYMS: &[(&str, &[PhraseTerm])] = &[
    (
        "查询",
        &[
            phrase_term("query", 0.60),
            phrase_term("search", 0.55),
            phrase_term("find", 0.55),
            phrase_term("lookup", 0.55),
            phrase_term("fetch", 0.50),
            phrase_term("list", 0.45),
        ],
    ),
    (
        "查找",
        &[
            phrase_term("query", 0.45),
            phrase_term("search", 0.50),
            phrase_term("find", 0.60),
            phrase_term("lookup", 0.55),
        ],
    ),
    (
        "获取",
        &[
            phrase_term("fetch", 0.55),
            phrase_term("lookup", 0.50),
            phrase_term("read", 0.50),
            phrase_term("load", 0.50),
            phrase_term("select", 0.50),
            phrase_term("query", 0.30),
            phrase_term("search", 0.30),
        ],
    ),
    (
        "列表",
        &[
            phrase_term("list", 0.65),
            phrase_term("query", 0.45),
            phrase_term("search", 0.40),
        ],
    ),
    (
        "法人身份证",
        &[
            phrase_term("legalpersonid", 0.90),
            phrase_term("legalpersonidcard", 0.85),
            phrase_term("identityno", 0.75),
            phrase_term("legalperson", 0.60),
            phrase_term("idcard", 0.45),
        ],
    ),
    (
        "法人",
        &[
            phrase_term("legalperson", 0.75),
            phrase_term("legalpersonid", 0.70),
            phrase_term("legal", 0.45),
            phrase_term("person", 0.45),
            phrase_term("company", 0.35),
            phrase_term("corporation", 0.35),
        ],
    ),
    (
        "身份证号",
        &[
            phrase_term("identityno", 0.80),
            phrase_term("idcard", 0.65),
            phrase_term("identity", 0.55),
            phrase_term("id", 0.35),
        ],
    ),
    (
        "身份证",
        &[
            phrase_term("idcard", 0.60),
            phrase_term("identity", 0.45),
            phrase_term("identityno", 0.55),
            phrase_term("id", 0.30),
            phrase_term("card", 0.25),
            phrase_term("cert", 0.25),
            phrase_term("certificate", 0.25),
        ],
    ),
    (
        "证件",
        &[
            phrase_term("idcard", 0.50),
            phrase_term("identity", 0.45),
            phrase_term("identityno", 0.50),
            phrase_term("cert", 0.30),
            phrase_term("certificate", 0.30),
        ],
    ),
    (
        "账号姓名",
        &[
            phrase_term("accountname", 0.85),
            phrase_term("customername", 0.70),
            phrase_term("enterprisename", 0.70),
            phrase_term("account", 0.60),
            phrase_term("name", 0.50),
        ],
    ),
    (
        "账号",
        &[
            phrase_term("account", 0.75),
            phrase_term("acct", 0.55),
            phrase_term("accountno", 0.65),
            phrase_term("accountnumber", 0.65),
            phrase_term("accountid", 0.65),
            phrase_term("accountname", 0.70),
            phrase_term("accountinfo", 0.75),
            phrase_term("accountlist", 0.75),
            phrase_term("relatedaccount", 0.70),
            phrase_term("relatedaccountlist", 0.70),
        ],
    ),
    (
        "账户",
        &[
            phrase_term("account", 0.75),
            phrase_term("acct", 0.55),
            phrase_term("accountno", 0.65),
            phrase_term("accountnumber", 0.65),
            phrase_term("accountid", 0.65),
            phrase_term("accountname", 0.70),
            phrase_term("accountinfo", 0.75),
            phrase_term("accountlist", 0.75),
            phrase_term("relatedaccount", 0.70),
            phrase_term("relatedaccountlist", 0.70),
        ],
    ),
    (
        "账户信息",
        &[
            phrase_term("accountinfo", 0.85),
            phrase_term("account", 0.75),
            phrase_term("relatedaccount", 0.70),
            phrase_term("relatedaccountlist", 0.70),
        ],
    ),
    (
        "姓名",
        &[
            phrase_term("name", 0.55),
            phrase_term("accountname", 0.80),
            phrase_term("username", 0.45),
            phrase_term("realname", 0.45),
            phrase_term("personname", 0.60),
            phrase_term("customername", 0.70),
            phrase_term("enterprisename", 0.70),
        ],
    ),
    (
        "主体名称",
        &[
            phrase_term("subjectname", 0.85),
            phrase_term("customername", 0.80),
            phrase_term("enterprisename", 0.80),
            phrase_term("companyname", 0.75),
            phrase_term("accountname", 0.70),
            phrase_term("name", 0.55),
        ],
    ),
    (
        "名称",
        &[
            phrase_term("name", 0.50),
            phrase_term("accountname", 0.65),
            phrase_term("username", 0.40),
            phrase_term("realname", 0.40),
            phrase_term("customername", 0.65),
            phrase_term("enterprisename", 0.65),
        ],
    ),
    (
        "用户",
        &[
            phrase_term("user", 0.60),
            phrase_term("account", 0.55),
            phrase_term("member", 0.45),
        ],
    ),
    (
        "客户",
        &[
            phrase_term("customer", 0.70),
            phrase_term("customername", 0.60),
            phrase_term("client", 0.55),
            phrase_term("account", 0.50),
        ],
    ),
    (
        "手机",
        &[
            phrase_term("mobile", 0.70),
            phrase_term("phone", 0.65),
            phrase_term("telephone", 0.55),
            phrase_term("tel", 0.50),
        ],
    ),
    (
        "手机号",
        &[
            phrase_term("mobile", 0.75),
            phrase_term("phone", 0.70),
            phrase_term("telephone", 0.55),
            phrase_term("tel", 0.50),
        ],
    ),
    (
        "电话",
        &[
            phrase_term("phone", 0.70),
            phrase_term("telephone", 0.60),
            phrase_term("tel", 0.55),
            phrase_term("mobile", 0.55),
        ],
    ),
    (
        "地址",
        &[phrase_term("address", 0.75), phrase_term("addr", 0.60)],
    ),
    ("订单", &[phrase_term("order", 0.75)]),
    (
        "机构",
        &[
            phrase_term("org", 0.65),
            phrase_term("organization", 0.65),
            phrase_term("institution", 0.55),
        ],
    ),
    (
        "企业",
        &[
            phrase_term("enterprise", 0.75),
            phrase_term("enterprisename", 0.65),
            phrase_term("company", 0.60),
            phrase_term("corp", 0.55),
            phrase_term("corporation", 0.55),
        ],
    ),
    (
        "公司",
        &[
            phrase_term("company", 0.70),
            phrase_term("companyname", 0.65),
            phrase_term("corp", 0.55),
            phrase_term("corporation", 0.55),
        ],
    ),
];

/// Weight applied to synonym-expanded terms (< 1.0 to not overpower originals).
pub const SYNONYM_WEIGHT: f32 = 0.6;

pub const CONFIGURED_SYNONYM_WEIGHT: f32 = 0.75;

/// Expand a single query term using the default synonym table.
///
/// Returns empty slice if the term has no synonyms.
pub fn expand(term: &str) -> &'static [&'static str] {
    DEFAULT_SYNONYMS.get(term).copied().unwrap_or(&[])
}

/// Expand known natural-language phrases in a full query into code-like terms.
pub fn expand_phrases(query: &str) -> Vec<PhraseExpansion> {
    let normalized = query
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let mut expansions = Vec::new();
    for &(phrase, terms) in DEFAULT_PHRASE_SYNONYMS {
        if normalized.contains(phrase) {
            expansions.push(PhraseExpansion { phrase, terms });
        }
    }
    expansions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_expands() {
        let syns = expand("query");
        assert!(syns.contains(&"search"));
        assert!(syns.contains(&"find"));
        assert!(!syns.contains(&"query")); // doesn't include itself
    }

    #[test]
    fn unknown_term_returns_empty() {
        assert!(expand("mdm").is_empty());
    }

    #[test]
    fn chinese_business_terms_expand_to_code_terms() {
        let expansions = expand_phrases("获取法人身份证账号姓名");
        assert!(expansions.iter().any(|item| item.phrase == "法人"
            && item.terms.iter().any(|term| term.term == "legalperson")));
        assert!(expansions
            .iter()
            .any(|item| item.phrase == "身份证"
                && item.terms.iter().any(|term| term.term == "idcard")));
        assert!(expansions
            .iter()
            .any(|item| item.phrase == "账号"
                && item.terms.iter().any(|term| term.term == "account")));
        assert!(expansions.iter().any(|item| item.phrase == "姓名"
            && item.terms.iter().any(|term| term.term == "accountname")));
        assert!(expansions.iter().any(|item| item.phrase == "法人身份证"
            && item.terms.iter().any(|term| term.term == "legalpersonid")));
        assert!(expansions.iter().any(|item| item.phrase == "账号姓名"
            && item.terms.iter().any(|term| term.term == "customername")));
    }

    #[test]
    fn configured_synonym_rule_normalizes_code_identifiers() {
        let rule = SynonymRule::new(
            "商户",
            vec![
                "merchantAccount".to_string(),
                "merchant_id".to_string(),
                "mchNo".to_string(),
            ],
            CONFIGURED_SYNONYM_WEIGHT,
        );
        assert!(rule.matches_query("查询商户资料", &tokenize::tokenize("查询商户资料")));
        assert!(rule.terms.contains(&"merchantaccount".to_string()));
        assert!(rule.terms.contains(&"merchant".to_string()));
        assert!(rule.terms.contains(&"account".to_string()));
        assert!(rule.terms.contains(&"merchantid".to_string()));
        assert!(rule.terms.contains(&"mchno".to_string()));
    }
}
