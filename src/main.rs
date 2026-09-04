//! Singularmem CLI — thin shell over `singularmem_core`.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use singularmem_core::{Error, ItemId, NewItem, ScopeFilter, Store, StoreOptions};

#[derive(Parser, Debug)]
#[command(
    name = "singularmem",
    version,
    about = "Local-first persistent memory layer for LLM workflows."
)]
struct Cli {
    /// Path to the `SQLite` store file. Defaults to the per-user XDG data dir.
    /// Also settable via the `SINGULARMEM_STORE` environment variable —
    /// the only way to point a hook (which runs with a fixed, flag-less
    /// command line) at a non-default store.
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
    /// Bulk-ingest Claude Code JSONL transcripts (idempotent).
    IngestTranscript(IngestTranscriptArgs),
    /// Bulk-ingest `OpenAI` Codex CLI rollout JSONL files (idempotent).
    IngestCodex(IngestCodexArgs),
    /// Bulk-ingest Cursor chat history from its per-user `state.vscdb` stores (idempotent).
    IngestCursor(IngestCursorArgs),
    /// Bulk-ingest a source tree, honouring .gitignore (idempotent).
    IngestDir(IngestDirArgs),
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
    /// Inspect and change item scopes.
    Scope(ScopeCommand),
    /// Print the newest items across a project's editor scopes.
    #[command(name = "wake-up")]
    WakeUp(WakeUpArgs),
    /// Run an editor hook event on stdin (never fails the calling editor).
    Hook(HookArgs),
    /// Install, uninstall, or inspect editor hook wiring.
    Hooks(HooksCommand),
}

#[derive(Args, Debug)]
struct HookArgs {
    /// The editor invoking the hook: `claude-code`, `codex`, or `cursor`.
    editor: String,
    /// The hook event: `session-start`, `stop`, `pre-compact`, or `session-end`.
    event: String,
}

#[derive(Args, Debug)]
struct HooksCommand {
    #[command(subcommand)]
    action: HooksAction,
}

#[derive(Subcommand, Debug)]
enum HooksAction {
    /// Wire this editor's hooks to the current binary.
    Install {
        /// The editor: `claude-code`, `codex`, or `cursor`.
        editor: String,
        /// Write to the project-local config instead of the user's home directory.
        #[arg(long)]
        project: bool,
        /// Print the merged config to stdout instead of writing it.
        #[arg(long)]
        print: bool,
    },
    /// Remove this editor's hooks, leaving everything else untouched.
    Uninstall {
        /// The editor: `claude-code`, `codex`, or `cursor`.
        editor: String,
        /// Remove from the project-local config instead of the user's home directory.
        #[arg(long)]
        project: bool,
    },
    /// Show whether hooks are installed for one editor, or all three.
    Status {
        /// The editor to check; every editor when omitted.
        editor: Option<String>,
        /// Read the project-local config instead of the user's home directory.
        #[arg(long)]
        project: bool,
    },
}

/// Shared `--scope`/`--scope-exact` flags, flattened into `ListArgs`,
/// `SearchArgs`, and `RetrieveArgs`.
#[derive(Args, Debug, Clone)]
struct ScopeArgs {
    /// Restrict to this scope and its descendants (e.g. `claude-code/myproj`).
    #[arg(long, value_name = "PATH")]
    scope: Option<String>,
    /// With --scope: match only that exact scope, not descendants.
    #[arg(long)]
    scope_exact: bool,
}

impl ScopeArgs {
    fn to_filter(&self) -> Result<Option<ScopeFilter>, CliError> {
        match (&self.scope, self.scope_exact) {
            (None, true) => Err(CliError::Usage("--scope-exact requires --scope".into())),
            (None, false) => Ok(None),
            (Some(p), true) => Ok(Some(ScopeFilter::exact(p)?)),
            (Some(p), false) => Ok(Some(ScopeFilter::descendants(p)?)),
        }
    }
}

#[derive(Args, Debug)]
struct ScopeCommand {
    #[command(subcommand)]
    action: ScopeAction,
}

