//! Singularmem CLI — thin shell over `singularmem_core`.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use singularmem_core::{Error, ItemId, NewItem, Store, StoreOptions};

#[derive(Parser, Debug)]
#[command(
    name = "singularmem",
    version,
    about = "Local-first persistent memory layer for LLM workflows."
)]
struct Cli {
    /// Path to the `SQLite` store file. Defaults to the per-user XDG data dir.
    #[arg(long, global = true, value_name = "PATH")]
    store: Option<PathBuf>,

    /// Open the store in read-only mode (refuses ingest).
    #[arg(long, global = true)]
    read_only: bool,

    /// Skip wiring up the Tantivy hook on open. Use for storage-only operations
    /// that don't need search, or when the Tantivy directory is intentionally absent.
    #[arg(long, global = true)]
    no_index: bool,

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
    /// Full-text search over the store.
    Search(SearchArgs),
    /// Rebuild the Tantivy index from the `SQLite` store.
    Reindex(ReindexArgs),
    /// Semantic (vector) search over the store.
    SemanticSearch(SemanticSearchArgs),
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

/// Which search backend(s) to use for `search`.
#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
enum SearchMode {
    /// Use hybrid when both `.tantivy/` and `.vectors/` exist; degrade to
    /// whichever single index is present; error when neither exists.
    Auto,
    /// Tantivy BM25 only.
    Lexical,
    /// `USearch` cosine only.
    Semantic,
    /// RRF-fused lexical + semantic; error if either is missing.
    Hybrid,
}

#[derive(Args, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct SearchArgs {
    /// One or more query tokens. Multiple tokens become an implicit AND.
    queries: Vec<String>,
    /// Which backend(s) to use. `auto` picks hybrid when both sidecars exist,
    /// falls back to whichever one is present, and errors when neither is.
    #[arg(short = 'm', long, value_enum, default_value_t = SearchMode::Auto)]
    mode: SearchMode,
    /// Max hits to return.
    #[arg(short = 'l', long, default_value = "20")]
    limit: usize,
    /// Skip first N hits (pagination, lexical mode only).
    #[arg(long, default_value = "0")]
    offset: usize,
    /// Per-ranker overfetch factor; hybrid only. Default 3.
    #[arg(long, default_value = "3")]
    fetch_multiplier: usize,
    /// RRF damping constant; hybrid only. Default 60.
    #[arg(long, default_value = "60")]
    rrf_k: usize,
    /// Suppress snippet highlighting (faster).
    #[arg(long)]
    no_snippets: bool,
    /// Include per-ranker rank columns in human output.
    #[arg(long)]
    show_ranks: bool,
    /// Emit JSON results instead of human-readable output.
    #[arg(long)]
    json: bool,
    /// Output format. (Legacy; `--json` and `--show-ranks` are preferred.)
    #[arg(long, value_enum, default_value_t = ListFormat::Table)]
    format: ListFormat,
}

#[derive(Args, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct ReindexArgs {
    /// Suppress progress output.
    #[arg(long)]
    quiet: bool,
    /// Also rebuild the vector index.
    #[arg(long)]
    with_embeddings: bool,
    /// Which embedding model to use. Only meaningful with --with-embeddings.
    #[arg(long, default_value = "all-mini-lm-l6-v2")]
    embedding_model: String,
    /// Destructive — delete .vectors/ before reindex (e.g. to switch models).
    #[arg(long)]
    reset_vectors: bool,
    /// Required to confirm --reset-vectors.
    #[arg(long)]
    force: bool,
}

#[derive(Args, Debug)]
struct SemanticSearchArgs {
    /// One or more query tokens. Multiple tokens are joined with a space.
    queries: Vec<String>,
    /// Max hits to return.
    #[arg(long, default_value = "20")]
    limit: usize,
    /// Minimum cosine-similarity score (0.0–1.0) for a hit to be included.
    #[arg(long, default_value = "0.0")]
    min_score: f32,
    /// Output format.
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
        Err(CliError::IndexOpen(ref e)) => {
            eprintln!("singularmem: {e}");
            ExitCode::from(2)
        }
        Err(CliError::QueryParse(ref e)) => {
            eprintln!("singularmem: invalid search query: {e}");
            ExitCode::from(1)
        }
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
    #[error("could not open Tantivy index: {0}")]
    IndexOpen(String),
    #[error("invalid search query: {0}")]
    QueryParse(String),
}

