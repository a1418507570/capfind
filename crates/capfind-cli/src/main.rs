use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

mod commands;
mod fmt;
mod indexer;

#[derive(Parser)]
#[command(name = "capfind", version, about = "Find reusable capabilities in big polyglot repos")]
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
    Init,
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
    },
    /// Print index statistics
    Stats,
    /// Search with detailed scoring explanation (alias for find --explain)
    Explain(FindArgs),
    /// Print what the parser extracts from a single file
    Diagnose {
        /// Path to a source file
        file: PathBuf,
    },
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
        Cmd::Init => commands::init(&repo_root),
        Cmd::Index { rehash } => commands::index(&repo_root, rehash),
        Cmd::Find(args) => commands::find(&repo_root, &args, cli.no_color),
        Cmd::Explain(mut args) => {
            args.explain = true;
            commands::find(&repo_root, &args, cli.no_color)
        }
        Cmd::Show { id } => commands::show(&repo_root, id, cli.no_color),
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