#[derive(Subcommand, Debug)]
enum ScopeAction {
    /// List every scope with its item count, sorted by path.
    List,
    /// Move one item to PATH (or clear its scope with `-`).
    Move {
        /// The item ID (26-char ULID, case-insensitive).
        id: String,
        /// Destination scope path, or `-` to clear it.
        path: String,
    },
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
    /// Assign this scope path to the item (e.g. `team/backend`).
    #[arg(long, value_name = "PATH")]
    scope: Option<String>,
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
struct IngestTranscriptArgs {
    /// Transcript files or directories (searched recursively for *.jsonl).
    /// Defaults to ~/.claude/projects.
    paths: Vec<PathBuf>,
    /// Keep only messages whose working directory equals DIR.
    #[arg(long, value_name = "DIR")]
    project: Option<PathBuf>,
    /// Keep subagent (sidechain) messages.
    #[arg(long)]
    include_sidechains: bool,
    /// Parse and report; write nothing.
    #[arg(long)]
    dry_run: bool,
    /// Suppress per-file progress lines.
    #[arg(long)]
    quiet: bool,
    /// Override the default `claude-code/<project>` scope for every ingested item.
    #[arg(long, value_name = "PATH")]
    scope: Option<String>,
}

#[derive(Args, Debug)]
struct IngestCodexArgs {
    /// Rollout files or directories (searched recursively for
    /// `rollout-*.jsonl`). Defaults to `~/.codex/sessions`.
    paths: Vec<PathBuf>,
    /// Keep only messages whose session `cwd` equals DIR.
    #[arg(long, value_name = "DIR")]
    project: Option<PathBuf>,
    /// Parse and report; write nothing.
    #[arg(long)]
    dry_run: bool,
    /// Suppress per-file progress lines.
    #[arg(long)]
    quiet: bool,
    /// Override the default `codex/<project>` scope for every ingested item.
    #[arg(long, value_name = "PATH")]
    scope: Option<String>,
}

#[derive(Args, Debug)]
struct IngestCursorArgs {
    /// Cursor's per-user directory (contains `globalStorage/` and
    /// `workspaceStorage/`). Defaults to the per-OS Cursor `User` dir.
    #[arg(long, value_name = "DIR")]
    cursor_dir: Option<PathBuf>,
    /// Keep only conversations whose workspace folder equals DIR.
    #[arg(long, value_name = "DIR")]
    project: Option<PathBuf>,
    /// Keep only this composer (conversation) id.
    #[arg(long)]
    conversation: Option<String>,
    /// Parse and report; write nothing.
    #[arg(long)]
    dry_run: bool,
    /// Suppress per-file progress lines.
    #[arg(long)]
    quiet: bool,
    /// Override the default `cursor/<project>` scope for every ingested item.
    #[arg(long, value_name = "PATH")]
    scope: Option<String>,
}

#[derive(Args, Debug)]
struct IngestDirArgs {
    /// Root directory to walk.
    path: PathBuf,
    /// Skip files larger than this many bytes.
    #[arg(long, default_value_t = singularmem_ingest::DEFAULT_MAX_FILE_BYTES)]
    max_file_bytes: u64,
    /// Parse and report; write nothing.
    #[arg(long)]
    dry_run: bool,
    /// Suppress per-file progress lines.
    #[arg(long)]
    quiet: bool,
    /// Override the default `files/<dirname>` scope for every ingested item.
    #[arg(long, value_name = "PATH")]
    scope: Option<String>,
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
    #[command(flatten)]
    scope: ScopeArgs,
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
    #[command(flatten)]
    scope: ScopeArgs,
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
    #[command(flatten)]
    scope: ScopeArgs,
}

#[derive(Args, Debug)]
struct WakeUpArgs {
    /// Restrict to this scope and its descendants (repeatable; OR-ed).
    /// Defaults to the editor scopes for `--project` (or the current
    /// directory) when omitted.
    #[arg(long = "scope", value_name = "PATH")]
    scope: Vec<String>,
    /// Project directory whose basename derives the default scopes.
    /// Defaults to the current directory. Ignored when `--scope` is given.
    #[arg(long, value_name = "DIR")]
    project: Option<PathBuf>,
    /// Also include the `files/<project>` scope in the default scope set.
    #[arg(long)]
    include_files: bool,
    /// Newest items to include.
    #[arg(long, default_value_t = 20)]
    limit: usize,
    /// Rendered byte budget; oldest blocks are dropped first to fit
    /// (applies to the rendered text; hook envelopes add a few bytes of
    /// JSON).
    #[arg(long, default_value_t = 8192)]
    max_bytes: usize,
    /// Which adapter to use for formatting.
    #[arg(short = 'a', long, default_value = "plain")]
    adapter: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = WakeUpFormat::Text)]
    format: WakeUpFormat,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum WakeUpFormat {
    /// Header plus adapter-formatted blocks, on stdout.
    Text,
    /// `{ scopes, total, shown, blocks, text }`.
    Json,
    /// Claude Code's `SessionStart` hook envelope.
    #[value(name = "claude-hook")]
    ClaudeHook,
    /// Codex's `SessionStart` hook envelope.
    #[value(name = "codex-hook")]
    CodexHook,
    /// Cursor's `SessionStart` hook envelope.
    #[value(name = "cursor-hook")]
    CursorHook,
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
        Err(e @ CliError::StoreReadOnly) => {
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

/// Resolve the store path from (in order of precedence) `--store`, the
/// `SINGULARMEM_STORE` environment variable (the only way to point a hook —
/// a fixed, flag-less command line — at a non-default store), and the
/// per-user XDG default.
fn resolve_store_path(cli: &Cli) -> PathBuf {
    cli.store
        .clone()
        .or_else(|| std::env::var_os("SINGULARMEM_STORE").map(PathBuf::from))
        .unwrap_or_else(default_store_path)
}

/// Wire up the Tantivy (and, opt-in, vector) `IndexHook`s on `store` so live
/// writes populate the search sidecars. A no-op when `no_index` is set.
///
/// Shared by the ingest verbs (`run`) and `cmd_hook_entry`'s save-event path
/// — `session-start` is read-only and never needs this.
fn wire_index_hooks(store: &mut Store, store_path: &Path, no_index: bool) {
    if no_index {
        return;
    }

    let mut hooks: Vec<Box<dyn singularmem_core::IndexHook>> = Vec::new();

    // Tantivy lexical-search hook (sub-project 2a behaviour — always attempt).
    let index_path = derive_index_path(store_path);
    match singularmem_search::Index::open(&index_path) {
        Ok(idx) => hooks.push(Box::new(idx)),
        Err(e) => tracing::warn!(
            error = %e,
            path = %index_path.display(),
            "could not open Tantivy index; lexical search will not work until reindex"
        ),
    }

    // Embedder / vector hook — opt-in: only when .vectors/ already exists.
    let vectors_path = derive_vectors_path(store_path);
    if vectors_path.exists() {
        let embedder: Option<Box<dyn singularmem_search::Embedder>> =
            match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                Some("mock") => Some(Box::new(
                    singularmem_search::testing::MockEmbedder::default(),
                )),
                _ => match singularmem_search::FastembedEmbedder::new() {
                    Ok(e) => Some(Box::new(e)),
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "embedder construction failed; semantic search will not work"
                        );
                        None
                    }
                },
            };
        if let Some(embedder) = embedder {
            match singularmem_search::EmbedderIndex::open(&vectors_path, embedder) {
                Ok(idx) => hooks.push(Box::new(idx)),
                Err(e) => tracing::warn!(
                    error = %e,
                    "vector index open failed; semantic search will not work"
                ),
            }
        }
    }

    if !hooks.is_empty() {
        store.set_hook(Some(Box::new(singularmem_core::hook::MultiHook::new(
            hooks,
        ))));
    }
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
        Box::new(singularmem_adapter_gemini::GeminiAdapter),
    ]
}

