//! Synonym expansion for query terms.
//!
//! Default synonym groups cover common coding verbs. Users extend via
//! `.capfind/config.toml`. Synonyms are only applied query-side (not during
//! indexing) to avoid postings bloat.

use once_cell::sync::Lazy;
use std::collections::HashMap;

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
    m
});

/// Weight applied to synonym-expanded terms (< 1.0 to not overpower originals).
pub const SYNONYM_WEIGHT: f32 = 0.6;

/// Expand a single query term using the default synonym table.
///
/// Returns empty slice if the term has no synonyms.
pub fn expand(term: &str) -> &'static [&'static str] {
    DEFAULT_SYNONYMS.get(term).copied().unwrap_or(&[])
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
}
