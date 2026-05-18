//! Singularmem MCP server — speaks Model Context Protocol over stdio so
//! MCP-compatible clients (Claude Code, Cursor, custom agents) can
//! retrieve memories from the user's local Singularmem store.
//!
//! This binary is a thin shell over `singularmem-retrieve::Retriever`
//! plus the four provider adapters from sub-projects 3b/3c/3d. The
//! constitution names the MCP server as an open-core deliverable
//! (Open / Closed Split, line 245).
//!
//! Sub-project 4a ships the foundation + one read tool
//! (`memory_retrieve`). Sub-project 4b will add `memory_ingest` and
//! utility tools.
//!
//! See `docs/superpowers/specs/2026-05-18-mcp-server-4a-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// `singularmem-mcp` — MCP server over stdio.
#[derive(Parser, Debug)]
#[command(
    name = "singularmem-mcp",
    version,
    about = "MCP server exposing Singularmem retrieval over stdio."
)]
struct Args {
    /// Path to the `SQLite` store. Defaults to the per-user XDG data dir
    /// (same convention as `singularmem`).
    #[arg(long, env = "SINGULARMEM_STORE", value_name = "PATH")]
    store: Option<PathBuf>,

    /// Default adapter when clients don't specify one.
    #[arg(
        long,
        env = "SINGULARMEM_DEFAULT_ADAPTER",
        value_enum,
        default_value_t = AdapterChoice::Plain,
    )]
    default_adapter: AdapterChoice,

    /// `tracing` log level for stderr.
    #[arg(long, env = "RUST_LOG", value_enum, default_value_t = LogLevel::Info)]
    log_level: LogLevel,
}

/// Adapter choices recognised at startup. Mirrors the registered
/// adapters in the root binary's `known_adapters()` function.
#[derive(Copy, Clone, Debug, ValueEnum)]
enum AdapterChoice {
    Plain,
    Claude,
    Openai,
    Gemini,
}

impl AdapterChoice {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Claude => "claude",
            Self::Openai => "openai",
            Self::Gemini => "gemini",
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    // Used by Config construction in Task 3.
    #[allow(dead_code)]
    const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// Default store path: XDG data dir + `singularmem/store.db`. Same
/// convention as the root binary's `default_store_path()`.
fn default_store_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("singularmem")
        .join("store.db")
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Args::parse();

    // Configure tracing to write to stderr only. stdout is owned by
    // rmcp for JSON-RPC framing — any println! or stray stdout write
    // would corrupt the protocol stream.
    let filter = match args.log_level {
        LogLevel::Trace => "trace",
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(filter)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let store_path = args.store.unwrap_or_else(default_store_path);
    let config =
        singularmem_mcp::Config::new(store_path, args.default_adapter.as_str().to_string());

    match singularmem_mcp::serve(config).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "MCP server exited with error");
            std::process::ExitCode::FAILURE
        }
    }
}