/// Look up an adapter by name in [`known_adapters`].
///
/// # Errors
/// `CliError::Usage` naming the known adapters when `name` matches none.
fn find_adapter(name: &str) -> Result<Box<dyn singularmem_retrieve::Adapter>, CliError> {
    let adapters = known_adapters();
    let Some(pos) = adapters.iter().position(|a| a.name() == name) else {
        let known: Vec<&str> = adapters.iter().map(|a| a.name()).collect();
        return Err(CliError::Usage(format!(
            "unknown adapter '{name}'; known adapters: {}",
            known.join(", ")
        )));
    };
    Ok(adapters
        .into_iter()
        .nth(pos)
        .unwrap_or_else(|| unreachable!()))
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
    item.scope = args.scope;
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

/// Ingest each file in `files` by opening it with `open` (which also
/// configures the resulting source), accumulating counts into `total` and
/// unopenable files into `failed_files`. Per-file open failures are warned
/// and counted; a store-level failure returns immediately.
fn ingest_files<S: singularmem_ingest::Source>(
    store: &Store,
    files: &[PathBuf],
    dry_run: bool,
    quiet: bool,
    open: impl Fn(&Path) -> singularmem_ingest::Result<S>,
    total: &mut singularmem_ingest::Report,
    failed_files: &mut usize,
) -> Result<(), CliError> {
    use singularmem_ingest::ingest_source;

    for file in files {
        let src = match open(file) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(path = %file.display(), error = %e, "cannot open source");
                *failed_files += 1;
                continue;
            }
        };
        let r = ingest_source(store, &src, dry_run)?;
        if !quiet {
            eprintln!(
                "{}: +{} ingested, {} skipped",
                file.display(),
                r.ingested,
                r.skipped_existing + r.skipped_filtered
            );
        }
        accumulate(total, r);
    }
    Ok(())
}

/// Resolve the directories/files a bulk-ingest command should scan: `paths`
/// if non-empty, otherwise a single default root. Each root is expanded
/// to files via `discover`; a root that is neither a directory nor a file
/// is `Error::NotFound`.
///
/// `default_root` is a closure rather than an eager `PathBuf` and is called
/// only when `paths` is empty, so a command given explicit paths never needs
/// `HOME` (or whatever the default root depends on) to be resolvable.
fn resolve_ingest_files(
    paths: &[PathBuf],
    default_root: impl FnOnce() -> Result<PathBuf, CliError>,
    discover: impl Fn(&Path) -> singularmem_ingest::Result<Vec<PathBuf>>,
) -> Result<Vec<PathBuf>, CliError> {
    let roots: Vec<PathBuf> = if paths.is_empty() {
        vec![default_root()?]
    } else {
        paths.to_vec()
    };

    let mut files: Vec<PathBuf> = Vec::new();
    for root in &roots {
        if root.is_dir() {
            files.extend(discover(root)?);
        } else if root.is_file() {
            files.push(root.clone());
        } else {
            return Err(singularmem_ingest::Error::NotFound { path: root.clone() }.into());
        }
    }
    Ok(files)
}

