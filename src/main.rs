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
    /// Retrieve memory blocks formatted for an LLM prompt.
    Retrieve(RetrieveArgs),
    /// \[DEPRECATED\] Semantic (vector) search. Use `search --mode semantic`.
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

#[derive(Args, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct RetrieveArgs {
    /// One or more query tokens. Multiple tokens are joined with a space
    /// before being passed to the underlying hybrid search.
    queries: Vec<String>,
    /// Which adapter to use for formatting. Defaults to `plain`.
    /// Sub-projects 3b/3c/3d add `claude`, `openai`, `gemini` to the registry.
    #[arg(short = 'a', long, default_value = "plain")]
    adapter: String,
    /// Max memory blocks to include in the formatted output.
    #[arg(short = 'l', long, default_value = "10")]
    limit: usize,
    /// Minimum score for a hit to be included.
    #[arg(long, default_value = "0.0")]
    min_score: f32,
    /// Underlying search mode (passed through to `HybridSearcher`).
    #[arg(short = 'm', long, value_enum, default_value_t = SearchMode::Auto)]
    mode: SearchMode,
    /// Per-ranker overfetch factor (hybrid only).
    #[arg(long, default_value = "3")]
    fetch_multiplier: usize,
    /// RRF damping constant (hybrid only).
    #[arg(long, default_value = "60")]
    rrf_k: usize,
    /// Emit `RetrievedContext` as JSON instead of adapter-formatted output.
    #[arg(long)]
    json: bool,
    /// Print "Retrieved N blocks in Xms" to stderr after the formatted output.
    #[arg(long)]
    show_elapsed: bool,
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
        Err(CliError::Search(
            e @ (singularmem_search::Error::NoIndexes
            | singularmem_search::Error::HybridMissingIndex { .. }
            | singularmem_search::Error::IndexMissing { .. }),
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
                    | singularmem_search::Error::IndexMissing { .. },
                )
                | singularmem_retrieve::Error::Core(singularmem_core::Error::NotFound { .. }) => 2,
                _ => 1,
            };
            eprintln!("singularmem: {e}");
            ExitCode::from(code)
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
    #[error("{0}")]
    Search(#[from] singularmem_search::Error),
    #[error("{0}")]
    Retrieve(#[from] singularmem_retrieve::Error),
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

/// Registry of available adapters. Sub-projects 3b/3c/3d each add one line
/// here AND one line to the root `Cargo.toml` `[dependencies]` section.
///
/// Order matters for the unknown-adapter error message: list adapters in
/// the order they should appear when the CLI tells the user what's
/// available.
fn known_adapters() -> Vec<Box<dyn singularmem_retrieve::Adapter>> {
    vec![
        Box::new(singularmem_retrieve::PlainAdapter),
        Box::new(singularmem_adapter_claude::ClaudeAdapter),
        Box::new(singularmem_adapter_openai::OpenAiAdapter),
        // 3d will add: Box::new(singularmem_adapter_gemini::GeminiAdapter),
    ]
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
        Command::Retrieve(args) => cmd_retrieve(store, store_path, &args),
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

/// Result of resolving a `SearchMode` for a given store path. Returned by
/// `resolve_search_mode`.
struct ResolvedSearchMode {
    /// The concrete search mode (never `Auto` after resolution).
    mode: SearchMode,
    /// Tantivy sidecar path.
    tantivy_path: PathBuf,
    /// Vectors sidecar path.
    vectors_path: PathBuf,
}

/// Probe the store's sidecar directories and resolve `requested_mode`
/// (which may be `Auto`) into a concrete mode (`Lexical`, `Semantic`,
/// or `Hybrid`). Surfaces the same set of errors `cmd_search` does:
/// `NoIndexes` for auto + neither sidecar, `HybridMissingIndex` for
/// explicit hybrid + one missing, `IndexMissing` for explicit
/// lexical/semantic + that sidecar missing.
fn resolve_search_mode(
    store_path: &Path,
    requested_mode: SearchMode,
) -> Result<ResolvedSearchMode, CliError> {
    let tantivy_path = derive_index_path(store_path);
    let vectors_path = derive_vectors_path(store_path);
    let has_lexical = tantivy_path.exists();
    let has_vectors = vectors_path.exists();

    // Resolve --mode auto → concrete mode (or NoIndexes error).
    let resolved = match requested_mode {
        SearchMode::Auto => match (has_lexical, has_vectors) {
            (true, true) => SearchMode::Hybrid,
            (true, false) => {
                tracing::info!(
                    path = %vectors_path.display(),
                    "no vector index; using lexical-only search"
                );
                SearchMode::Lexical
            }
            (false, true) => {
                tracing::info!(
                    path = %tantivy_path.display(),
                    "no lexical index; using semantic-only search"
                );
                SearchMode::Semantic
            }
            (false, false) => return Err(CliError::Search(singularmem_search::Error::NoIndexes)),
        },
        m => m,
    };

    // Explicit-mode pre-flight checks (Auto bypassed via the degradation above).
    match resolved {
        SearchMode::Hybrid => {
            if !has_lexical {
                return Err(CliError::Search(
                    singularmem_search::Error::HybridMissingIndex {
                        missing: "lexical",
                        path: tantivy_path,
                    },
                ));
            }
            if !has_vectors {
                return Err(CliError::Search(
                    singularmem_search::Error::HybridMissingIndex {
                        missing: "semantic",
                        path: vectors_path,
                    },
                ));
            }
        }
        SearchMode::Lexical if !has_lexical => {
            return Err(CliError::Search(singularmem_search::Error::IndexMissing {
                path: tantivy_path,
            }));
        }
        SearchMode::Semantic if !has_vectors => {
            return Err(CliError::Search(singularmem_search::Error::IndexMissing {
                path: vectors_path,
            }));
        }
        _ => {}
    }

    Ok(ResolvedSearchMode {
        mode: resolved,
        tantivy_path,
        vectors_path,
    })
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
    use singularmem_search::{EmbedderIndex, HybridSearchOptions, HybridSearcher, Index};

    let resolved = resolve_search_mode(store_path, args.mode)?;
    let ResolvedSearchMode {
        mode: resolved_mode,
        tantivy_path,
        vectors_path,
    } = resolved;

    let query_str = args.queries.join(" ");
    let opts = HybridSearchOptions {
        limit: args.limit,
        fetch_multiplier: args.fetch_multiplier,
        rrf_k: args.rrf_k,
        include_snippets: !args.no_snippets,
    };

    // Open whichever indexes the resolved mode requires.
    let lex_opt: Option<Index> =
        if matches!(resolved_mode, SearchMode::Lexical | SearchMode::Hybrid) {
            Some(Index::open(&tantivy_path)?)
        } else {
            None
        };
    let sem_opt: Option<EmbedderIndex> =
        if matches!(resolved_mode, SearchMode::Semantic | SearchMode::Hybrid) {
            let embedder: Box<dyn singularmem_search::Embedder> =
                match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                    Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                    _ => Box::new(singularmem_search::FastembedEmbedder::new()?),
                };
            Some(EmbedderIndex::open(&vectors_path, embedder)?)
        } else {
            None
        };

    let searcher = match (&lex_opt, &sem_opt) {
        (Some(l), Some(s)) => HybridSearcher::new(l, s),
        (Some(l), None) => HybridSearcher::lexical_only(l),
        (None, Some(s)) => HybridSearcher::semantic_only(s),
        (None, None) => unreachable!("pre-flight guarantees at least one index"),
    };
    let results = searcher.search(&query_str, &opts)?;

    render_search_results(&results, args)?;
    Ok(())
}

fn render_search_results(
    results: &singularmem_search::HybridSearchResults,
    args: &SearchArgs,
) -> Result<(), CliError> {
    use singularmem_search::ScoreKind;

    if results.hits.is_empty() {
        tracing::info!("0 matches");
        return Ok(());
    }

    let mut out = io::stdout().lock();
    if args.json {
        serde_json::to_writer(&mut out, results)?;
        writeln!(out)?;
        return Ok(());
    }

    for hit in &results.hits {
        let tag = match hit.score_kind {
            ScoreKind::Rrf => "rrf",
            ScoreKind::Bm25 => "bm25",
            ScoreKind::Cosine => "cos",
        };
        let snip = hit.snippet.as_deref().unwrap_or("").replace('\n', " ");
        if args.show_ranks {
            let lex = hit
                .lexical_rank
                .map_or_else(|| "—".to_string(), |r| r.to_string());
            let sem = hit
                .semantic_rank
                .map_or_else(|| "—".to_string(), |r| r.to_string());
            writeln!(
                out,
                "{}  {}={:.4}  lex={}  sem={}  {}",
                hit.id, tag, hit.score, lex, sem, snip
            )?;
        } else {
            writeln!(out, "{}  {}={:.4}  {}", hit.id, tag, hit.score, snip)?;
        }
    }
    Ok(())
}

fn cmd_retrieve(store: &Store, store_path: &Path, args: &RetrieveArgs) -> Result<(), CliError> {
    use singularmem_retrieve::{Adapter, RetrieveOptions, Retriever};
    use singularmem_search::{EmbedderIndex, HybridSearchOptions, HybridSearcher, Index};

    // Adapter lookup before any I/O so unknown-adapter errors fail fast.
    let adapters = known_adapters();
    let adapter: &dyn Adapter = adapters
        .iter()
        .find(|a| a.name() == args.adapter.as_str())
        .map(std::convert::AsRef::as_ref)
        .ok_or_else(|| {
            let known: Vec<&str> = adapters.iter().map(|a| a.name()).collect();
            CliError::Usage(format!(
                "unknown adapter '{}'; known adapters: {}",
                args.adapter,
                known.join(", ")
            ))
        })?;

    // Mode resolution + sidecar probing — same helper cmd_search uses.
    let ResolvedSearchMode {
        mode: resolved_mode,
        tantivy_path,
        vectors_path,
    } = resolve_search_mode(store_path, args.mode)?;

    let query_str = args.queries.join(" ");
    let search_opts = HybridSearchOptions {
        limit: args
            .limit
            .saturating_mul(args.fetch_multiplier)
            .max(args.limit),
        fetch_multiplier: args.fetch_multiplier,
        rrf_k: args.rrf_k,
        include_snippets: false, // we use full content, not snippets
    };
    let opts = RetrieveOptions {
        max_blocks: args.limit,
        min_score: args.min_score,
        search: search_opts,
    };

    // Open whichever indexes the resolved mode requires.
    let lex_opt: Option<Index> =
        if matches!(resolved_mode, SearchMode::Lexical | SearchMode::Hybrid) {
            Some(Index::open(&tantivy_path)?)
        } else {
            None
        };
    let sem_opt: Option<EmbedderIndex> =
        if matches!(resolved_mode, SearchMode::Semantic | SearchMode::Hybrid) {
            let embedder: Box<dyn singularmem_search::Embedder> =
                match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                    Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                    _ => Box::new(singularmem_search::FastembedEmbedder::new()?),
                };
            Some(EmbedderIndex::open(&vectors_path, embedder)?)
        } else {
            None
        };

    let searcher = match (&lex_opt, &sem_opt) {
        (Some(l), Some(s)) => HybridSearcher::new(l, s),
        (Some(l), None) => HybridSearcher::lexical_only(l),
        (None, Some(s)) => HybridSearcher::semantic_only(s),
        (None, None) => unreachable!("pre-flight guarantees at least one index"),
    };
    let retriever = Retriever::new(store, &searcher);
    let context = retriever.retrieve(&query_str, &opts)?;

    let mut out = io::stdout().lock();
    if args.json {
        serde_json::to_writer(&mut out, &context)?;
        writeln!(out)?;
    } else {
        let formatted = adapter.format(&context);
        write!(out, "{formatted}")?;
    }
    drop(out);

    if args.show_elapsed {
        eprintln!(
            "Retrieved {} blocks in {:.2}ms (considered {})",
            context.blocks.len(),
            context.elapsed.as_secs_f64() * 1000.0,
            context.total_considered
        );
    }
    Ok(())
}

fn cmd_semantic_search(store_path: &Path, args: &SemanticSearchArgs) -> Result<(), CliError> {
    use std::sync::OnceLock;
    static DEPRECATION_NOTICE: OnceLock<()> = OnceLock::new();
    DEPRECATION_NOTICE.get_or_init(|| {
        eprintln!("note: 'semantic-search' is deprecated; use 'search --mode semantic'");
    });

    // Forward through cmd_search with mode=Semantic.
    let forwarded = SearchArgs {
        queries: args.queries.clone(),
        mode: SearchMode::Semantic,
        limit: args.limit,
        offset: 0,
        fetch_multiplier: 3,
        rrf_k: 60,
        no_snippets: true, // semantic mode has no snippets anyway
        show_ranks: false,
        json: matches!(args.format, ListFormat::Jsonl),
        format: args.format,
    };
    cmd_search(store_path, &forwarded)
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
