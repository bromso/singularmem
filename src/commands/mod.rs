//! CLI surface: `Cli`, `Command`, and every subcommand's argument struct,
//! plus the handful of helpers shared by more than one `commands::*` module.
//!
//! Declaration order here drives clap's `--help` output order, so it must not
//! be reordered independently of the actual `--help` text (see
//! `tests/help_snapshots.rs`). Each subcommand's implementation (the
//! `cmd_*` functions) lives in its own sibling module.

pub mod bulk;
pub mod graph;
pub mod hooks;
pub mod index;
pub mod items;
pub mod search;
pub mod wakeup;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use singularmem_core::ScopeFilter;

use crate::commands::graph::GraphCommand;
use crate::CliError;

#[derive(Parser, Debug)]
#[command(
    name = "singularmem",
    version,
    about = "Local-first persistent memory layer for LLM workflows."
)]
pub struct Cli {
    /// Path to the `SQLite` store file. Defaults to the per-user XDG data dir.
    /// Also settable via the `SINGULARMEM_STORE` environment variable —
    /// the only way to point a hook (which runs with a fixed, flag-less
    /// command line) at a non-default store.
    #[arg(long, global = true, value_name = "PATH")]
    pub store: Option<PathBuf>,

    /// Open the store in read-only mode (refuses ingest).
    #[arg(long, global = true)]
    pub read_only: bool,

    /// Skip wiring up the Tantivy hook on open. Use for storage-only operations
    /// that don't need search, or when the Tantivy directory is intentionally absent.
    #[arg(long, global = true)]
    pub no_index: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
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
    /// Record and query the temporal knowledge graph.
    Graph(GraphCommand),
}

#[derive(Args, Debug)]
pub struct HookArgs {
    /// The editor invoking the hook: `claude-code`, `codex`, or `cursor`.
    pub editor: String,
    /// The hook event: `session-start`, `stop`, `pre-compact`, or `session-end`.
    pub event: String,
}

#[derive(Args, Debug)]
pub struct HooksCommand {
    #[command(subcommand)]
    pub action: HooksAction,
}