/// Canonicalise `project`, falling back to the given path unchanged when
/// canonicalisation fails (e.g. it does not exist).
fn canonicalize_project(project: Option<&PathBuf>) -> Option<PathBuf> {
    let p = project?;
    Some(p.canonicalize().unwrap_or_else(|_| p.clone()))
}

fn cmd_ingest_transcript(store: &Store, args: &IngestTranscriptArgs) -> Result<(), CliError> {
    use singularmem_ingest::{discover_transcripts, ClaudeTranscript, Report};

    // Validate --scope up front so a typo is a usage error before any
    // parsing or filesystem work happens.
    let scope_override = args
        .scope
        .as_deref()
        .map(singularmem_core::scope::validate)
        .transpose()?;

    let files = resolve_ingest_files(
        &args.paths,
        || {
            Ok(dirs::home_dir()
                .ok_or_else(|| CliError::Usage("cannot determine home directory".into()))?
                .join(".claude")
                .join("projects"))
        },
        |p| discover_transcripts(p),
    )?;

    let project = canonicalize_project(args.project.as_ref());
    let mut total = Report::default();
    let mut failed_files = 0usize;
    // A store-level failure mid-loop still gets a summary for the files
    // already processed before it propagates.
    let outcome = ingest_files(
        store,
        &files,
        args.dry_run,
        args.quiet,
        |file| {
            let mut s = ClaudeTranscript::open(file)?;
            s.include_sidechains = args.include_sidechains;
            s.project_filter.clone_from(&project);
            s.scope_override.clone_from(&scope_override);
            Ok(s)
        },
        &mut total,
        &mut failed_files,
    );
    total.failed += failed_files;
    print_summary(&total, files.len());
    outcome?;
    if total.failed > 0 {
        return Err(CliError::IngestPartial {
            failed: total.failed,
        });
    }
    Ok(())
}

fn cmd_ingest_codex(store: &Store, args: &IngestCodexArgs) -> Result<(), CliError> {
    use singularmem_ingest::{discover_codex_sessions, CodexRollout, Report};

    // Validate --scope up front so a typo is a usage error before any
    // parsing or filesystem work happens.
    let scope_override = args
        .scope
        .as_deref()
        .map(singularmem_core::scope::validate)
        .transpose()?;

    let files = resolve_ingest_files(
        &args.paths,
        || codex_root().ok_or_else(|| CliError::Usage("cannot determine home directory".into())),
        |p| discover_codex_sessions(p),
    )?;

    let project = canonicalize_project(args.project.as_ref());
    let mut total = Report::default();
    let mut failed_files = 0usize;
    let outcome = ingest_files(
        store,
        &files,
        args.dry_run,
        args.quiet,
        |file| {
            let mut s = CodexRollout::open(file)?;
            s.project_filter.clone_from(&project);
            s.scope_override.clone_from(&scope_override);
            Ok(s)
        },
        &mut total,
        &mut failed_files,
    );
    total.failed += failed_files;
    print_summary(&total, files.len());
    outcome?;
    if total.failed > 0 {
        return Err(CliError::IngestPartial {
            failed: total.failed,
        });
    }
    Ok(())
}

fn cmd_ingest_cursor(store: &Store, args: &IngestCursorArgs) -> Result<(), CliError> {
    use singularmem_ingest::{default_cursor_user_dir, ingest_source, CursorChats};

    // Validate --scope up front so a typo is a usage error before any
    // parsing or filesystem work happens.
    let scope_override = args
        .scope
        .as_deref()
        .map(singularmem_core::scope::validate)
        .transpose()?;

    let dir = match &args.cursor_dir {
        Some(d) => d.clone(),
        None => default_cursor_user_dir()
            .ok_or_else(|| CliError::Usage("cannot determine home directory".into()))?,
    };

    let mut src = CursorChats::open(&dir)?;
    src.project_filter = canonicalize_project(args.project.as_ref());
    src.conversation_filter.clone_from(&args.conversation);
    src.scope_override = scope_override;

    let r = ingest_source(store, &src, args.dry_run)?;
    if !args.quiet {
        eprintln!(
            "{}: +{} ingested, {} skipped",
            dir.display(),
            r.ingested,
            r.skipped_existing + r.skipped_filtered
        );
    }
    print_summary(&r, 1);
    if r.failed > 0 {
        return Err(CliError::IngestPartial { failed: r.failed });
    }
    Ok(())
}

