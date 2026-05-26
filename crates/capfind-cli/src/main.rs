use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

mod adoption;
mod asset_map;
mod commands;
mod config;
mod context;
mod eval;
mod file_diagnosis;
mod fmt;
mod indexer;

#[derive(Parser)]
#[command(
    name = "capfind",
    version,
    about = "Find reusable capabilities in big polyglot repos"
)]
pub struct Cli {
    /// Disable colored output
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Verbosity (repeat for more)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Initialize .capfind/ in the repo root
    Init(InitArgs),
    /// Scan the repo and build/update the index
    Index {
        /// Force full rebuild (ignore mtime cache)
        #[arg(long)]
        rehash: bool,
    },
    /// Search indexed capabilities
    Find(FindArgs),
    /// Show full details for a capability by ID
    Show {
        /// Capability ID from index
        id: u32,
        /// Task text for optional inspected-stage adoption recording
        #[arg(long)]
        task: Option<String>,
        /// Record an inspected-stage adoption event for this capability
        #[arg(long)]
        record_inspected: bool,
        /// Optional agent/session id for adoption funnel correlation
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Print index statistics
    Stats,
    /// Search with detailed scoring explanation (alias for find --explain)
    Explain(FindArgs),
    /// Agent-friendly preflight check for duplicate capabilities
    Agent(AgentArgs),
    /// Low-token AI context packet for coding agents
    Context(ContextArgs),
    /// Diagnose index coverage and configuration
    Doctor(DoctorArgs),
    /// Diagnose why a query does or does not return expected assets
    DiagnoseQuery(DiagnoseQueryArgs),
    /// Diagnose file-level index and parser coverage
    DiagnoseFile(DiagnoseFileArgs),
    /// Generate a local product/architecture dashboard
    Dashboard(DashboardArgs),
    /// Emit a machine-readable code asset/service map
    Map(MapArgs),
    /// Evaluate search and file diagnosis quality against golden fixtures
    Eval(EvalArgs),
    /// Record whether generated code adopted a candidate
    RecordAdoption(RecordAdoptionArgs),
    /// Detect adoption from git diff and record matched candidates
    DetectAdoption(DetectAdoptionArgs),
    /// MCP-compatible tool catalog and local tool-call shim
    Mcp(McpArgs),
    /// Print what the parser extracts from a single file
    Diagnose {
        /// Path to a source file
        file: PathBuf,
    },
}

#[derive(clap::Args, Clone)]
pub struct InitArgs {
    /// Generate repo-specific suggested capfind and Agent/MCP config files under .capfind/
    #[arg(long)]
    pub product_config: bool,

    /// Overwrite generated product config files when they already exist
    #[arg(long)]
    pub force: bool,
}

#[derive(clap::Args, Clone)]
pub struct FindArgs {
    /// Query terms (e.g. "mdm query")
    pub query: Vec<String>,

    /// Max results
    #[arg(long, short = 'n', default_value_t = 10)]
    pub limit: usize,

    /// Filter by language
    #[arg(long, value_enum)]
    pub lang: Option<LangFilter>,

    /// Filter by capability kind
    #[arg(long, value_enum)]
    pub kind: Option<KindFilter>,

    /// Filter by file path prefix
    #[arg(long)]
    pub path: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Show scoring explanation
    #[arg(long)]
    pub explain: bool,
}

#[derive(clap::Args, Clone)]
pub struct AgentArgs {
    /// Natural-language task or intent to check before coding
    pub task: Vec<String>,

    /// Max candidate capabilities to return
    #[arg(long, short = 'n', default_value_t = 5)]
    pub limit: usize,

    /// Filter by language
    #[arg(long, value_enum)]
    pub lang: Option<LangFilter>,

    /// Filter by capability kind
    #[arg(long, value_enum)]
    pub kind: Option<KindFilter>,

    /// Filter by file path prefix
    #[arg(long)]
    pub path: Option<String>,

    /// Build the index automatically when .capfind/index.cfi is missing
    #[arg(long)]
    pub auto_index: bool,