#[derive(Subcommand, Debug)]
pub enum HooksAction {
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
pub struct ScopeArgs {
    /// Restrict to this scope and its descendants (e.g. `claude-code/myproj`).
    #[arg(long, value_name = "PATH")]
    scope: Option<String>,
    /// With --scope: match only that exact scope, not descendants.
    #[arg(long)]
    scope_exact: bool,
}

impl ScopeArgs {
    pub fn to_filter(&self) -> Result<Option<ScopeFilter>, CliError> {
        match (&self.scope, self.scope_exact) {
            (None, true) => Err(CliError::Usage("--scope-exact requires --scope".into())),
            (None, false) => Ok(None),
            (Some(p), true) => Ok(Some(ScopeFilter::exact(p)?)),
            (Some(p), false) => Ok(Some(ScopeFilter::descendants(p)?)),
        }
    }
}

#[derive(Args, Debug)]
pub struct ScopeCommand {
    #[command(subcommand)]
    pub action: ScopeAction,
}

#[derive(Subcommand, Debug)]
pub enum ScopeAction {
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
pub struct IngestArgs {
    /// Item content as a literal string.
    #[arg(long, conflicts_with_all = ["file", "stdin"])]
    pub content: Option<String>,
    /// Read item content from a file.
    #[arg(long, conflicts_with_all = ["content", "stdin"])]
    pub file: Option<PathBuf>,
    /// Read item content from stdin.
    #[arg(long, conflicts_with_all = ["content", "file"])]
    pub stdin: bool,
    /// Tag (repeatable).
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Free-form provenance label.
    #[arg(long)]
    pub source: Option<String>,
    /// Supersedes the given prior item ID.
    #[arg(long)]
    pub supersedes: Option<String>,
    /// Inline JSON object as the metadata payload.
    #[arg(long)]
    pub metadata: Option<String>,
    /// Assign this scope path to the item (e.g. `team/backend`).
    #[arg(long, value_name = "PATH")]
    pub scope: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = IngestFormat::Id)]
    pub format: IngestFormat,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum IngestFormat {
    Id,
    Json,
}

#[derive(Args, Debug)]
pub struct IngestTranscriptArgs {
    /// Transcript files or directories (searched recursively for *.jsonl).
    /// Defaults to ~/.claude/projects.
    pub paths: Vec<PathBuf>,
    /// Keep only messages whose working directory equals DIR.
    #[arg(long, value_name = "DIR")]
    pub project: Option<PathBuf>,
    /// Keep subagent (sidechain) messages.
    #[arg(long)]
    pub include_sidechains: bool,
    /// Parse and report; write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Suppress per-file progress lines.
    #[arg(long)]
    pub quiet: bool,
    /// Override the default `claude-code/<project>` scope for every ingested item.
    #[arg(long, value_name = "PATH")]
    pub scope: Option<String>,
}

#[derive(Args, Debug)]
pub struct IngestCodexArgs {
    /// Rollout files or directories (searched recursively for
    /// `rollout-*.jsonl`). Defaults to `~/.codex/sessions`.
    pub paths: Vec<PathBuf>,
    /// Keep only messages whose session `cwd` equals DIR.
    #[arg(long, value_name = "DIR")]
    pub project: Option<PathBuf>,
    /// Parse and report; write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Suppress per-file progress lines.
    #[arg(long)]
    pub quiet: bool,
    /// Override the default `codex/<project>` scope for every ingested item.
    #[arg(long, value_name = "PATH")]
    pub scope: Option<String>,
}

#[derive(Args, Debug)]
pub struct IngestCursorArgs {
    /// Cursor's per-user directory (contains `globalStorage/` and
    /// `workspaceStorage/`). Defaults to `SINGULARMEM_CURSOR_DIR`, then to
    /// the per-OS Cursor `User` dir.
    #[arg(long, value_name = "DIR")]
    pub cursor_dir: Option<PathBuf>,
    /// Keep only conversations whose workspace folder equals DIR.
    #[arg(long, value_name = "DIR")]
    pub project: Option<PathBuf>,
    /// Keep only this composer (conversation) id.
    #[arg(long)]
    pub conversation: Option<String>,
    /// Parse and report; write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Suppress per-file progress lines.
    #[arg(long)]
    pub quiet: bool,
    /// Override the default `cursor/<project>` scope for every ingested item.
    #[arg(long, value_name = "PATH")]
    pub scope: Option<String>,
}

#[derive(Args, Debug)]
pub struct IngestDirArgs {
    /// Root directory to walk.
    pub path: PathBuf,
    /// Skip files larger than this many bytes.
    #[arg(long, default_value_t = singularmem_ingest::DEFAULT_MAX_FILE_BYTES)]
    pub max_file_bytes: u64,
    /// Parse and report; write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Suppress per-file progress lines.
    #[arg(long)]
    pub quiet: bool,
    /// Override the default `files/<dirname>` scope for every ingested item.
    #[arg(long, value_name = "PATH")]
    pub scope: Option<String>,
}

#[derive(Args, Debug)]
pub struct GetArgs {
    /// The item ID (26-char ULID, case-insensitive).
    pub id: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = GetFormat::Text)]
    pub format: GetFormat,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum GetFormat {
    Text,
    Json,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Filter to items containing every named tag (AND-semantics, repeatable).
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = ListFormat::Table)]
    pub format: ListFormat,
    /// Cap the number of items returned.
    #[arg(long)]
    pub limit: Option<usize>,
    #[command(flatten)]
    pub scope: ScopeArgs,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ListFormat {
    Table,
    Jsonl,
    Ids,
}

#[derive(Args, Debug)]
pub struct RevisionsArgs {
    pub id: String,
    #[arg(long, value_enum, default_value_t = ListFormat::Table)]
    pub format: ListFormat,
}

/// Which search backend(s) to use for `search`.
#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum SearchMode {
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
pub struct SearchArgs {
    /// One or more query tokens. Multiple tokens become an implicit AND.
    pub queries: Vec<String>,
    /// Which backend(s) to use. `auto` picks hybrid when both sidecars exist,
    /// falls back to whichever one is present, and errors when neither is.
    #[arg(short = 'm', long, value_enum, default_value_t = SearchMode::Auto)]
    pub mode: SearchMode,
    /// Max hits to return.
    #[arg(short = 'l', long, default_value = "20")]
    pub limit: usize,
    /// Skip first N hits (pagination, lexical mode only).
    #[arg(long, default_value = "0")]
    pub offset: usize,
    /// Per-ranker overfetch factor; hybrid only. Default 3.
    #[arg(long, default_value = "3")]
    pub fetch_multiplier: usize,
    /// RRF damping constant; hybrid only. Default 60.
    #[arg(long, default_value = "60")]
    pub rrf_k: usize,
    /// Suppress snippet highlighting (faster).
    #[arg(long)]
    pub no_snippets: bool,
    /// Include per-ranker rank columns in human output.
    #[arg(long)]
    pub show_ranks: bool,
    /// Emit JSON results instead of human-readable output.
    #[arg(long)]
    pub json: bool,
    /// Output format. (Legacy; `--json` and `--show-ranks` are preferred.)
    #[arg(long, value_enum, default_value_t = ListFormat::Table)]
    pub format: ListFormat,
    #[command(flatten)]
    pub scope: ScopeArgs,
}