fn run(cli: Cli) -> Result<(), CliError> {
    let store_path = cli.store.clone().unwrap_or_else(default_store_path);
    let opts = StoreOptions {
        read_only: cli.read_only,
    };
    let mut store = Store::open_with_options(&store_path, opts)?;

    // Auto-wire hooks for write commands so live ingest populates the indices.
    // Read/search commands open their own Index instances; if we auto-wired here
    // AND those commands opened again, Tantivy's writer lock would conflict
    // (single-writer-per-Directory).
    let needs_hook = matches!(cli.command, Command::Ingest(_));
    if needs_hook && !cli.no_index {
        let mut hooks: Vec<Box<dyn singularmem_core::IndexHook>> = Vec::new();

        // Tantivy lexical-search hook (sub-project 2a behaviour — always attempt).
        let index_path = derive_index_path(&store_path);
        match singularmem_search::Index::open(&index_path) {
            Ok(idx) => hooks.push(Box::new(idx)),
            Err(e) => tracing::warn!(
                error = %e,
                path = %index_path.display(),
                "could not open Tantivy index; lexical search will not work until reindex"
            ),
        }

        // Embedder / vector hook — opt-in: only when .vectors/ already exists.
        let vectors_path = derive_vectors_path(&store_path);
        if vectors_path.exists() {
            let embedder: Box<dyn singularmem_search::Embedder> =
                match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                    Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                    _ => match singularmem_search::FastembedEmbedder::new() {
                        Ok(e) => Box::new(e),
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "embedder construction failed; semantic search will not work"
                            );
                            // Skip embedder hook; proceed with whatever hooks were assembled.
                            if !hooks.is_empty() {
                                store.set_hook(Some(Box::new(
                                    singularmem_core::hook::MultiHook::new(hooks),
                                )));
                            }
                            return run_command(cli.command, &store, &store_path);
                        }
                    },
                };
            match singularmem_search::EmbedderIndex::open(&vectors_path, embedder) {
                Ok(idx) => hooks.push(Box::new(idx)),
                Err(e) => tracing::warn!(
                    error = %e,
                    "vector index open failed; semantic search will not work"
                ),
            }
        }

        if !hooks.is_empty() {
            store.set_hook(Some(Box::new(singularmem_core::hook::MultiHook::new(
                hooks,
            ))));
        }
    }

    run_command(cli.command, &store, &store_path)
}

fn run_command(command: Command, store: &Store, store_path: &Path) -> Result<(), CliError> {
    match command {
        Command::Ingest(args) => cmd_ingest(store, args),
        Command::Get(args) => cmd_get(store, &args),
        Command::List(args) => cmd_list(store, &args),
        Command::Revisions(args) => cmd_revisions(store, &args),
        Command::Export => cmd_export(store),
        Command::Search(args) => cmd_search(store_path, &args),
        Command::Reindex(args) => cmd_reindex(store, store_path, &args),
        Command::SemanticSearch(args) => cmd_semantic_search(store_path, &args),
    }
}

fn derive_index_path(store_path: &Path) -> PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".tantivy");
    PathBuf::from(s)
}

fn derive_vectors_path(store_path: &Path) -> PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".vectors");
    PathBuf::from(s)
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

fn cmd_search(store_path: &Path, args: &SearchArgs) -> Result<(), CliError> {
    use singularmem_search::{Index, Query, SearchOptions};
    // Suppress unused-arg warnings; Task 10 wires these through.
    let _ = (
        &args.mode,
        args.fetch_multiplier,
        args.rrf_k,
        args.show_ranks,
        args.json,
    );
    let index_path = derive_index_path(store_path);
    let index = Index::open(&index_path).map_err(|e| CliError::IndexOpen(e.to_string()))?;
    let query_str = args.queries.join(" ");
    let query = Query::parse(&query_str).map_err(|e| CliError::QueryParse(e.to_string()))?;
    let opts = SearchOptions {
        limit: args.limit,
        offset: args.offset,
        include_snippets: !args.no_snippets,
    };
    let results = index
        .search(&query, opts)
        .map_err(|e| CliError::IndexOpen(e.to_string()))?;

    if results.total_matched == 0 {
        tracing::info!("0 matches");
        return Ok(());
    }

    let mut out = io::stdout().lock();
    for hit in &results.hits {
        match args.format {
            ListFormat::Ids => writeln!(out, "{}", hit.id)?,
            ListFormat::Jsonl => {
                let line = serde_json::json!({
                    "id": hit.id.to_string(),
                    "score": hit.score,
                    "snippet": hit.snippet,
                });
                serde_json::to_writer(&mut out, &line)?;
                writeln!(out)?;
            }
            ListFormat::Table => {
                let snip = hit.snippet.as_deref().unwrap_or("").replace('\n', " ");
                writeln!(out, "{:.4}\t{}\t{}", hit.score, hit.id, snip)?;
            }
        }
    }
    Ok(())
}

