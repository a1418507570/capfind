//! Tokenizer: splits identifiers and queries into searchable terms.
//!
//! Handles camelCase, PascalCase, snake_case, kebab-case, and path segments.
//! Also preserves the original full token (lowercased) for exact-phrase matching.

/// Split a string into search tokens.
///
/// - Splits on non-alphanumeric characters (whitespace, `/`, `.`, `_`, `-`, etc.)
/// - Further splits camelCase boundaries: `queryAssetList` → `[query, asset, list]`
/// - Also keeps the full joined token: `queryAssetList` → `[queryassetlist]` (extra)
/// - Filters stopwords and single-char tokens.
/// - All tokens are lowercased.
pub fn tokenize(input: &str) -> Vec<String> {
    let mut out = Vec::new();

    for raw in input.split(|c: char| !c.is_alphanumeric()) {
        if raw.is_empty() {
            continue;
        }

        // Keep the full token (lowercased) for exact matching.
        let full = raw.to_ascii_lowercase();
        if full.len() > 1 && !is_stopword(&full) {
            out.push(full.clone());
        }

        // Split on camelCase/PascalCase boundaries.
        let parts = split_camel(raw);
        if parts.len() > 1 {
            for part in parts {
                let lc = part.to_ascii_lowercase();
                if lc.len() <= 1 || is_stopword(&lc) {
                    continue;
                }
                // Don't duplicate if same as full.
                if lc != full {
                    out.push(lc);
                }
            }
        }
    }

    out
}

/// Tokenize specifically for HTTP paths: split on `/`.
pub fn tokenize_path(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        out.extend(tokenize(segment));
    }
    out
}

/// Split a word on camelCase/PascalCase boundaries.
///
/// `queryAsset` → `["query", "Asset"]`
/// `XMLParser` → `["XML", "Parser"]`
/// `getHTTPResponse` → `["get", "HTTP", "Response"]`
pub fn split_camel(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;

    for i in 1..bytes.len() {
        let prev = bytes[i - 1];
        let curr = bytes[i];

        let split_here =
            // lowercase → uppercase: `queryA` → split before A
            (prev.is_ascii_lowercase() && curr.is_ascii_uppercase()) ||
            // digit→letter boundary: `2List` → split before L (but NOT letter→digit like v→2)
            (prev.is_ascii_digit() && curr.is_ascii_alphabetic()) ||
            // uppercase run → next lowercase (for acronyms): `HTTPResponse` → `HTTP` + `Response`
            (i + 1 < bytes.len()
                && prev.is_ascii_uppercase()
                && curr.is_ascii_uppercase()
                && bytes[i + 1].is_ascii_lowercase());

        if split_here {
            if start < i {
                parts.push(&s[start..i]);
            }
            start = i;
        }
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

fn is_stopword(s: &str) -> bool {
    STOPWORDS.contains(&s)
}

const STOPWORDS: &[&str] = &[
    "get", "do", "impl", "util", "base", "common", "helper", "handler", "handle",
    "new", "old", "default", "abstract", "internal", "java", "go", "proto",
    "foo", "bar", "baz", "tmp", "temp", "the", "for", "and", "with", "from",
    "this", "that", "void", "null", "true", "false",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_camel_split() {
        assert_eq!(split_camel("queryAsset"), vec!["query", "Asset"]);
        assert_eq!(split_camel("XMLParser"), vec!["XML", "Parser"]);
        assert_eq!(split_camel("getHTTPResponse"), vec!["get", "HTTP", "Response"]);
        assert_eq!(split_camel("simple"), vec!["simple"]);
        assert_eq!(split_camel("v2List"), vec!["v2", "List"]);
    }

    #[test]
    fn tokenize_query() {
        let tokens = tokenize("mdm query");
        assert!(tokens.contains(&"mdm".to_string()));
        assert!(tokens.contains(&"query".to_string()));
    }

    #[test]
    fn tokenize_camel_case() {
        let tokens = tokenize("queryAssetList");
        assert!(tokens.contains(&"queryassetlist".to_string())); // full
        assert!(tokens.contains(&"query".to_string()));
        assert!(tokens.contains(&"asset".to_string()));
        assert!(tokens.contains(&"list".to_string()));
    }

    #[test]
    fn tokenize_path_segments() {
        let tokens = tokenize_path("/mdm/query");
        assert!(tokens.contains(&"mdm".to_string()));
        assert!(tokens.contains(&"query".to_string()));
    }

    #[test]
    fn stopwords_filtered() {
        let tokens = tokenize("getUser");
        // "get" is a stopword; "user" is not; "getuser" is the full token
        assert!(!tokens.contains(&"get".to_string()));
        assert!(tokens.contains(&"user".to_string()));
        assert!(tokens.contains(&"getuser".to_string()));
    }
}
