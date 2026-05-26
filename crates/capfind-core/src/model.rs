//! Core data model for capfind.
//!
//! Everything here is what ends up in the on-disk index file. Field layouts are
//! stable within a major index version; breaking changes bump [`crate::INDEX_VERSION`].

use serde::{Deserialize, Serialize};

/// What kind of reusable thing this capability represents.
///
/// Ordered by "how likely a developer is looking for this" — highest-level
/// entry points first, internals last. [`capfind-search`] uses the ordering
/// for layer-boost scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Kind {
    /// Spring `@RequestMapping`/`@GetMapping`/… or gorilla/mux route, or any
    /// public HTTP surface.
    HttpEndpoint = 0,
    /// tRPC / proto `rpc Foo(Req) returns (Rsp)`.
    RpcMethod = 1,
    /// `@Service`, `@Component`, or a Go function living in `server/*/service/`.
    ServiceMethod = 2,
    /// `@Repository` or a Go function living in `server/*/repo/`.
    DaoMethod = 3,
    /// Fallback bucket — exported but not classifiable.
    Other = 9,
}

impl Kind {
    pub const ALL: &'static [Kind] = &[
        Kind::HttpEndpoint,
        Kind::RpcMethod,
        Kind::ServiceMethod,
        Kind::DaoMethod,
        Kind::Other,
    ];

    /// Lowercase name, used in CLI flags and JSON output.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::HttpEndpoint => "HttpEndpoint",
            Kind::RpcMethod => "RpcMethod",
            Kind::ServiceMethod => "ServiceMethod",
            Kind::DaoMethod => "DaoMethod",
            Kind::Other => "Other",
        }
    }
}

/// Source language of a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Lang {
    Java = 0,
    Go = 1,
    Proto = 2,
}

impl Lang {
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Java => "Java",
            Lang::Go => "Go",
            Lang::Proto => "Proto",
        }
    }
}

/// Which scoring field a term reference belongs to.
///
/// Kept in sync with the weight table in `capfind-search` (`FIELD_WEIGHTS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Field {
    HttpPath = 0,
    ClassName = 1,
    MethodName = 2,
    AnnotationValue = 3,
    Doc = 4,
    PackageModule = 5,
}

impl Field {
    pub const ALL: &'static [Field] = &[
        Field::HttpPath,
        Field::ClassName,
        Field::MethodName,
        Field::AnnotationValue,
        Field::Doc,
        Field::PackageModule,
    ];
}

/// One (term, field, tf) triple. Packed to keep `Capability` small.
///
/// `term_id` indexes into `IndexBody::vocab`. `tf` caps at 65 535 — more than
/// enough for any realistic method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermRef {
    pub term_id: u32,
    pub field: Field,
    pub tf: u16,
}

/// HTTP-specific detail. Present iff `Capability::kind == HttpEndpoint`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpInfo {
    /// GET / POST / PUT / DELETE / PATCH / ANY
    pub method: String,
    /// Final merged path (class-level + method-level joined with `/`).
    pub path: String,
    pub consumes: Option<String>,
    pub produces: Option<String>,
}

/// RPC-specific detail. Present iff `Capability::kind == RpcMethod`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcInfo {
    pub service: String,
    pub rpc: String,
    pub req: String,
    pub rsp: String,
    pub proto_file: Option<String>,
}

/// A single reusable capability in the indexed repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub id: u32,
    pub kind: Kind,
    pub lang: Lang,

    pub module: String,
    pub package: String,
    pub class: Option<String>,
    pub method: String,
    pub signature: String,
    pub annotations: Vec<String>,
    pub http: Option<HttpInfo>,
    pub rpc: Option<RpcInfo>,
    pub doc: Option<String>,
    pub tags: Vec<String>,

    pub file: String,
    pub line: u32,
    pub byte_range: (u32, u32),

    /// Post-tokenize term references — filled in during index build,
    /// consumed by `capfind-search` to score queries.
    pub terms: Vec<TermRef>,
}

impl Capability {
    /// A stable `class.method` identifier for display.
    pub fn qualified(&self) -> String {
        match (&self.class, self.method.as_str()) {
            (Some(c), m) => format!("{c}#{m}"),
            (None, m) => m.to_string(),
        }
    }
}

/// Filesystem metadata used by the incremental index.
///
/// `mtime_ns` + `size` are cheap. `blake3` is only recomputed when the cheap
/// pair already disagrees — see `capfind-cli`'s `index` implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStat {
    pub mtime_ns: i64,
    pub size: u64,
    pub blake3: [u8; 32],
}
