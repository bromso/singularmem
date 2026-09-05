//! Singularmem CLI — thin shell over `singularmem_core`.

use std::io;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use singularmem_core::{Error, Store, StoreOptions};

mod commands;

use commands::bulk::{cmd_ingest_codex, cmd_ingest_cursor, cmd_ingest_dir, cmd_ingest_transcript};
use commands::graph::{cmd_graph, GraphAction, GraphCommand};
use commands::hooks::{cmd_hook_entry, cmd_hooks};
use commands::index::wire_index_hooks;
use commands::items::{cmd_export, cmd_get, cmd_ingest, cmd_list, cmd_revisions, cmd_scope};
use commands::search::{cmd_reindex, cmd_retrieve, cmd_search, cmd_semantic_search};
use commands::wakeup::cmd_wake_up;
use commands::{resolve_store_path, Cli, Command, ScopeAction, ScopeCommand};

fn main() -> ExitCode {
    // Subscribe tracing to stderr at WARN level by default; user can override
    // with RUST_LOG=… environment variable.
    let _ = tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Lib(Error::NotFound { .. })) => ExitCode::from(2),
        Err(CliError::Lib(Error::UnsupportedFormatVersion { .. })) => ExitCode::from(3),
        Err(CliError::IndexOpen(ref e)) => {
            eprintln!("singularmem: {e}");
            ExitCode::from(2)
        }
        Err(CliError::Search(
            e @ (singularmem_search::Error::NoIndexes
            | singularmem_search::Error::HybridMissingIndex { .. }
            | singularmem_search::Error::IndexMissing { .. }
            | singularmem_search::Error::IndexSchemaMismatch { .. }),
        )) => {
            eprintln!("singularmem: {e}");
            ExitCode::from(2)
        }
        Err(CliError::Retrieve(ref e)) => {
            // Map retrieve-crate errors to the same exit codes as their
            // underlying search/core errors, plus EmptyQuery → 1.
            let code = match e {
                singularmem_retrieve::Error::Search(
                    singularmem_search::Error::NoIndexes
                    | singularmem_search::Error::HybridMissingIndex { .. }
                    | singularmem_search::Error::IndexMissing { .. }
                    | singularmem_search::Error::IndexSchemaMismatch { .. },
                )
                | singularmem_retrieve::Error::Core(singularmem_core::Error::NotFound { .. }) => 2,
                _ => 1,
            };
            eprintln!("singularmem: {e}");
            ExitCode::from(code)
        }
        Err(
            e @ (CliError::StoreReadOnly
            | CliError::Lib(Error::FactNotFound { .. } | Error::FactIdNotFound { .. })),
        ) => {
            eprintln!("singularmem: {e}");
            ExitCode::from(2)
        }
        Err(CliError::Ingest(singularmem_ingest::Error::NotFound { ref path })) => {
            eprintln!("singularmem: path not found: {}", path.display());
            ExitCode::from(2)
        }
        Err(CliError::IngestPartial { .. }) => ExitCode::from(1),
        Err(e) => {
            eprintln!("singularmem: {e}");
            ExitCode::from(1)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CliError {
    #[error("{0}")]
    Lib(#[from] Error),
    #[error("usage: {0}")]
    Usage(String),
    #[error("I/O: {0}")]
    Io(#[from] io::Error),
    #[error("invalid JSON for --metadata: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid item ID: {0}")]
    InvalidId(#[from] ulid::DecodeError),
    #[error("could not open Tantivy index: {0}")]
    IndexOpen(String),
    #[error("{0}")]
    Search(#[from] singularmem_search::Error),
    #[error("{0}")]
    Retrieve(#[from] singularmem_retrieve::Error),
    #[error("{0}")]
    Ingest(#[from] singularmem_ingest::Error),
    #[error("store is opened read-only; this command requires write access")]
    StoreReadOnly,
    #[error("{failed} item(s) failed during bulk ingest; see warnings above")]
    IngestPartial { failed: usize },
    #[error("{0}")]
    Hooks(#[from] singularmem_hooks::Error),
}

fn run(cli: Cli) -> Result<(), CliError> {
    // `hooks install|uninstall|status` never touches the store: dispatch
    // before `Store::open_with_options` so it works even when no store
    // exists yet (a common first-run scenario) and never contends for its
    // lock.
    if let Command::Hooks(cmd) = &cli.command {
        return cmd_hooks(cmd);
    }

    // `hook <editor> <event>` must never fail the calling editor — including
    // when the store itself cannot be opened (bad `--store` path, an
    // unsupported on-disk format version, …). Dispatch it before opening the
    // store so `cmd_hook_entry` can open (and gracefully fail to open) the
    // store on its own terms.
    if let Command::Hook(args) = &cli.command {
        return cmd_hook_entry(&cli, args);
    }

    let store_path = resolve_store_path(&cli);
    let opts = StoreOptions {
        read_only: cli.read_only,
    };
    let mut store = Store::open_with_options(&store_path, opts)?;

    if cli.read_only
        && matches!(
            cli.command,
            Command::IngestTranscript(_)
                | Command::IngestCodex(_)
                | Command::IngestCursor(_)
                | Command::IngestDir(_)
                | Command::Scope(ScopeCommand {
                    action: ScopeAction::Move { .. }
                })
                | Command::Graph(GraphCommand {
                    action: GraphAction::Add { .. }
                        | GraphAction::Invalidate { .. }
                        | GraphAction::Supersede { .. }
                })
        )
    {
        return Err(CliError::StoreReadOnly);
    }

    // Auto-wire hooks for write commands so live ingest populates the indices.
    // Read/search commands open their own Index instances; if we auto-wired here
    // AND those commands opened again, Tantivy's writer lock would conflict
    // (single-writer-per-Directory).
    let needs_hook = matches!(
        cli.command,
        Command::Ingest(_)
            | Command::IngestTranscript(_)
            | Command::IngestCodex(_)
            | Command::IngestCursor(_)
            | Command::IngestDir(_)
    );
    if needs_hook {
        wire_index_hooks(&mut store, &store_path, cli.no_index);
    }

    run_command(cli.command, &store, &store_path)
}

fn run_command(command: Command, store: &Store, store_path: &Path) -> Result<(), CliError> {
    match command {
        Command::Ingest(args) => cmd_ingest(store, args),
        Command::IngestTranscript(args) => cmd_ingest_transcript(store, &args),
        Command::IngestCodex(args) => cmd_ingest_codex(store, &args),
        Command::IngestCursor(args) => cmd_ingest_cursor(store, &args),
        Command::IngestDir(args) => cmd_ingest_dir(store, &args),
        Command::Get(args) => cmd_get(store, &args),
        Command::List(args) => cmd_list(store, &args),
        Command::Revisions(args) => cmd_revisions(store, &args),
        Command::Export => cmd_export(store),
        Command::Search(args) => cmd_search(store, store_path, &args),
        Command::Reindex(args) => cmd_reindex(store, store_path, &args),
        Command::Retrieve(args) => cmd_retrieve(store, store_path, &args),
        Command::SemanticSearch(args) => cmd_semantic_search(store, store_path, &args),
        Command::Scope(cmd) => cmd_scope(store, &cmd),
        Command::WakeUp(args) => cmd_wake_up(store, &args),
        Command::Hook(_) => unreachable!("Command::Hook is dispatched before the store opens"),
        Command::Hooks(_) => unreachable!("Command::Hooks is dispatched before the store opens"),
        Command::Graph(cmd) => cmd_graph(store, cmd),
    }
}