fn cmd_ingest_dir(store: &Store, args: &IngestDirArgs) -> Result<(), CliError> {
    use singularmem_ingest::{ingest_source, DirectoryWalker};

    // Validate --scope up front so a typo is a usage error before any
    // filesystem walking happens.
    let scope_override = args
        .scope
        .as_deref()
        .map(singularmem_core::scope::validate)
        .transpose()?;

    let mut src = DirectoryWalker::new(&args.path)?;
    src.max_file_bytes = args.max_file_bytes;
    src.scope_override = scope_override;
    let r = ingest_source(store, &src, args.dry_run)?;
    if !args.quiet {
        eprintln!(
            "{}: +{} ingested, {} skipped",
            src.root.display(),
            r.ingested,
            r.skipped_existing + r.skipped_filtered
        );
    }
    print_summary(&r, src.visited_files());
    if r.failed > 0 {
        return Err(CliError::IngestPartial { failed: r.failed });
    }
    Ok(())
}

fn accumulate(total: &mut singularmem_ingest::Report, r: singularmem_ingest::Report) {
    total.ingested += r.ingested;
    total.skipped_existing += r.skipped_existing;
    total.skipped_filtered += r.skipped_filtered;
    total.failed += r.failed;
}

fn print_summary(r: &singularmem_ingest::Report, files: usize) {
    eprintln!(
        "ingested {}, skipped {} existing, {} filtered, {} failed across {} files",
        r.ingested, r.skipped_existing, r.skipped_filtered, r.failed, files
    );
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
    let filter = args.scope.to_filter()?;
    let iter: Box<dyn Iterator<Item = singularmem_core::Result<singularmem_core::Item>>> =
        Box::new(store.list_by_tags_scoped(&tag_refs, filter.as_ref())?);

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

fn cmd_scope(store: &Store, cmd: &ScopeCommand) -> Result<(), CliError> {
    let mut out = io::stdout().lock();
    match &cmd.action {
        ScopeAction::List => {
            for (path, count) in store.scopes()? {
                writeln!(out, "{path}\t{count}")?;
            }
        }
        ScopeAction::Move { id, path } => {
            let id = id.parse::<ItemId>()?;
            let scope = if path == "-" {
                None
            } else {
                Some(path.as_str())
            };
            let item = store.set_scope(id, scope)?;
            writeln!(out, "{}", item.id)?;
            eprintln!("note: search indexes keep the previous scope until `singularmem reindex`");
        }
    }
    Ok(())
}

fn cmd_wake_up(store: &Store, args: &WakeUpArgs) -> Result<(), CliError> {
    use singularmem_retrieve::wakeup::{build, render, ScopeSet, WakeupOptions};

    let set = if args.scope.is_empty() {
        // Pass `--project` through **raw**. The save side derives
        // `<editor>/<basename of the cwd the editor reported>`, so
        // canonicalising here would look up a different scope than the one
        // written for a symlinked project directory. `ScopeSet::for_project`
        // canonicalises internally only when the raw basename is unusable
        // (`.`, `..`), which is what makes `--project .` work.
        let dir = match &args.project {
            Some(p) => p.clone(),
            None => std::env::current_dir()?,
        };
        ScopeSet::for_project(&dir, args.include_files)
    } else {
        ScopeSet(
            args.scope
                .iter()
                .map(|s| singularmem_core::ScopeFilter::descendants(s))
                .collect::<Result<_, _>>()?,
        )
    };
    let adapter = find_adapter(&args.adapter)?;
    let w = build(
        store,
        &set,
        &WakeupOptions {
            limit: args.limit,
            max_bytes: args.max_bytes,
        },
    )?;
    let text = render(&w, &*adapter, args.max_bytes);

    let mut out = io::stdout().lock();
    match args.format {
        WakeUpFormat::Text => write!(out, "{text}")?,
        WakeUpFormat::Json => {
            serde_json::to_writer(
                &mut out,
                &serde_json::json!({
                    "scopes": w.scopes,
                    "total": w.total,
                    "shown": w.shown,
                    "blocks": w.context.blocks,
                    "text": text,
                }),
            )?;
            writeln!(out)?;
        }
        WakeUpFormat::ClaudeHook => {
            write_hook_envelope(&mut out, singularmem_hooks::Editor::ClaudeCode, &text)?;
        }
        WakeUpFormat::CodexHook => {
            write_hook_envelope(&mut out, singularmem_hooks::Editor::Codex, &text)?;
        }
        WakeUpFormat::CursorHook => {
            write_hook_envelope(&mut out, singularmem_hooks::Editor::Cursor, &text)?;
        }
    }
    Ok(())
}

/// Write a session-start hook envelope for `editor` wrapping `text`, followed
/// by a trailing newline.
fn write_hook_envelope(
    out: &mut impl Write,
    editor: singularmem_hooks::Editor,
    text: &str,
) -> Result<(), CliError> {
    serde_json::to_writer(
        &mut *out,
        &singularmem_hooks::session_start_envelope(editor, text),
    )?;
    writeln!(out)?;
    Ok(())
}

/// Entry point for `hook <editor> <event>`, dispatched from [`run`] *before*
/// the store is opened. A hook must never fail the editor invoking it, so
/// this never returns an error: every failure — an unknown editor/event, a
/// store that cannot be opened, an ingest failure — is logged as a warning
/// on stderr and swallowed.
///
/// `session-start` still needs a valid envelope on stdout even when the
/// store cannot be opened, so it is special-cased to always print one (with
/// an empty context string on failure) rather than simply warning and doing
/// nothing.
#[allow(
    clippy::unnecessary_wraps,
    reason = "signature matches every other cmd_* dispatched from run_command"
)]
fn cmd_hook_entry(cli: &Cli, args: &HookArgs) -> Result<(), CliError> {
    use singularmem_hooks::{parse_input, Editor, Event};

    let Ok(editor) = args.editor.parse::<Editor>() else {
        tracing::warn!(editor = %args.editor, "unknown editor; hook does nothing");
        return Ok(());
    };
    let Ok(event) = args.event.parse::<Event>() else {
        tracing::warn!(event = %args.event, "unknown event; hook does nothing");
        return Ok(());
    };

    let mut raw = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut raw) {
        tracing::warn!(error = %e, "could not read hook input from stdin");
        raw.clear();
    }
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "could not parse hook input JSON; proceeding with empty input");
        serde_json::Value::Null
    });
    let input = parse_input(editor, &json);

    let store_path = resolve_store_path(cli);
    let store_result = Store::open_with_options(
        &store_path,
        StoreOptions {
            read_only: cli.read_only,
        },
    );

    match event {
        Event::SessionStart => match store_result {
            Ok(store) => {
                if let Err(e) = hook_session_start(&store, editor, &input) {
                    tracing::warn!(error = %e, editor = %args.editor, event = %args.event, "hook failed; editor continues");
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %store_path.display(),
                    "could not open store; emitting empty session-start context"
                );
                let mut out = io::stdout().lock();
                if let Err(e2) = write_hook_envelope(&mut out, editor, "") {
                    tracing::warn!(error = %e2, "failed to emit session-start envelope");
                }
            }
        },
        Event::Stop | Event::PreCompact | Event::SessionEnd => match store_result {
            Ok(mut store) => {
                // session-start is read-only; save events are the only ones
                // that write, so only they need the index hooks wired up.
                wire_index_hooks(&mut store, &store_path, cli.no_index);
                if let Err(e) = hook_save_event(&store, editor, &input) {
                    tracing::warn!(error = %e, editor = %args.editor, event = %args.event, "hook failed; editor continues");
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %store_path.display(),
                    editor = %args.editor,
                    event = %args.event,
                    "could not open store; nothing ingested"
                );
            }
        },
    }
    Ok(())
}

