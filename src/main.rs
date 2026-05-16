//! Singularmem CLI — thin shell over `singularmem_core`.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use singularmem_core::{Error, ItemId, NewItem, Store, StoreOptions};

#[derive(Parser, Debug)]
#[command(name = "singularmem", version, about = "Local-first persistent memory layer for LLM workflows.")]
struct Cli {
    /// Path to the `SQLite` store file. Defaults to the per-user XDG data dir.
    #[arg(long, global = true, value_name = "PATH")]
    store: Option<PathBuf>,

    /// Open the store in read-only mode (refuses ingest).
    #[arg(long, global = true)]
    read_only: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Add a new item to the store.
    Ingest(IngestArgs),
    /// Fetch one item by ID.
    Get(GetArgs),
    /// Enumerate items, optionally filtered by tag.
    List(ListArgs),
    /// Show the supersedes chain for an item, newest-first.
    Revisions(RevisionsArgs),
    /// Emit the entire store as JSONL on stdout.
    Export,
}

#[derive(Args, Debug)]
struct IngestArgs {
    /// Item content as a literal string.
    #[arg(long, conflicts_with_all = ["file", "stdin"])]
    content: Option<String>,
    /// Read item content from a file.
    #[arg(long, conflicts_with_all = ["content", "stdin"])]
    file: Option<PathBuf>,
    /// Read item content from stdin.
    #[arg(long, conflicts_with_all = ["content", "file"])]
    stdin: bool,
    /// Tag (repeatable).
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// Free-form provenance label.
    #[arg(long)]
    source: Option<String>,
    /// Supersedes the given prior item ID.
    #[arg(long)]
    supersedes: Option<String>,
    /// Inline JSON object as the metadata payload.
    #[arg(long)]
    metadata: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = IngestFormat::Id)]
    format: IngestFormat,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum IngestFormat {
    Id,
    Json,
}

#[derive(Args, Debug)]
struct GetArgs {
    /// The item ID (26-char ULID, case-insensitive).
    id: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = GetFormat::Text)]
    format: GetFormat,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum GetFormat {
    Text,
    Json,
}

#[derive(Args, Debug)]
struct ListArgs {
    /// Filter to items containing every named tag (AND-semantics, repeatable).
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = ListFormat::Table)]
    format: ListFormat,
    /// Cap the number of items returned.
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ListFormat {
    Table,
    Jsonl,
    Ids,
}

#[derive(Args, Debug)]
struct RevisionsArgs {
    id: String,
    #[arg(long, value_enum, default_value_t = ListFormat::Table)]
    format: ListFormat,
}

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
        Err(e) => {
            eprintln!("singularmem: {e}");
            ExitCode::from(1)
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum CliError {
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
}

fn run(cli: Cli) -> Result<(), CliError> {
    let store_path = cli.store.unwrap_or_else(default_store_path);
    let opts = StoreOptions {
        read_only: cli.read_only,
    };
    let store = Store::open_with_options(&store_path, opts)?;

    match cli.command {
        Command::Ingest(args) => cmd_ingest(&store, args),
        Command::Get(args) => cmd_get(&store, &args),
        Command::List(args) => cmd_list(&store, &args),
        Command::Revisions(args) => cmd_revisions(&store, &args),
        Command::Export => cmd_export(&store),
    }
}

fn default_store_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("singularmem")
        .join("store.db")
}

fn cmd_ingest(store: &Store, args: IngestArgs) -> Result<(), CliError> {
    let content = match (args.content, args.file, args.stdin) {
        (Some(s), None, false) => s,
        (None, Some(p), false) => std::fs::read_to_string(&p)?,
        (None, None, true) => {
            let mut s = String::new();
            io::stdin().read_to_string(&mut s)?;
            s
        }
        _ => {
            return Err(CliError::Usage(
                "exactly one of --content, --file, --stdin must be provided".into(),
            ))
        }
    };

    let mut item = NewItem::text(content);
    item.tags = args.tags;
    item.source = args.source;
    if let Some(s) = args.supersedes {
        item.supersedes = Some(s.parse::<ItemId>()?);
    }
    if let Some(meta_text) = args.metadata {
        item.metadata = serde_json::from_str(&meta_text)?;
    }

    let stored = store.ingest(item)?;
    let mut out = io::stdout().lock();
    match args.format {
        IngestFormat::Id => writeln!(out, "{}", stored.id)?,
        IngestFormat::Json => {
            serde_json::to_writer(&mut out, &stored)?;
            writeln!(out)?;
        }
    }
    Ok(())
}

fn cmd_get(store: &Store, args: &GetArgs) -> Result<(), CliError> {
    let id = args.id.parse::<ItemId>()?;
    let item = store.get(id)?;
    let mut out = io::stdout().lock();
    match args.format {
        GetFormat::Text => write!(out, "{}", item.content)?,
        GetFormat::Json => {
            serde_json::to_writer(&mut out, &item)?;
            writeln!(out)?;
        }
    }
    Ok(())
}

fn cmd_list(store: &Store, args: &ListArgs) -> Result<(), CliError> {
    let tag_refs: Vec<&str> = args.tags.iter().map(String::as_str).collect();
    let iter: Box<dyn Iterator<Item = singularmem_core::Result<singularmem_core::Item>>> =
        if tag_refs.is_empty() {
            Box::new(store.list()?)
        } else {
            Box::new(store.list_by_tags(&tag_refs)?)
        };

    let iter: Box<dyn Iterator<Item = singularmem_core::Result<singularmem_core::Item>>> =
        if let Some(limit) = args.limit {
            Box::new(iter.take(limit))
        } else {
            iter
        };

    let mut out = io::stdout().lock();
    match args.format {
        ListFormat::Ids => {
            for r in iter {
                let item = r?;
                writeln!(out, "{}", item.id)?;
            }
        }
        ListFormat::Jsonl => {
            for r in iter {
                let item = r?;
                serde_json::to_writer(&mut out, &item)?;
                writeln!(out)?;
            }
        }
        ListFormat::Table => {
            // Two columns: ID  CONTENT (truncated to 80 chars).
            for r in iter {
                let item = r?;
                let snippet: String = item.content.chars().take(80).collect();
                writeln!(out, "{}\t{}", item.id, snippet.replace('\n', " "))?;
            }
        }
    }
    Ok(())
}

fn cmd_revisions(store: &Store, args: &RevisionsArgs) -> Result<(), CliError> {
    let id = args.id.parse::<ItemId>()?;
    let history = store.revision_history(id)?;
    let mut out = io::stdout().lock();
    for item in history {
        match args.format {
            ListFormat::Ids => writeln!(out, "{}", item.id)?,
            ListFormat::Jsonl => {
                serde_json::to_writer(&mut out, &item)?;
                writeln!(out)?;
            }
            ListFormat::Table => {
                let snippet: String = item.content.chars().take(80).collect();
                writeln!(out, "{}\t{}", item.id, snippet.replace('\n', " "))?;
            }
        }
    }
    Ok(())
}

fn cmd_export(store: &Store) -> Result<(), CliError> {
    let mut out = io::stdout().lock();
    store.export(&mut out)?;
    Ok(())
}