    /// Disable automatic index creation when .capfind/index.cfi is missing
    #[arg(long, conflicts_with = "auto_index")]
    pub no_auto_index: bool,

    /// Exit with code 2 when similar candidates are found; useful for blocking hooks
    #[arg(long)]
    pub fail_on_candidates: bool,

    /// Accepted for hook ergonomics; agent output is always JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Clone)]
pub struct ContextArgs {
    /// Natural-language task or intent to ground in indexed assets
    pub task: Vec<String>,

    /// Max candidate assets to return
    #[arg(long, short = 'n', default_value_t = 5)]
    pub limit: usize,

    /// Filter by language
    #[arg(long, value_enum)]
    pub lang: Option<LangFilter>,

    /// Filter by capability kind
    #[arg(long, value_enum)]
    pub kind: Option<KindFilter>,

    /// Filter by file path prefix
    #[arg(long)]
    pub path: Option<String>,

    /// Accepted for compatibility; context auto-indexes by default
    #[arg(long)]
    pub auto_index: bool,

    /// Disable automatic index creation when .capfind/index.cfi is missing
    #[arg(long, conflicts_with = "auto_index")]
    pub no_auto_index: bool,

    /// Record shown-stage adoption events for returned candidates
    #[arg(long)]
    pub record_shown: bool,

    /// Optional agent/session id for adoption funnel correlation
    #[arg(long)]
    pub session_id: Option<String>,
}

#[derive(clap::Args, Clone)]
pub struct DoctorArgs {
    /// Emit machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args, Clone)]
pub struct DiagnoseQueryArgs {
    /// Natural-language query or task to diagnose
    pub query: Vec<String>,

    /// Max diagnostic hits to return
    #[arg(long, short = 'n', default_value_t = 10)]
    pub limit: usize,

    /// Filter by language
    #[arg(long, value_enum)]
    pub lang: Option<LangFilter>,

    /// Filter by capability kind
    #[arg(long, value_enum)]
    pub kind: Option<KindFilter>,

    /// Filter by file path prefix
    #[arg(long)]
    pub path: Option<String>,

    /// Accepted for compatibility; diagnose-query auto-indexes by default
    #[arg(long)]
    pub auto_index: bool,

    /// Disable automatic index creation when .capfind/index.cfi is missing
    #[arg(long, conflicts_with = "auto_index")]
    pub no_auto_index: bool,
}

#[derive(clap::Args, Clone)]
pub struct DiagnoseFileArgs {
    /// Source/reference file to diagnose
    pub file: PathBuf,

    /// Accepted for compatibility; diagnose-file auto-indexes by default
    #[arg(long)]
    pub auto_index: bool,

    /// Disable automatic index creation when .capfind/index.cfi is missing
    #[arg(long, conflicts_with = "auto_index")]
    pub no_auto_index: bool,
}

#[derive(clap::Args, Clone)]
pub struct DashboardArgs {
    /// Emit the dashboard data as JSON instead of writing HTML
    #[arg(long)]
    pub json: bool,

    /// Restrict dashboard topology and asset map to a module/path prefix
    #[arg(long)]
    pub module: Option<String>,

    /// Restrict the visual graph to relationship kinds; repeat or comma-separate values
    #[arg(long = "relationship", value_delimiter = ',')]
    pub relationships: Vec<String>,

    /// Max module-link edges to render in the visual graph
    #[arg(long, default_value_t = 16)]
    pub graph_limit: usize,

    /// Record a dashboard snapshot in .capfind/dashboard-history.jsonl
    #[arg(long, conflicts_with = "no_record_history")]
    pub record_history: bool,

    /// Disable dashboard history recording for this run
    #[arg(long, conflicts_with = "record_history")]
    pub no_record_history: bool,

    /// HTML output path. Defaults to .capfind/dashboard.html
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(clap::Args, Clone)]
pub struct MapArgs {
    /// Restrict the map to a module/path prefix
    #[arg(long)]
    pub module: Option<String>,

    /// Max capability nodes to include in the graph section
    #[arg(long, default_value_t = 200)]
    pub limit: usize,

    /// Accepted for compatibility; map auto-indexes by default
    #[arg(long)]
    pub auto_index: bool,

