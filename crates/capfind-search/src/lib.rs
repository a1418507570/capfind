//! # capfind-search
//!
//! Deterministic search engine: tokenize queries and capabilities, apply BM25
//! with field-weighted scoring, synonym expansion, and structural boosts.
//!
//! Zero LLM. Same query + same index = same answer, always.

#![forbid(unsafe_code)]

pub mod score;
pub mod synonyms;
pub mod tokenize;

pub use score::{search, ExplainTrace, Hit, ScorerConfig, TermHit, FIELD_WEIGHTS};
pub use synonyms::{SynonymRule, CONFIGURED_SYNONYM_WEIGHT, SYNONYM_WEIGHT};
pub use tokenize::{split_camel, tokenize, tokenize_path};