/// The `SessionStart` hook: build the project's wake-up context (scoped to
/// `input.cwd`, or the current directory when the editor sends none) and
/// print it wrapped in the editor's session-start envelope.
///
/// Cursor is the one editor that can have several workspace roots open in
/// the same window, and it reports all of them; the spec calls for the union
/// of every root's scopes there, rather than just the first one's (which is
/// all `input.cwd` carries). Duplicate scope paths — two roots with the same
/// basename — are collapsed so the scope set stays a set.
fn hook_session_start(
    store: &Store,
    editor: singularmem_hooks::Editor,
    input: &singularmem_hooks::HookInput,
) -> Result<(), CliError> {
    use singularmem_hooks::session_start_envelope;
    use singularmem_retrieve::wakeup::{build, render, ScopeSet, WakeupOptions};

    let set =
        if matches!(editor, singularmem_hooks::Editor::Cursor) && input.workspace_roots.len() > 1 {
            let mut seen = std::collections::HashSet::new();
            let mut filters = Vec::new();
            for root in &input.workspace_roots {
                for f in ScopeSet::for_project(root, false).0 {
                    if seen.insert(f.path.clone()) {
                        filters.push(f);
                    }
                }
            }
            ScopeSet(filters)
        } else {
            let dir = match &input.cwd {
                Some(p) => p.clone(),
                None => std::env::current_dir()?,
            };
            ScopeSet::for_project(&dir, false)
        };
    let opts = WakeupOptions::default();
    let text = match build(store, &set, &opts) {
        Ok(w) => render(&w, &singularmem_retrieve::PlainAdapter, opts.max_bytes),
        Err(e) => {
            tracing::warn!(error = %e, "wake-up failed; emitting empty context");
            String::new()
        }
    };
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &session_start_envelope(editor, &text))?;
    writeln!(out)?;
    Ok(())
}