    /// Disable automatic index creation when .capfind/index.cfi is missing
    #[arg(long, conflicts_with = "auto_index")]
    pub no_auto_index: bool,
}

#[derive(clap::Args, Clone)]
pub struct EvalArgs {
    /// Query golden cases in JSONL format
    #[arg(long, default_value = "fixtures/eval/queries.jsonl")]
    pub queries: PathBuf,

    /// File diagnosis golden cases in JSONL format
    #[arg(long, default_value = "fixtures/eval/files.jsonl")]
    pub files: PathBuf,

    /// Asset-map edge golden cases in JSONL format
    #[arg(long, default_value = "fixtures/eval/edges.jsonl")]
    pub edges: PathBuf,

    /// Asset-map diagnostic golden cases in JSONL format
    #[arg(long, default_value = "fixtures/eval/diagnostics.jsonl")]
    pub diagnostics: PathBuf,

    /// Asset-map ownership golden cases in JSONL format
    #[arg(long, default_value = "fixtures/eval/ownerships.jsonl")]
    pub ownerships: PathBuf,

    /// Default top-K limit for query cases without their own limit
    #[arg(long, default_value_t = 10)]
    pub limit: usize,

    /// Logical suite name for eval history grouping
    #[arg(long)]
    pub suite: Option<String>,

    /// Record an eval snapshot in .capfind/eval-history.jsonl
    #[arg(long, conflicts_with = "no_record_history")]
    pub record_history: bool,

    /// Disable eval history recording for this run
    #[arg(long, conflicts_with = "record_history")]
    pub no_record_history: bool,

    /// Accepted for compatibility; eval auto-indexes by default
    #[arg(long)]
    pub auto_index: bool,

    /// Disable automatic index creation when .capfind/index.cfi is missing
    #[arg(long, conflicts_with = "auto_index")]
    pub no_auto_index: bool,

    /// Exit with code 2 if query Recall@K is below this threshold
    #[arg(long)]
    pub fail_under_recall: Option<f64>,

    /// Exit with code 2 if file diagnosis pass rate is below this threshold
    #[arg(long)]
    pub fail_under_file_pass: Option<f64>,

    /// Exit with code 2 if asset-map edge pass rate is below this threshold
    #[arg(long)]
    pub fail_under_edge_pass: Option<f64>,

    /// Exit with code 2 if asset-map diagnostic pass rate is below this threshold
    #[arg(long)]
    pub fail_under_diagnostic_pass: Option<f64>,

    /// Exit with code 2 if ownership metadata pass rate is below this threshold
    #[arg(long)]
    pub fail_under_ownership_pass: Option<f64>,
}

#[derive(clap::Args, Clone)]
pub struct RecordAdoptionArgs {
    /// Candidate id from capfind context/search/show
    pub candidate_id: u32,

    /// Task text that produced the candidate
    #[arg(long)]
    pub task: String,

    /// Funnel stage to record
    #[arg(long, value_enum)]
    pub stage: Option<AdoptionStage>,

    /// Mark the candidate as not adopted
    #[arg(long)]
    pub rejected: bool,

    /// Reason category when the candidate was rejected
    #[arg(long = "rejected-reason")]
    pub rejected_reason: Option<String>,

    /// Optional implementation evidence such as changed file paths
    #[arg(long)]
    pub file: Vec<String>,

    /// Optional short note
    #[arg(long)]
    pub note: Option<String>,

    /// Optional agent/session id for adoption funnel correlation
    #[arg(long)]
    pub session_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AdoptionStage {
    Shown,
    Inspected,
    Adopted,
    Rejected,
}

impl AdoptionStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shown => "shown",
            Self::Inspected => "inspected",
            Self::Adopted => "adopted",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(clap::Args, Clone)]
pub struct DetectAdoptionArgs {
    /// Git revision or range to compare against (default: HEAD)
    #[arg(long, default_value = "HEAD")]
    pub since: String,

    /// Optional task text to store with auto-detected adoption events
    #[arg(long)]
    pub task: Option<String>,

