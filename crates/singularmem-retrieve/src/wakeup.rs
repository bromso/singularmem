//! Wake-up context: the newest items in a project's scopes, rendered
//! through an [`Adapter`] under a byte budget. Spec:
//! `docs/superpowers/specs/2026-09-04-hooks-wakeup-13-design.md`.

use std::path::Path;
use std::time::Instant;

use singularmem_core::{ScopeFilter, Store};
use singularmem_search::ScoreKind;

use crate::adapter::Adapter;
use crate::retriever::{MemoryBlock, RetrievedContext};

/// One or more scope filters, OR-ed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSet(pub Vec<ScopeFilter>);

/// Editor prefixes that receive a project's session transcripts.
const EDITOR_PREFIXES: [&str; 3] = ["claude-code", "codex", "cursor"];

impl ScopeSet {
    /// The default scopes for a project directory: one per editor prefix
    /// (`claude-code/<b>`, `codex/<b>`, `cursor/<b>`) and, when
    /// `include_files`, `files/<b>`. A basename that is not a valid scope
    /// segment yields an empty set.
    ///
    /// The basename is taken from `dir` **as given**, not from its canonical
    /// form: the save side derives `<editor>/<basename of the raw cwd>` the
    /// editor reported, so canonicalising here would make a symlinked
    /// project directory (`~/dev/current -> ~/dev/project-v2`) save under
    /// `current` and wake up under `project-v2`.
    ///
    /// `dir` is canonicalised only when its raw basename is unusable —
    /// `None` (`.` and `..`, whose `Path::file_name` is `None`) or empty —
    /// so those still resolve to the real basename instead of silently
    /// yielding an empty scope set. Canonicalisation falling back to `dir`
    /// unchanged (e.g. the path does not exist) yields an empty set.
    #[must_use]
    pub fn for_project(dir: &Path, include_files: bool) -> Self {
        let canonical: std::path::PathBuf;
        let base = match dir.file_name().and_then(|b| b.to_str()) {
            Some(b) if !b.is_empty() => b,
            _ => {
                canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
                match canonical.file_name().and_then(|b| b.to_str()) {
                    Some(b) if !b.is_empty() => b,
                    _ => return Self(Vec::new()),
                }
            }
        };
        let mut prefixes: Vec<&str> = EDITOR_PREFIXES.to_vec();
        if include_files {
            prefixes.push("files");
        }
        Self(
            prefixes
                .into_iter()
                .filter_map(|p| ScopeFilter::descendants(&format!("{p}/{base}")).ok())
                .collect(),
        )
    }

    /// The scope paths, in order.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.0.iter().map(|f| f.path.clone()).collect()
    }
}

/// Options for [`build`].
#[derive(Debug, Clone, Copy)]
pub struct WakeupOptions {
    /// Newest items to include. Default 20.
    pub limit: usize,
    /// Rendered byte budget passed to [`render`] by callers. Default 8192.
    pub max_bytes: usize,
}

impl Default for WakeupOptions {
    fn default() -> Self {
        Self {
            limit: 20,
            max_bytes: 8192,
        }
    }
}

/// The built wake-up context.
#[derive(Debug, Clone)]
pub struct Wakeup {
    /// Blocks oldest → newest, `score` 0.0, `score_kind` RRF, `query`
    /// `wake-up:<scopes>`; renderable by any [`Adapter`].
    pub context: RetrievedContext,
    /// Items matching the scope set in total.
    pub total: usize,
    /// Scope paths queried, in order.
    pub scopes: Vec<String>,
    /// Blocks in `context` (≤ `limit`).
    pub shown: usize,
}

/// Collect the newest `opts.limit` items across `scopes`.
///
/// # Errors
/// Propagates store errors.
pub fn build(store: &Store, scopes: &ScopeSet, opts: &WakeupOptions) -> crate::Result<Wakeup> {
    let start = Instant::now();
    // `count_scoped_any` unions the filters in one query so an item in more
    // than one scope (e.g. `a` and `a/b`) is counted once, not once per
    // matching filter.
    let total = store.count_scoped_any(&scopes.0)?;
    let mut items = Vec::new();
    for f in &scopes.0 {
        items.extend(store.recent(Some(f), opts.limit)?);
    }
    // Newest first across scopes, then keep `limit`, then chronological.
    // Overlapping scope filters (e.g. `a` and `a/b`) can return the same
    // item from more than one `store.recent` call; dedup by id via a
    // `HashSet` rather than `dedup_by` on adjacents, since two duplicates
    // can end up non-adjacent (or tied on `created_at`) after the sort.
    items.sort_by_key(|item| std::cmp::Reverse(item.created_at));
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(item.id));
    items.truncate(opts.limit);
    items.reverse();
    let shown = items.len();
    let names = scopes.names();
    let blocks = items
        .into_iter()
        .map(|item| MemoryBlock {
            id: item.id,
            content: item.content,
            score: 0.0,
            score_kind: ScoreKind::Rrf,
            source: item.source,
            tags: item.tags,
            created_at: item.created_at,
            scope: item.scope,
        })
        .collect();
    Ok(Wakeup {
        context: RetrievedContext {
            blocks,
            query: format!("wake-up:{}", names.join(",")),
            elapsed: start.elapsed(),
            total_considered: total,
        },
        total,
        scopes: names,
        shown,
    })
}

/// The one-line header for the *pre-budget* block count (`w.shown`).
///
/// [`render`] recomputes its own header with the post-budget count via
/// [`header_for`]; use this only when no budgeting is applied.
#[must_use]
pub fn header(w: &Wakeup) -> String {
    header_for(w, w.shown)
}

/// The one-line header that always precedes the rendered blocks, reporting
/// `kept` as the number of blocks actually shown.
#[must_use]
pub fn header_for(w: &Wakeup, kept: usize) -> String {
    format!(
        "# Singularmem wake-up — {} — {} items, showing last {}\n",
        w.scopes.join(", "),
        w.total,
        kept
    )
}

/// Header plus adapter output, budgeted to `max_bytes`.
///
/// Drops the oldest blocks until the whole string fits. The header always
/// survives, and is recomputed on every iteration so its `showing last N`
/// reflects the blocks that actually survived the budget rather than the
/// pre-budget `w.shown`.
#[must_use]
pub fn render(w: &Wakeup, adapter: &dyn Adapter, max_bytes: usize) -> String {
    let mut ctx = w.context.clone();
    loop {
        let head = header_for(w, ctx.blocks.len());
        if ctx.blocks.is_empty() {
            return head;
        }
        let out = format!("{head}{}", adapter.format(&ctx));
        if out.len() <= max_bytes {
            return out;
        }
        ctx.blocks.remove(0);
    }
}