#[derive(Args, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct ReindexArgs {
    /// Suppress progress output.
    #[arg(long)]
    pub quiet: bool,
    /// Also rebuild the vector index.
    #[arg(long)]
    pub with_embeddings: bool,
    /// Which embedding model to use. Only meaningful with --with-embeddings.
    #[arg(long, default_value = "all-mini-lm-l6-v2")]
    pub embedding_model: String,
    /// Destructive — delete .vectors/ before reindex (e.g. to switch models).
    #[arg(long)]
    pub reset_vectors: bool,
    /// Required to confirm --reset-vectors.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct SemanticSearchArgs {
    /// One or more query tokens. Multiple tokens are joined with a space.
    pub queries: Vec<String>,
    /// Max hits to return.
    #[arg(long, default_value = "20")]
    pub limit: usize,
    /// Minimum cosine-similarity score (0.0–1.0) for a hit to be included.
    #[arg(long, default_value = "0.0")]
    pub min_score: f32,
    /// Output format.
    #[arg(long, value_enum, default_value_t = ListFormat::Table)]
    pub format: ListFormat,
}

#[derive(Args, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct RetrieveArgs {
    /// One or more query tokens. Multiple tokens are joined with a space
    /// before being passed to the underlying hybrid search.
    pub queries: Vec<String>,
    /// Which adapter to use for formatting. Defaults to `plain`.
    /// Sub-projects 3b/3c/3d add `claude`, `openai`, `gemini` to the registry.
    #[arg(short = 'a', long, default_value = "plain")]
    pub adapter: String,
    /// Max memory blocks to include in the formatted output.
    #[arg(short = 'l', long, default_value = "10")]
    pub limit: usize,
    /// Minimum score for a hit to be included.
    #[arg(long, default_value = "0.0")]
    pub min_score: f32,
    /// Underlying search mode (passed through to `HybridSearcher`).
    #[arg(short = 'm', long, value_enum, default_value_t = SearchMode::Auto)]
    pub mode: SearchMode,
    /// Per-ranker overfetch factor (hybrid only).
    #[arg(long, default_value = "3")]
    pub fetch_multiplier: usize,
    /// RRF damping constant (hybrid only).
    #[arg(long, default_value = "60")]
    pub rrf_k: usize,
    /// Emit `RetrievedContext` as JSON instead of adapter-formatted output.
    #[arg(long)]
    pub json: bool,
    /// Print "Retrieved N blocks in Xms" to stderr after the formatted output.
    #[arg(long)]
    pub show_elapsed: bool,
    #[command(flatten)]
    pub scope: ScopeArgs,
}

#[derive(Args, Debug)]
pub struct WakeUpArgs {
    /// Restrict to this scope and its descendants (repeatable; OR-ed).
    /// Defaults to the editor scopes for `--project` (or the current
    /// directory) when omitted.
    #[arg(long = "scope", value_name = "PATH")]
    pub scope: Vec<String>,
    /// Project directory whose basename derives the default scopes.
    /// Defaults to the current directory. Ignored when `--scope` is given.
    #[arg(long, value_name = "DIR")]
    pub project: Option<PathBuf>,
    /// Also include the `files/<project>` scope in the default scope set.
    #[arg(long)]
    pub include_files: bool,
    /// Newest items to include.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
    /// Rendered byte budget; oldest blocks are dropped first to fit
    /// (applies to the rendered text; hook envelopes add a few bytes of
    /// JSON).
    #[arg(long, default_value_t = 8192)]
    pub max_bytes: usize,
    /// Which adapter to use for formatting.
    #[arg(short = 'a', long, default_value = "plain")]
    pub adapter: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = WakeUpFormat::Text)]
    pub format: WakeUpFormat,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum WakeUpFormat {
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

/// Resolve the store path from (in order of precedence) `--store`, the
/// `SINGULARMEM_STORE` environment variable (the only way to point a hook —
/// a fixed, flag-less command line — at a non-default store), and the
/// per-user XDG default.
pub fn resolve_store_path(cli: &Cli) -> PathBuf {
    cli.store
        .clone()
        .or_else(|| std::env::var_os("SINGULARMEM_STORE").map(PathBuf::from))
        .unwrap_or_else(default_store_path)
}

fn default_store_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("singularmem")
        .join("store.db")
}

/// Canonicalise `project`, falling back to the given path unchanged when
/// canonicalisation fails (e.g. it does not exist).
pub fn canonicalize_project(project: Option<&PathBuf>) -> Option<PathBuf> {
    let p = project?;
    Some(p.canonicalize().unwrap_or_else(|_| p.clone()))
}

/// Accumulate a per-file/per-source [`singularmem_ingest::Report`] into a
/// running total. Shared by `commands::bulk`'s multi-file ingest verbs and
/// `commands::hooks`'s Codex hook, which scans (and so accumulates over)
/// more than one rollout file per event.
pub fn accumulate(total: &mut singularmem_ingest::Report, r: singularmem_ingest::Report) {
    total.ingested += r.ingested;
    total.skipped_existing += r.skipped_existing;
    total.skipped_filtered += r.skipped_filtered;
    total.failed += r.failed;
}
