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
    /// `dir` is canonicalised first (falling back to `dir` unchanged if
    /// canonicalisation fails, e.g. the path does not exist) so that `.` and
    /// `..` — whose `Path::file_name` is `None` — resolve to the real
    /// basename instead of silently yielding an empty scope set.
    #[must_use]
    pub fn for_project(dir: &Path, include_files: bool) -> Self {
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let Some(base) = dir.file_name().and_then(|b| b.to_str()) else {
            return Self(Vec::new());
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

/// The one-line header that always precedes the rendered blocks.
#[must_use]
pub fn header(w: &Wakeup) -> String {
    format!(
        "# Singularmem wake-up — {} — {} items, showing last {}\n",
        w.scopes.join(", "),
        w.total,
        w.shown
    )
}

/// Header plus adapter output, dropping the oldest blocks until the whole
/// string fits in `max_bytes`. The header always survives.
#[must_use]
pub fn render(w: &Wakeup, adapter: &dyn Adapter, max_bytes: usize) -> String {
    let head = header(w);
    let mut ctx = w.context.clone();
    loop {
        let body = if ctx.blocks.is_empty() {
            String::new()
        } else {
            adapter.format(&ctx)
        };
        let out = format!("{head}{body}");
        if out.len() <= max_bytes || ctx.blocks.is_empty() {
            return if ctx.blocks.is_empty() { head } else { out };
        }
        ctx.blocks.remove(0);
    }
}