fn cmd_semantic_search(store_path: &Path, args: &SemanticSearchArgs) -> Result<(), CliError> {
    let vectors_path = derive_vectors_path(store_path);

    // The .vectors/ directory is the opt-in signal: it is created by
    // `reindex --with-embeddings`. If it doesn't exist, semantic search is not
    // available for this store yet.
    if !vectors_path.exists() {
        return Err(CliError::IndexOpen(format!(
            "vector index not found at {}; run `singularmem reindex --with-embeddings` first",
            vectors_path.display()
        )));
    }

    // Production CLI uses FastembedEmbedder. Tests inject MockEmbedder via env var
    // to stay fast and network-free.
    let embedder: Box<dyn singularmem_search::Embedder> =
        match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
            Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
            _ => Box::new(
                singularmem_search::FastembedEmbedder::new()
                    .map_err(|e| CliError::IndexOpen(format!("embedder init failed: {e}")))?,
            ),
        };
    let idx = singularmem_search::EmbedderIndex::open(&vectors_path, embedder)
        .map_err(|e| CliError::IndexOpen(e.to_string()))?;
    let query_str = args.queries.join(" ");
    let results = idx
        .semantic_search(
            &query_str,
            &singularmem_search::SemanticSearchOptions {
                limit: args.limit,
                min_score: args.min_score,
            },
        )
        .map_err(|e| CliError::IndexOpen(e.to_string()))?;

    if results.hits.is_empty() {
        tracing::info!("0 matches");
        return Ok(());
    }
    let mut out = io::stdout().lock();
    for hit in &results.hits {
        match args.format {
            ListFormat::Ids => writeln!(out, "{}", hit.id)?,
            ListFormat::Jsonl => {
                serde_json::to_writer(
                    &mut out,
                    &serde_json::json!({
                        "id": hit.id.to_string(),
                        "score": hit.score,
                    }),
                )?;
                writeln!(out)?;
            }
            ListFormat::Table => writeln!(out, "{:.4}\t{}", hit.score, hit.id)?,
        }
    }
    Ok(())
}

fn cmd_reindex(store: &Store, store_path: &Path, args: &ReindexArgs) -> Result<(), CliError> {
    use singularmem_search::Index;

    // Phase 1: Tantivy lexical reindex (always).
    let index_path = derive_index_path(store_path);
    let index = Index::open(&index_path).map_err(|e| CliError::IndexOpen(e.to_string()))?;
    let progress = |n: u64| {
        if !args.quiet {
            tracing::info!("reindex (tantivy): {n} items processed");
        }
    };
    let count = index
        .reindex_from(store.list()?.filter_map(Result::ok), progress)
        .map_err(|e| CliError::IndexOpen(e.to_string()))?;
    tracing::info!("reindex (tantivy): {count} items total");

    // Phase 2: Embedder / vector reindex (only when --with-embeddings is given).
    if args.with_embeddings {
        let vectors_path = derive_vectors_path(store_path);

        if args.reset_vectors {
            if !args.force {
                return Err(CliError::Usage(
                    "--reset-vectors requires --force to confirm the destructive operation".into(),
                ));
            }
            if vectors_path.exists() {
                std::fs::remove_dir_all(&vectors_path).map_err(CliError::Io)?;
                tracing::warn!(
                    path = %vectors_path.display(),
                    "deleted existing vector index"
                );
            }
        }

        let model = match args.embedding_model.as_str() {
            "all-mini-lm-l6-v2" => singularmem_search::EmbeddingModel::AllMiniLmL6V2,
            "bge-small-en" => singularmem_search::EmbeddingModel::BgeSmallEnV15,
            "nomic-embed" => singularmem_search::EmbeddingModel::NomicEmbedTextV15,
            other => {
                return Err(CliError::Usage(format!(
                    "unknown --embedding-model: {other}"
                )))
            }
        };

        let embedder: Box<dyn singularmem_search::Embedder> =
            match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                _ => Box::new(
                    singularmem_search::FastembedEmbedder::with_model(model)
                        .map_err(|e| CliError::IndexOpen(format!("embedder init: {e}")))?,
                ),
            };

        let embedder_idx = singularmem_search::EmbedderIndex::open(&vectors_path, embedder)
            .map_err(|e| CliError::IndexOpen(e.to_string()))?;

        for (i, item_r) in store.list()?.enumerate() {
            let item = item_r?;
            singularmem_core::IndexHook::on_reindex(&embedder_idx, &item)
                .map_err(|e| CliError::IndexOpen(e.to_string()))?;
            if !args.quiet && (i + 1) % 100 == 0 {
                tracing::info!("reindex (embeddings): {} items", i + 1);
            }
        }
        singularmem_core::IndexHook::commit(&embedder_idx)
            .map_err(|e| CliError::IndexOpen(e.to_string()))?;
        tracing::info!("reindex (embeddings) complete");
    }

    Ok(())
}
