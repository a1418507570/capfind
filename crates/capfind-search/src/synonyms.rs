//! Synonym expansion for query terms.
//!
//! Default synonym groups cover common coding verbs. Users extend via
//! `.capfind/config.toml`. Synonyms are only applied query-side (not during
//! indexing) to avoid postings bloat.

use once_cell::sync::Lazy;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct PhraseExpansion {
    pub phrase: &'static str,
    pub terms: &'static [&'static str],
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
        &["identity", "id", "card", "cert", "certificate"][..],
    );
    m.insert("identity", &["idcard", "id", "card"][..]);
    m.insert(
        "account",
        &["acct", "accountno", "accountnumber", "accountname"][..],
    );
    m.insert("name", &["username", "realname", "accountname"][..]);
    m.insert(
        "legalperson",
        &["legal", "person", "company", "corporation"][..],
    );
    m
});

static DEFAULT_PHRASE_SYNONYMS: &[(&str, &[&str])] = &[
    (
        "查询",
        &["query", "search", "find", "lookup", "fetch", "list"],
    ),
    ("查找", &["query", "search", "find", "lookup"]),
    ("获取", &["query", "search", "fetch", "lookup"]),
    ("列表", &["list", "query", "search"]),
    (
        "法人",
        &["legalperson", "legal", "person", "company", "corporation"],
    ),
    (
        "身份证",
        &["idcard", "identity", "id", "card", "cert", "certificate"],
    ),
    ("证件", &["idcard", "identity", "cert", "certificate"]),
    (
        "账号",
        &["account", "acct", "accountno", "accountnumber", "accountid"],
    ),
    (
        "账户",
        &["account", "acct", "accountno", "accountnumber", "accountid"],
    ),
    (
        "姓名",
        &["name", "accountname", "username", "realname", "personname"],
    ),
    ("名称", &["name", "accountname", "username", "realname"]),
    ("用户", &["user", "account", "member"]),
    ("客户", &["customer", "client", "account"]),
    ("手机", &["mobile", "phone", "telephone", "tel"]),
    ("手机号", &["mobile", "phone", "telephone", "tel"]),
    ("电话", &["phone", "telephone", "tel", "mobile"]),
    ("地址", &["address", "addr"]),
    ("订单", &["order"]),
    ("机构", &["org", "organization", "institution"]),
    ("企业", &["enterprise", "company", "corp", "corporation"]),
    ("公司", &["company", "corp", "corporation"]),
];

/// Weight applied to synonym-expanded terms (< 1.0 to not overpower originals).
pub const SYNONYM_WEIGHT: f32 = 0.6;

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
        assert!(expansions
            .iter()
            .any(|item| item.phrase == "法人" && item.terms.contains(&"legalperson")));
        assert!(expansions
            .iter()
            .any(|item| item.phrase == "身份证" && item.terms.contains(&"idcard")));
        assert!(expansions
            .iter()
            .any(|item| item.phrase == "账号" && item.terms.contains(&"account")));
        assert!(expansions
            .iter()
            .any(|item| item.phrase == "姓名" && item.terms.contains(&"accountname")));
    }
}