/// The `Stop`/`PreCompact`/`SessionEnd` hooks: ingest whatever transcript(s)
/// the event identifies, per editor.
fn hook_save_event(
    store: &Store,
    editor: singularmem_hooks::Editor,
    input: &singularmem_hooks::HookInput,
) -> Result<(), CliError> {
    use singularmem_hooks::Editor;

    let report = match editor {
        Editor::ClaudeCode => hook_ingest_claude(store, input)?,
        Editor::Codex => hook_ingest_codex(store, input)?,
        Editor::Cursor => hook_ingest_cursor(store, input)?,
    };
    tracing::info!(
        ingested = report.ingested,
        skipped = report.skipped_existing,
        failed = report.failed,
        "hook ingest complete"
    );
    Ok(())
}

/// Claude Code always sends `transcript_path` on `Stop`/`PreCompact`/
/// `SessionEnd`; without it there is nothing to ingest.
fn hook_ingest_claude(
    store: &Store,
    input: &singularmem_hooks::HookInput,
) -> Result<singularmem_ingest::Report, CliError> {
    use singularmem_ingest::{ingest_source, ClaudeTranscript};

    let Some(p) = &input.transcript_path else {
        return Err(CliError::Usage("hook input has no transcript_path".into()));
    };
    let src = ClaudeTranscript::open(p)?;
    Ok(ingest_source(store, &src, false)?)
}

/// Resolve the Codex sessions root: the `SINGULARMEM_CODEX_ROOT` environment
/// variable when set, otherwise `default_codex_root()` (`~/.codex/sessions`).
/// Read by both the `hook`'s Codex fallback scan and `ingest-codex`'s
/// default root, mirroring how `SINGULARMEM_CURSOR_DIR` overrides Cursor's
/// per-user directory for the hook.
fn codex_root() -> Option<PathBuf> {
    std::env::var_os("SINGULARMEM_CODEX_ROOT")
        .map(PathBuf::from)
        .or_else(singularmem_ingest::default_codex_root)
}

/// Codex sends `transcript_path` when it has one; otherwise scan the Codex
/// root (see [`codex_root`]) for rollout files whose filename contains
/// `session_id`. Without a `transcript_path` *or* a non-empty `session_id`
/// there is nothing to scope the scan to, so nothing is ingested rather than
/// scanning the whole root with an empty filter (which would match every
/// session).
fn hook_ingest_codex(
    store: &Store,
    input: &singularmem_hooks::HookInput,
) -> Result<singularmem_ingest::Report, CliError> {
    use singularmem_ingest::{discover_codex_sessions, ingest_source, CodexRollout, Report};

    let files: Vec<PathBuf> = if let Some(p) = &input.transcript_path {
        vec![p.clone()]
    } else {
        let want = input.session_id.clone().unwrap_or_default();
        if want.is_empty() {
            tracing::warn!(
                "codex hook payload has no transcript_path or session_id; nothing ingested"
            );
            return Ok(Report::default());
        }
        let root = codex_root()
            .ok_or_else(|| CliError::Usage("cannot determine home directory".into()))?;
        discover_codex_sessions(&root)?
            .into_iter()
            .filter(|f| f.to_string_lossy().contains(&want))
            .collect()
    };

    let mut total = Report::default();
    for f in files {
        let src = CodexRollout::open(&f)?;
        accumulate(&mut total, ingest_source(store, &src, false)?);
    }
    Ok(total)
}

/// Cursor identifies the conversation to ingest by `conversation_id` when
/// present; otherwise fall back to filtering by `cwd` as the project.
/// Without either, there is nothing to scope the ingest to, so nothing is
/// ingested rather than pulling in every conversation. `SINGULARMEM_CURSOR_DIR`
/// overrides the default per-OS Cursor user directory (documented in
/// `docs/hooks.md`; used by tests to point at a fixture).
fn hook_ingest_cursor(
    store: &Store,
    input: &singularmem_hooks::HookInput,
) -> Result<singularmem_ingest::Report, CliError> {
    use singularmem_ingest::{default_cursor_user_dir, ingest_source, CursorChats, Report};

    if input.conversation_id.is_none() && input.cwd.is_none() {
        tracing::warn!("cursor hook payload has no conversation_id or cwd; nothing ingested");
        return Ok(Report::default());
    }

    let user = std::env::var_os("SINGULARMEM_CURSOR_DIR")
        .map(PathBuf::from)
        .or_else(default_cursor_user_dir)
        .ok_or_else(|| CliError::Usage("cannot determine Cursor user directory".into()))?;
    let mut src = CursorChats::open(&user)?;
    src.conversation_filter.clone_from(&input.conversation_id);
    if src.conversation_filter.is_none() {
        src.project_filter.clone_from(&input.cwd);
    }
    Ok(ingest_source(store, &src, false)?)
}