    /// Restrict detection to specific candidate ids
    #[arg(long = "candidate-id")]
    pub candidate_id: Vec<u32>,

    /// Preview detections without writing .capfind/adoptions.jsonl
    #[arg(long)]
    pub dry_run: bool,

    /// Optional agent/session id for adoption funnel correlation
    #[arg(long)]
    pub session_id: Option<String>,

    /// Accepted for compatibility; detect-adoption auto-indexes by default
    #[arg(long)]
    pub auto_index: bool,

    /// Disable automatic index creation when .capfind/index.cfi is missing
    #[arg(long, conflicts_with = "auto_index")]
    pub no_auto_index: bool,
}

#[derive(clap::Args, Clone)]
pub struct McpArgs {
    /// Run a JSON-RPC MCP stdio server
    #[arg(long)]
    pub stdio: bool,

    /// Build the index automatically when MCP tools are called and no index exists
    #[arg(long)]
    pub auto_index: bool,

    /// Disable automatic index creation when MCP tools are called and no index exists
    #[arg(long, conflicts_with = "auto_index")]
    pub no_auto_index: bool,

    /// Print MCP tool definitions as JSON and exit
    #[arg(long)]
    pub list_tools: bool,

    /// Call a tool once without starting a long-running server
    #[arg(long)]
    pub call: Option<String>,

    /// JSON arguments for --call
    #[arg(long, default_value = "{}")]
    pub args: String,
}

#[derive(Clone, ValueEnum)]
pub enum LangFilter {
    Java,
    Go,
    Proto,
}

#[derive(Clone, ValueEnum)]
pub enum KindFilter {
    Endpoint,
    Rpc,
    Service,
    Dao,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Init tracing based on verbosity.
    let level = match cli.verbose {
        0 => tracing::Level::WARN,
        1 => tracing::Level::INFO,
        2 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .init();

    // Determine repo root.
    let repo_root = find_repo_root()?;

    match cli.cmd {
        Cmd::Init(args) => commands::init(&repo_root, &args),
        Cmd::Index { rehash } => commands::index(&repo_root, rehash),
        Cmd::Find(args) => commands::find(&repo_root, &args, cli.no_color),
        Cmd::Explain(mut args) => {
            args.explain = true;
            commands::find(&repo_root, &args, cli.no_color)
        }
        Cmd::Agent(args) => commands::agent(&repo_root, &args),
        Cmd::Context(args) => commands::context(&repo_root, &args),
        Cmd::Doctor(args) => commands::doctor(&repo_root, &args),
        Cmd::DiagnoseQuery(args) => commands::diagnose_query(&repo_root, &args),
        Cmd::DiagnoseFile(args) => commands::diagnose_file(&repo_root, &args),
        Cmd::Dashboard(args) => commands::dashboard(&repo_root, &args),
        Cmd::Map(args) => commands::asset_map(&repo_root, &args),
        Cmd::Eval(args) => commands::eval(&repo_root, &args),
        Cmd::RecordAdoption(args) => commands::record_adoption(&repo_root, &args),
        Cmd::DetectAdoption(args) => commands::detect_adoption(&repo_root, &args),
        Cmd::Mcp(args) => commands::mcp(&repo_root, &args),
        Cmd::Show {
            id,
            task,
            record_inspected,
            session_id,
        } => commands::show(
            &repo_root,
            id,
            cli.no_color,
            task.as_deref(),
            record_inspected,
            session_id.as_deref(),
        ),
        Cmd::Stats => commands::stats(&repo_root),
        Cmd::Diagnose { file } => commands::diagnose(&file),
    }
}

/// Walk up from CWD to find repo root (has .git, pom.xml, go.work, etc.)
fn find_repo_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("cannot determine CWD")?;
    let mut dir = cwd.as_path();
    loop {
        if dir.join(".git").exists()
            || dir.join("pom.xml").exists()
            || dir.join("go.work").exists()
            || dir.join("settings.gradle").exists()
            || dir.join("settings.gradle.kts").exists()
            || dir.join("Cargo.toml").exists()
        {
            return Ok(dir.to_path_buf());
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => return Ok(cwd), // fallback to CWD
        }
    }
}