/// `hooks install|uninstall|status`. Never opens the store — see the
/// dispatch in [`run`].
fn cmd_hooks(cmd: &HooksCommand) -> Result<(), CliError> {
    use singularmem_hooks::{
        config_path, merge, read_config, remove, status, write_config, Editor,
    };

    let bin = std::env::current_exe()?;
    let project_dir = std::env::current_dir()?;
    let parse = |s: &str| {
        s.parse::<Editor>().map_err(|_| {
            CliError::Usage(format!(
                "unknown editor '{s}'; expected claude-code, codex, or cursor"
            ))
        })
    };
    let mut out = io::stdout().lock();

    match &cmd.action {
        HooksAction::Install {
            editor,
            project,
            print,
        } => {
            let editor = parse(editor)?;
            let path = config_path(editor, project.then_some(project_dir.as_path()))?;
            let existing = read_config(&path)?;
            let merged = merge(editor, &existing, &bin);
            if *print {
                serde_json::to_writer_pretty(&mut out, &merged)?;
                writeln!(out)?;
            } else {
                write_config(&path, &merged)?;
                writeln!(out, "{}", path.display())?;
            }
        }
        HooksAction::Uninstall { editor, project } => {
            let editor = parse(editor)?;
            let path = config_path(editor, project.then_some(project_dir.as_path()))?;
            let existing = read_config(&path)?;
            // Only rewrite the file when we actually have something to
            // remove — an uninstall over a config that never had our hooks
            // (foreign-only, or simply absent) must leave it byte-for-byte
            // untouched rather than reformatting it.
            if status(editor, &existing).installed {
                write_config(&path, &remove(editor, &existing))?;
            }
            writeln!(out, "{}", path.display())?;
        }
        HooksAction::Status { editor, project } => {
            let editors: Vec<Editor> = match editor {
                Some(e) => vec![parse(e)?],
                None => vec![Editor::ClaudeCode, Editor::Codex, Editor::Cursor],
            };
            for e in editors {
                let path = config_path(e, project.then_some(project_dir.as_path()))?;
                match read_config(&path) {
                    Ok(cfg) => {
                        let s = status(e, &cfg);
                        writeln!(
                            out,
                            "{e}\t{}\t{}\t{}",
                            if s.installed { "installed" } else { "absent" },
                            path.display(),
                            if s.installed && s.bin_exists {
                                "bin ok"
                            } else if s.installed {
                                "bin missing"
                            } else {
                                "-"
                            }
                        )?;
                    }
                    Err(err) => {
                        eprintln!("warning: {err}");
                        writeln!(out, "{e}\tinvalid\t{}\t-", path.display())?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn cmd_search(store: &Store, store_path: &Path, args: &SearchArgs) -> Result<(), CliError> {
    use singularmem_search::{EmbedderIndex, HybridSearchOptions, HybridSearcher, Index};

    let filter = args.scope.to_filter()?;
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
        scope: filter.clone(),
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
    let searcher = if filter.is_some() && sem_opt.is_some() {
        searcher.with_scope_lookup(store)
    } else {
        searcher
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
    use singularmem_retrieve::{RetrieveOptions, Retriever};
    use singularmem_search::{EmbedderIndex, HybridSearchOptions, HybridSearcher, Index};

    // Adapter lookup before any I/O so unknown-adapter errors fail fast.
    let adapter = find_adapter(&args.adapter)?;
    let adapter = &*adapter;

    let filter = args.scope.to_filter()?;

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
        scope: filter.clone(),
    };
    let opts = RetrieveOptions {
        max_blocks: args.limit,
        min_score: args.min_score,
        search: search_opts,
        scope: filter,
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

fn cmd_semantic_search(
    store: &Store,
    store_path: &Path,
    args: &SemanticSearchArgs,
) -> Result<(), CliError> {
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
        scope: ScopeArgs {
            scope: None,
            scope_exact: false,
        },
    };
    cmd_search(store, store_path, &forwarded)
}

/// Open the Tantivy sidecar at `index_path` for reindexing, recreating it
/// from scratch when it was built with an older schema.
fn open_or_rebuild_index(index_path: &Path) -> Result<singularmem_search::Index, CliError> {
    use singularmem_search::Index;

    match Index::open(index_path) {
        Ok(index) => Ok(index),
        Err(singularmem_search::Error::IndexSchemaMismatch { .. }) => {
            // Destructive action: always announce it, even with --quiet.
            eprintln!("rebuilding Tantivy sidecar with the current schema");
            std::fs::remove_dir_all(index_path).map_err(|e| {
                CliError::IndexOpen(format!(
                    "removing stale sidecar {}: {e}",
                    index_path.display()
                ))
            })?;
            Index::open(index_path).map_err(|e| CliError::IndexOpen(e.to_string()))
        }
        Err(e) => Err(CliError::IndexOpen(e.to_string())),
    }
}

fn cmd_reindex(store: &Store, store_path: &Path, args: &ReindexArgs) -> Result<(), CliError> {
    // Phase 1: Tantivy lexical reindex (always).
    let index_path = derive_index_path(store_path);
    let index = open_or_rebuild_index(&index_path)?;
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
