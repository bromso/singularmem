---
title: Session hooks and wake-up (Sub-project 13)
date: 2026-09-04
status: draft
sub-project: 13-hooks-wakeup
spec: ../specs/2026-09-04-hooks-wakeup-13-design.md
---

# Session Hooks and Wake-up (Sub-project 13) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Editor hooks (Claude Code, Codex CLI, Cursor) that save the live transcript on stop / pre-compaction and inject recent project memory at session start; a standalone `wake-up` command; Codex and Cursor transcript parsers; a `hooks install|uninstall|status` verb.

**Architecture:** Two new `Source` impls in `singularmem-ingest` (`CodexRollout`, `CursorChats`); `Store::recent` in core; a `wakeup` module in `singularmem-retrieve` that builds a `RetrievedContext` for the existing adapters with a byte budget; a new pure crate `singularmem-hooks` (editor enums, stdin parsing, output envelopes, config-file merge/remove/status); the CLI gains `ingest-codex`, `ingest-cursor`, `wake-up`, `hook <editor> <event>`, and `hooks install|uninstall|status`. The hook verb never blocks the editor.

**Tech Stack:** Rust 1.80, rusqlite 0.32 (immutable open for Cursor DBs), serde_json, clap 4, jiff, assert_cmd, tempfile.

## Global Constraints

- Codex line schema (community-documented, parser defensive): `session_meta` (payload `id`, `cwd`), `response_item` with `payload.type == "message"`, `role` `user|assistant`, `content[] { type: input_text|output_text, text }`; everything else skipped. External id `codex:<session_id>:<line_no>[#n]`; source `codex:<session_id>`; tags `codex`, `role:<role>`, `transcript`; default scope `codex/<basename of cwd>`.
- Cursor: user dir per OS (macOS `~/Library/Application Support/Cursor/User`, Linux `~/.config/Cursor/User`, Windows `%APPDATA%\Cursor\User`); `workspaceStorage/<hash>/workspace.json` → `folder` (`file://` URI); `workspaceStorage/<hash>/state.vscdb` `ItemTable` key `composer.composerData` → `allComposers[] { composerId, createdAt }`; `globalStorage/state.vscdb` `cursorDiskKV` keys `composerData:<id>` (`name`, `createdAt`, `fullConversationHeadersOnly[] { bubbleId, type }`, fallback `conversation[]`) and `bubbleId:<composer>:<bubble>` (`type` 1 user / 2 assistant, `text`). Open with `immutable=1`; fall back to a temp copy. External id `cursor:<composerId>:<bubbleId>[#n]`; source `cursor:<composerId>`; tags `cursor`, `role:<role>`, `transcript`; default scope `cursor/<basename of workspace folder>`.
- Wake-up: `Store::recent` = `ORDER BY created_at DESC LIMIT n`, output oldest→newest; default scope set for basename `b` = `[claude-code/b, codex/b, cursor/b]` plus `files/b` only with `include_files`; blocks carry `score 0.0`, `ScoreKind::Rrf`, `query = "wake-up:<scopes joined by ,>"`; header `# Singularmem wake-up — <scopes> — <total> items, showing last <n>`; byte budget default 8192 drops oldest blocks first, header always survives; zero items → header only, exit 0.
- Envelopes: Claude Code and Codex session start `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"<text>"}}`; Cursor `{"additional_context":"<text>"}`.
- Hook config entries exactly as the spec's "Hook config entries" section: Claude Code `SessionStart` (matcher `startup|resume|clear|compact`, timeout 30), `Stop` (`async: true`), `PreCompact` (timeout 60), `SessionEnd` (timeout 60); Codex `SessionStart`/`Stop`(async)/`PreCompact` with `matcher: "*"`; Cursor `version: 1`, `sessionStart`(30)/`stop`(60, `loop_limit: 1`)/`preCompact`(60)/`sessionEnd`(60). Command string: `"<absolute bin path>" hook <editor> <event>`; ours ⇔ command contains `singularmem hook `.
- `hook` verb always exits 0; errors to stderr. Bulk verbs share `ingest-transcript`'s exit codes and summary line.
- Clippy pedantic + nursery `-D warnings`; fmt; no network in tests; `#![forbid(unsafe_code)]`; every commit signed off. Branch `hooks-wakeup-13`.

## File Structure

| Path | Responsibility |
|---|---|
| `crates/singularmem-core/src/query.rs` | `Store::recent` |
| `crates/singularmem-retrieve/src/wakeup.rs` (new) | `ScopeSet`, `WakeupOptions`, `Wakeup`, `build`, `render` |
| `crates/singularmem-ingest/src/codex.rs` (new) | `CodexRollout`, `discover_codex_sessions`, `default_codex_root` |
| `crates/singularmem-ingest/src/cursor.rs` (new) | `CursorChats`, `default_cursor_user_dir` |
| `crates/singularmem-ingest/src/project_filter.rs` (new) | `ProjectFilter` moved out of `claude.rs` for reuse |
| `crates/singularmem-hooks/` (new crate) | `Editor`, `Event`, `HookInput`, `parse_input`, `session_start_envelope`, `entries`, `merge`, `remove`, `status`, `config_path`, `write_config` |
| `src/main.rs` | five new verbs; shared `ingest_files` helper |
| `tests/cli.rs`, `tests/fixtures/codex/`, `tests/fixtures/hooks/` | CLI tests + fixtures |
| `README.md`, `docs/hooks.md` (new), `.github/workflows/publish-cargo.yml` | docs, publish order |

---

### Task 1: `Store::recent` and the wake-up builder

**Files:**
- Modify: `crates/singularmem-core/src/query.rs`
- Create: `crates/singularmem-retrieve/src/wakeup.rs`; Modify: `crates/singularmem-retrieve/src/lib.rs`
- Create: `crates/singularmem-core/tests/recent.rs`, `crates/singularmem-retrieve/tests/wakeup.rs`

**Interfaces:**
- Produces: `Store::recent(&self, filter: Option<&ScopeFilter>, limit: usize) -> Result<Vec<Item>>` (newest first); `Store::count_scoped(&self, filter: Option<&ScopeFilter>) -> Result<usize>`.
- Produces in `singularmem_retrieve::wakeup`: `pub struct ScopeSet(pub Vec<ScopeFilter>)`, `ScopeSet::for_project(dir: &Path, include_files: bool) -> ScopeSet`, `ScopeSet::names(&self) -> Vec<String>`, `pub struct WakeupOptions { pub limit: usize, pub max_bytes: usize }` (Default 20 / 8192), `pub struct Wakeup { pub context: RetrievedContext, pub total: usize, pub scopes: Vec<String>, pub shown: usize }`, `pub fn build(store: &Store, scopes: &ScopeSet, opts: &WakeupOptions) -> Result<Wakeup>`, `pub fn header(w: &Wakeup) -> String`, `pub fn render(w: &Wakeup, adapter: &dyn Adapter, max_bytes: usize) -> String`.

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** none
**Blocks:** Tasks 5, 6

- [ ] **Step 1: Failing tests**

`crates/singularmem-core/tests/recent.rs`:

```rust
use singularmem_core::{NewItem, ScopeFilter, Store};
use tempfile::TempDir;

fn seeded() -> (TempDir, Store) {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    for (c, sc) in [("one", "a"), ("two", "a/b"), ("three", "x"), ("four", "a")] {
        let mut n = NewItem::text(c);
        n.scope = Some(sc.into());
        s.ingest(n).unwrap();
    }
    s.ingest(NewItem::text("unscoped")).unwrap();
    (d, s)
}

#[test]
fn recent_is_newest_first_and_limited() {
    let (_d, s) = seeded();
    let v: Vec<String> = s.recent(None, 3).unwrap().into_iter().map(|i| i.content).collect();
    assert_eq!(v, vec!["unscoped", "four", "three"]);
}

#[test]
fn recent_respects_scope_filter() {
    let (_d, s) = seeded();
    let f = ScopeFilter::descendants("a").unwrap();
    let v: Vec<String> = s.recent(Some(&f), 10).unwrap().into_iter().map(|i| i.content).collect();
    assert_eq!(v, vec!["four", "two", "one"]);
    assert_eq!(s.count_scoped(Some(&f)).unwrap(), 3);
    assert_eq!(s.count_scoped(None).unwrap(), 5);
}

#[test]
fn recent_zero_limit_is_empty() {
    let (_d, s) = seeded();
    assert!(s.recent(None, 0).unwrap().is_empty());
}
```

`crates/singularmem-retrieve/tests/wakeup.rs`:

```rust
use std::path::Path;

use singularmem_core::{NewItem, ScopeFilter, Store};
use singularmem_retrieve::wakeup::{build, header, render, ScopeSet, WakeupOptions};
use singularmem_retrieve::PlainAdapter;
use tempfile::TempDir;

fn store_with(items: &[(&str, &str)]) -> (TempDir, Store) {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    for (c, sc) in items {
        let mut n = NewItem::text(*c);
        n.scope = Some((*sc).into());
        s.ingest(n).unwrap();
    }
    (d, s)
}

#[test]
fn scope_set_for_project_unions_editor_scopes() {
    let set = ScopeSet::for_project(Path::new("/tmp/My Repo"), false);
    // basename "My Repo" is not a valid segment (space) → normalised? No: invalid → dropped.
    assert!(set.0.is_empty());
    let set = ScopeSet::for_project(Path::new("/home/me/singularmem"), false);
    assert_eq!(set.names(), vec!["claude-code/singularmem", "codex/singularmem", "cursor/singularmem"]);
    let set = ScopeSet::for_project(Path::new("/home/me/singularmem"), true);
    assert_eq!(set.names().last().map(String::as_str), Some("files/singularmem"));
}

#[test]
fn build_returns_recent_items_oldest_to_newest_across_scopes() {
    let (_d, s) = store_with(&[("c1", "claude-code/p"), ("x", "cursor/other"), ("k1", "codex/p"), ("c2", "claude-code/p")]);
    let set = ScopeSet::for_project(Path::new("/w/p"), false);
    let w = build(&s, &set, &WakeupOptions { limit: 2, max_bytes: 8192 }).unwrap();
    assert_eq!(w.total, 3);
    assert_eq!(w.shown, 2);
    let contents: Vec<&str> = w.context.blocks.iter().map(|b| b.content.as_str()).collect();
    assert_eq!(contents, vec!["k1", "c2"]);
    assert_eq!(w.context.query, "wake-up:claude-code/p,codex/p,cursor/p");
    assert!(w.context.blocks.iter().all(|b| b.score == 0.0));
}

#[test]
fn render_has_header_and_budget_drops_oldest_first() {
    let (_d, s) = store_with(&[("aaaa aaaa aaaa", "codex/p"), ("bbbb bbbb bbbb", "codex/p"), ("cccc cccc cccc", "codex/p")]);
    let set = ScopeSet(vec![ScopeFilter::descendants("codex/p").unwrap()]);
    let w = build(&s, &set, &WakeupOptions::default()).unwrap();
    let full = render(&w, &PlainAdapter, 100_000);
    assert!(full.starts_with("# Singularmem wake-up — codex/p — 3 items, showing last 3\n"));
    assert!(full.contains("aaaa") && full.contains("cccc"));
    let small = render(&w, &PlainAdapter, full.len() - 20);
    assert!(small.starts_with(&header(&w)));
    assert!(!small.contains("aaaa"), "oldest dropped first");
    assert!(small.contains("cccc"));
    assert!(small.len() <= full.len() - 20);
    let tiny = render(&w, &PlainAdapter, 10);
    assert_eq!(tiny.trim_end(), header(&w).trim_end(), "header always survives");
}

#[test]
fn empty_store_gives_header_only() {
    let d = TempDir::new().unwrap();
    let s = Store::open(d.path().join("s.db")).unwrap();
    let set = ScopeSet::for_project(Path::new("/w/p"), false);
    let w = build(&s, &set, &WakeupOptions::default()).unwrap();
    assert_eq!(w.total, 0);
    let out = render(&w, &PlainAdapter, 8192);
    assert_eq!(out, "# Singularmem wake-up — claude-code/p, codex/p, cursor/p — 0 items, showing last 0\n");
}
```

- [ ] **Step 2: Run, expect failure** — `cargo test -p singularmem-core --test recent` and `cargo test -p singularmem-retrieve --test wakeup` → unresolved.

- [ ] **Step 3: `Store::recent` and `count_scoped`** in `query.rs`, next to `list_by_tags_scoped`:

```rust
    /// The newest `limit` items, optionally restricted to `filter`. Newest
    /// first (callers wanting chronological order reverse the vector).
    ///
    /// # Errors
    /// `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn recent(&self, filter: Option<&ScopeFilter>, limit: usize) -> Result<Vec<Item>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut sql = String::from("SELECT id FROM items WHERE 1=1");
        let mut params: Vec<String> = Vec::new();
        if let Some(f) = filter {
            let (clause, binds) = f.sql_clause();
            sql.push_str(" AND ");
            sql.push_str(clause);
            params.extend(binds);
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        params.push(limit.to_string());
        let ids: Vec<ItemId> = {
            let conn = self.conn.lock().expect("store mutex poisoned");
            let mut stmt = conn.prepare(&sql).map_err(|e| Error::Sqlite { context: "preparing recent query", source: e })?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(params.iter()), |r| r.get::<_, String>(0))
                .map_err(|e| Error::Sqlite { context: "executing recent query", source: e })?
                .collect::<rusqlite::Result<Vec<String>>>()
                .map_err(|e| Error::Sqlite { context: "collecting recent IDs", source: e })?;
            rows.into_iter().map(|s| s.parse::<ItemId>()).collect::<std::result::Result<Vec<_>, _>>()?
        };
        ids.into_iter().map(|id| self.get(id)).collect()
    }

    /// Number of items matching `filter` (all items when `None`).
    ///
    /// # Errors
    /// `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn count_scoped(&self, filter: Option<&ScopeFilter>) -> Result<usize> {
        let mut sql = String::from("SELECT COUNT(*) FROM items WHERE 1=1");
        let mut params: Vec<String> = Vec::new();
        if let Some(f) = filter {
            let (clause, binds) = f.sql_clause();
            sql.push_str(" AND ");
            sql.push_str(clause);
            params.extend(binds);
        }
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n: i64 = conn
            .query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| r.get(0))
            .map_err(|e| Error::Sqlite { context: "counting scoped items", source: e })?;
        Ok(usize::try_from(n).unwrap_or(0))
    }
```

Note: `LIMIT ?` bound as a TEXT string is accepted by SQLite for LIMIT (it converts); if the `recent_is_newest_first_and_limited` test fails on that, bind `limit` as `i64` via a separate `params!` build instead.

- [ ] **Step 4: `wakeup.rs`**

```rust
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
    #[must_use]
    pub fn for_project(dir: &Path, include_files: bool) -> Self {
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
        Self { limit: 20, max_bytes: 8192 }
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
    let mut items = Vec::new();
    let mut total = 0usize;
    for f in &scopes.0 {
        total += store.count_scoped(Some(f))?;
        items.extend(store.recent(Some(f), opts.limit)?);
    }
    // Newest first across scopes, then keep `limit`, then chronological.
    items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    items.dedup_by(|a, b| a.id == b.id);
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
        let body = if ctx.blocks.is_empty() { String::new() } else { adapter.format(&ctx) };
        let out = format!("{head}{body}");
        if out.len() <= max_bytes || ctx.blocks.is_empty() {
            return if ctx.blocks.is_empty() { head } else { out };
        }
        ctx.blocks.remove(0);
    }
}
```

Note `MemoryBlock` has a `scope` field since sub-project 12 — check `retriever.rs` and include every field. In `lib.rs`: `pub mod wakeup;`. Since scope filtering in `recent` uses `sql_clause`, confirm it is `pub(crate)` in core — it is; `recent` lives in core so that is fine.

- [ ] **Step 5: Run, lint, commit**

`cargo test -p singularmem-core --test recent`, `cargo test -p singularmem-retrieve --test wakeup`, workspace clippy/fmt.

```bash
git add crates/singularmem-core crates/singularmem-retrieve
git commit -s -m "feat(core,retrieve): Store::recent and the wake-up context builder"
```

---

### Task 2: Codex rollout source

**Files:**
- Create: `crates/singularmem-ingest/src/project_filter.rs` (move `ProjectFilter` out of `claude.rs`; `pub(crate)`), `crates/singularmem-ingest/src/codex.rs`, `crates/singularmem-ingest/tests/fixtures/codex-rollout.jsonl`, `crates/singularmem-ingest/tests/fixtures/codex-legacy.jsonl`, `crates/singularmem-ingest/tests/codex_parse.rs`
- Modify: `crates/singularmem-ingest/src/claude.rs` (use the shared `ProjectFilter`), `src/lib.rs` (exports)

**Interfaces:**
- Produces: `pub struct CodexRollout { pub path: PathBuf, pub project_filter: Option<PathBuf>, pub scope_override: Option<String>, pub chunk_bytes: usize, .. }`, `CodexRollout::open(path) -> Result<Self>`, `impl Source`, `pub fn discover_codex_sessions(root) -> Result<Vec<PathBuf>>` (files named `rollout-*.jsonl`, recursive, sorted), `pub fn default_codex_root() -> Option<PathBuf>` (`$HOME/.codex/sessions`).

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** none
**Blocks:** Task 5

- [ ] **Step 1: Fixtures**

`tests/fixtures/codex-rollout.jsonl` (7 lines):

```
{"timestamp":"2026-09-01T10:00:00.000Z","type":"session_meta","payload":{"id":"sess-1","cwd":"/home/me/proj","originator":"codex_cli_rs","cli_version":"0.50.0"}}
{"timestamp":"2026-09-01T10:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5-codex"}}
{"timestamp":"2026-09-01T10:00:02.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"How do I run the tests?"}]}}
{"timestamp":"2026-09-01T10:00:03.000Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"cargo test\"}","call_id":"c1"}}
{"timestamp":"2026-09-01T10:00:04.000Z","type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"ok"}}
{"timestamp":"2026-09-01T10:00:05.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Run cargo test."},{"type":"output_text","text":"It takes a minute."}]}}
{"timestamp":"2026-09-01T10:00:06.000Z","type":"event_msg","payload":{"type":"agent_message","message":"Run cargo test."}}
{this is not json}
```

(8 lines including the malformed last one.) `tests/fixtures/codex-legacy.jsonl` (no `session_meta`):

```
{"timestamp":"2026-08-01T00:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"legacy hello"}]}}
```

- [ ] **Step 2: Failing tests** — `tests/codex_parse.rs`:

```rust
use std::path::{Path, PathBuf};

use singularmem_ingest::{discover_codex_sessions, CodexRollout, Source};

fn fx(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[test]
fn keeps_only_user_and_assistant_messages() {
    let src = CodexRollout::open(fx("codex-rollout.jsonl")).unwrap();
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    for r in src.items() {
        match r { Ok(i) => ok.push(i), Err(e) => errs.push(e) }
    }
    assert_eq!(errs.len(), 1);
    assert!(matches!(errs[0], singularmem_ingest::Error::Json { line: 8, .. }));
    let ids: Vec<&str> = ok.iter().map(|i| i.external_id.as_deref().unwrap()).collect();
    assert_eq!(ids, vec!["codex:sess-1:3", "codex:sess-1:6"]);
    assert_eq!(ok[1].content, "Run cargo test.\n\nIt takes a minute.");
    assert_eq!(ok[1].tags, vec!["codex", "role:assistant", "transcript"]);
    assert_eq!(ok[1].source.as_deref(), Some("codex:sess-1"));
    assert_eq!(ok[1].metadata["cwd"], "/home/me/proj");
    assert_eq!(ok[1].metadata["occurred_at"], "2026-09-01T10:00:05Z");
    assert_eq!(ok[1].metadata["line"], 6);
    assert_eq!(src.default_scope(&ok[0]).as_deref(), Some("codex/proj"));
    // function_call, function_call_output count as filtered; turn_context/event_msg are structural.
    assert_eq!(src.filtered_count(), 2);
}

#[test]
fn legacy_file_without_session_meta_uses_stem_and_no_scope() {
    let src = CodexRollout::open(fx("codex-legacy.jsonl")).unwrap();
    let items: Vec<_> = src.items().map(Result::unwrap).collect();
    assert_eq!(items[0].external_id.as_deref(), Some("codex:codex-legacy:1"));
    assert_eq!(items[0].metadata["cwd"], serde_json::Value::Null);
    assert_eq!(src.default_scope(&items[0]), None);
}

#[test]
fn project_filter_and_override() {
    let mut src = CodexRollout::open(fx("codex-rollout.jsonl")).unwrap();
    src.project_filter = Some(PathBuf::from("/home/me/other"));
    assert_eq!(src.items().filter_map(Result::ok).count(), 0);
    src.project_filter = Some(PathBuf::from("/home/me/proj"));
    assert_eq!(src.items().filter_map(Result::ok).count(), 2);
    src.scope_override = Some("Team/X".into());
    let first = src.items().find_map(Result::ok).unwrap();
    assert_eq!(src.default_scope(&first).as_deref(), Some("team/x"));
}

#[test]
fn discover_finds_rollout_files_only() {
    let d = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(d.path().join("2026/09/01")).unwrap();
    std::fs::write(d.path().join("2026/09/01/rollout-2026-09-01T10-00-00-abc.jsonl"), "").unwrap();
    std::fs::write(d.path().join("2026/09/01/notes.jsonl"), "").unwrap();
    let found = discover_codex_sessions(d.path()).unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].file_name().unwrap().to_str().unwrap().starts_with("rollout-"));
}
```

- [ ] **Step 3: Run, expect failure.**

- [ ] **Step 4: Implement**

`project_filter.rs`: move the `ProjectFilter` struct and impl from `claude.rs` verbatim, `pub(crate) struct ProjectFilter` with `pub(crate) fn new` / `matches`; `claude.rs` gains `use crate::project_filter::ProjectFilter;`.

`codex.rs`:

```rust
//! OpenAI Codex CLI rollout source (`~/.codex/sessions/**/rollout-*.jsonl`).
//!
//! The line schema is community-documented, not official; this parser is
//! defensive: anything that is not a `response_item` message with text is
//! ignored.

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use singularmem_core::NewItem;

use crate::chunk::{chunk_text, DEFAULT_CHUNK_BYTES};
use crate::error::{Error, Result};
use crate::project_filter::ProjectFilter;
use crate::Source;

/// One Codex rollout file.
#[derive(Debug)]
pub struct CodexRollout {
    /// Path to the `.jsonl` file.
    pub path: PathBuf,
    /// Keep only messages from sessions whose `cwd` names this directory.
    pub project_filter: Option<PathBuf>,
    /// Explicit scope override; wins over `codex/<cwd basename>`.
    pub scope_override: Option<String>,
    /// Chunk cap in bytes.
    pub chunk_bytes: usize,
    filtered: Cell<usize>,
    derived_memo: RefCell<Option<(String, Option<String>)>>,
}

#[derive(Deserialize)]
struct Line {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    payload: Option<serde_json::Value>,
}

/// Session-level facts read from the `session_meta` line (or fallbacks).
struct Session {
    id: String,
    cwd: Option<String>,
}

impl CodexRollout {
    /// Open a rollout file.
    ///
    /// # Errors
    /// `Error::NotFound` when `path` is not a file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.is_file() {
            return Err(Error::NotFound { path });
        }
        Ok(Self {
            path,
            project_filter: None,
            scope_override: None,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            filtered: Cell::new(0),
            derived_memo: RefCell::new(None),
        })
    }

    fn file_stem(&self) -> String {
        self.path.file_stem().map_or_else(|| "unknown".to_string(), |s| s.to_string_lossy().into_owned())
    }

    /// Build items for one `response_item` message line. `None` = structural
    /// (not counted); `Some(vec![])` = deliberately filtered (counted).
    fn line_to_items(&self, line_no: usize, line: &Line, session: &Session, filter: Option<&mut ProjectFilter>) -> Option<Vec<NewItem>> {
        if line.kind != "response_item" {
            return None;
        }
        let payload = line.payload.as_ref()?;
        if payload.get("type").and_then(|t| t.as_str()) != Some("message") {
            return Some(Vec::new()); // function_call, function_call_output, reasoning, …
        }
        let role = payload.get("role").and_then(|r| r.as_str())?;
        if role != "user" && role != "assistant" {
            return Some(Vec::new());
        }
        if let Some(f) = filter {
            if !f.matches(session.cwd.as_deref()) {
                return Some(Vec::new());
            }
        }
        let text: Vec<&str> = payload
            .get("content")
            .and_then(|c| c.as_array())
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| matches!(b.get("type").and_then(|t| t.as_str()), Some("input_text" | "output_text" | "text")))
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        let chunks = chunk_text(&text.join("\n\n"), self.chunk_bytes);
        if chunks.is_empty() {
            return Some(Vec::new());
        }
        let occurred_at = line.timestamp.as_deref().and_then(|t| t.parse::<jiff::Timestamp>().ok()).map(|t| t.to_string());
        let chunk_count = chunks.len();
        let items = chunks
            .into_iter()
            .enumerate()
            .map(|(i, content)| {
                let mut tags = vec!["codex".to_string(), format!("role:{role}"), "transcript".to_string()];
                tags.sort();
                let external_id = if chunk_count == 1 {
                    format!("codex:{}:{line_no}", session.id)
                } else {
                    format!("codex:{}:{line_no}#{i}", session.id)
                };
                NewItem {
                    content,
                    supersedes: None,
                    tags,
                    source: Some(format!("codex:{}", session.id)),
                    metadata: serde_json::json!({
                        "session_id": session.id,
                        "line": line_no,
                        "role": role,
                        "cwd": session.cwd,
                        "occurred_at": occurred_at,
                        "chunk_index": i,
                        "chunk_count": chunk_count,
                    }),
                    external_id: Some(external_id),
                    scope: None,
                }
            })
            .collect();
        Some(items)
    }

    fn derived_scope(&self, item: &NewItem) -> Option<String> {
        let cwd = item.metadata.get("cwd")?.as_str()?;
        if let Some((seen, result)) = self.derived_memo.borrow().as_ref() {
            if seen == cwd {
                return result.clone();
            }
        }
        let result = derive_scope("codex", cwd);
        *self.derived_memo.borrow_mut() = Some((cwd.to_string(), result.clone()));
        result
    }
}

/// `<prefix>/<basename of dir>` validated as a scope, warning on failure.
pub(crate) fn derive_scope(prefix: &str, dir: &str) -> Option<String> {
    let Some(base) = Path::new(dir).file_name() else {
        tracing::warn!(dir, "directory has no basename; item left unscoped");
        return None;
    };
    let Some(base) = base.to_str() else {
        tracing::warn!(dir, "basename is not valid UTF-8; item left unscoped");
        return None;
    };
    match singularmem_core::scope::validate(&format!("{prefix}/{base}")) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(dir, error = %e, "basename is not a valid scope segment; item left unscoped");
            None
        }
    }
}

impl Source for CodexRollout {
    fn name(&self) -> String {
        self.path.display().to_string()
    }

    fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_> {
        self.filtered.set(0);
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(source) => return Box::new(std::iter::once(Err(Error::Io { path: self.path.clone(), source }))),
        };
        let mut filter = self.project_filter.as_deref().map(ProjectFilter::new);
        let mut session = Session { id: self.file_stem(), cwd: None };
        let mut warned_no_meta = false;
        let iter = BufReader::new(file).lines().enumerate().flat_map(move |(idx, line)| {
            let line_no = idx + 1;
            let raw = match line {
                Ok(l) => l,
                Err(source) => return vec![Err(Error::Io { path: self.path.clone(), source })],
            };
            if raw.trim().is_empty() {
                return Vec::new();
            }
            let parsed: Line = match serde_json::from_str(&raw) {
                Ok(p) => p,
                Err(source) => return vec![Err(Error::Json { path: self.path.clone(), line: line_no, source })],
            };
            if parsed.kind == "session_meta" {
                if let Some(p) = &parsed.payload {
                    if let Some(id) = p.get("id").and_then(|v| v.as_str()) {
                        session.id = id.to_string();
                    }
                    session.cwd = p.get("cwd").and_then(|v| v.as_str()).map(str::to_string);
                }
                return Vec::new();
            }
            if line_no == 1 && !warned_no_meta {
                warned_no_meta = true;
                tracing::warn!(path = %self.path.display(), "rollout has no session_meta line; using file stem as session id");
            }
            self.line_to_items(line_no, &parsed, &session, filter.as_mut()).map_or_else(Vec::new, |items| {
                if items.is_empty() {
                    self.filtered.set(self.filtered.get() + 1);
                }
                items.into_iter().map(Ok).collect()
            })
        });
        Box::new(iter)
    }

    fn filtered_count(&self) -> usize {
        self.filtered.get()
    }

    fn default_scope(&self, item: &NewItem) -> Option<String> {
        if let Some(o) = &self.scope_override {
            match singularmem_core::scope::validate(o) {
                Ok(s) => return Some(s),
                Err(e) => tracing::warn!(r#override = %o, error = %e, "ignoring invalid scope override; using derived scope"),
            }
        }
        self.derived_scope(item)
    }
}

/// `$HOME/.codex/sessions`, if a home directory is known.
#[must_use]
pub fn default_codex_root() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex").join("sessions"))
}

/// Recursively find `rollout-*.jsonl` files under `root`, sorted.
///
/// # Errors
/// `Error::NotFound` if `root` does not exist; `Error::Io` on read failure.
pub fn discover_codex_sessions(root: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let mut all = crate::claude::discover_transcripts(root)?;
    all.retain(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rollout-")));
    Ok(all)
}
```

Refactor `claude.rs::compute_derived_scope` to call `crate::codex::derive_scope("claude-code", cwd)` (or move `derive_scope` into `project_filter.rs` as the shared home — pick one; the point is one implementation). `lib.rs`: `pub mod codex; pub(crate) mod project_filter;` and `pub use codex::{default_codex_root, discover_codex_sessions, CodexRollout};`.

- [ ] **Step 5: Run, lint, commit**

```bash
git add crates/singularmem-ingest
git commit -s -m "feat(ingest): Codex CLI rollout source"
```

---

### Task 3: Cursor chat source

**Files:**
- Create: `crates/singularmem-ingest/src/cursor.rs`, `crates/singularmem-ingest/tests/cursor_parse.rs`
- Modify: `crates/singularmem-ingest/Cargo.toml` (`rusqlite = { workspace = true }`), `src/lib.rs`

**Interfaces:**
- Produces: `pub struct CursorChats { pub user_dir: PathBuf, pub project_filter: Option<PathBuf>, pub conversation_filter: Option<String>, pub scope_override: Option<String>, pub chunk_bytes: usize, .. }`, `CursorChats::open(user_dir) -> Result<Self>` (`NotFound` unless `globalStorage/state.vscdb` exists), `impl Source` (`name()` = user dir), `pub fn default_cursor_user_dir() -> Option<PathBuf>`, and a test-support builder `pub fn write_fixture(user_dir: &Path, workspaces: &[FixtureWorkspace])` behind `#[cfg(any(test, feature = "testing"))]` — add a `testing` feature to the crate so the CLI tests can build a fixture too.

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** none
**Blocks:** Task 5

- [ ] **Step 1: Failing tests** — `tests/cursor_parse.rs`:

```rust
use std::path::PathBuf;

use singularmem_ingest::cursor::{write_fixture, FixtureBubble, FixtureWorkspace};
use singularmem_ingest::{CursorChats, Source};
use tempfile::TempDir;

fn fixture() -> (TempDir, PathBuf) {
    let d = TempDir::new().unwrap();
    let user = d.path().join("User");
    write_fixture(
        &user,
        &[
            FixtureWorkspace {
                hash: "aaa",
                folder: Some("/home/me/proj"),
                composers: vec![("c1", "Fix the build", 1_741_900_656_013, vec![
                    FixtureBubble { id: "b1", kind: 1, text: "why does the build fail?" },
                    FixtureBubble { id: "b2", kind: 2, text: "Because of X." },
                    FixtureBubble { id: "b3", kind: 2, text: "" },
                ])],
            },
            FixtureWorkspace {
                hash: "bbb",
                folder: Some("/home/me/other"),
                composers: vec![("c2", "Other", 1_741_900_700_000, vec![
                    FixtureBubble { id: "b4", kind: 1, text: "hello other" },
                ])],
            },
            FixtureWorkspace { hash: "ccc", folder: None, composers: vec![] },
        ],
    );
    (d, user)
}

#[test]
fn parses_workspaces_composers_and_bubbles() {
    let (_d, user) = fixture();
    let src = CursorChats::open(&user).unwrap();
    let items: Vec<_> = src.items().map(Result::unwrap).collect();
    let ids: Vec<&str> = items.iter().map(|i| i.external_id.as_deref().unwrap()).collect();
    assert_eq!(ids, vec!["cursor:c1:b1", "cursor:c1:b2", "cursor:c2:b4"]);
    assert_eq!(items[0].tags, vec!["cursor", "role:user", "transcript"]);
    assert_eq!(items[1].metadata["role"], "assistant");
    assert_eq!(items[1].metadata["title"], "Fix the build");
    assert_eq!(items[1].metadata["workspace"], "/home/me/proj");
    assert_eq!(items[1].metadata["composer_created_at"], "2025-03-13T21:17:36.013Z");
    assert_eq!(items[1].metadata["index"], 1);
    assert_eq!(src.default_scope(&items[0]).as_deref(), Some("cursor/proj"));
    assert_eq!(src.default_scope(&items[2]).as_deref(), Some("cursor/other"));
    // b3 (empty text) filtered; workspace ccc (no workspace.json) counted as filtered.
    assert_eq!(src.filtered_count(), 2);
}

#[test]
fn filters_by_project_and_conversation() {
    let (_d, user) = fixture();
    let mut src = CursorChats::open(&user).unwrap();
    src.project_filter = Some(PathBuf::from("/home/me/other"));
    assert_eq!(src.items().filter_map(Result::ok).count(), 1);
    src.project_filter = None;
    src.conversation_filter = Some("c1".into());
    assert_eq!(src.items().filter_map(Result::ok).count(), 2);
}

#[test]
fn missing_global_db_is_not_found() {
    let d = TempDir::new().unwrap();
    assert!(matches!(CursorChats::open(d.path()), Err(singularmem_ingest::Error::NotFound { .. })));
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement `cursor.rs`**

```rust
//! Cursor IDE chat source: reads the `state.vscdb` SQLite stores. Nothing
//! here is documented by Cursor; the key shapes were captured from a live
//! install and the parser tolerates missing fields.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use singularmem_core::NewItem;

use crate::chunk::{chunk_text, DEFAULT_CHUNK_BYTES};
use crate::codex::derive_scope;
use crate::error::{Error, Result};
use crate::project_filter::ProjectFilter;
use crate::Source;

/// Cursor's per-user chat store.
#[derive(Debug)]
pub struct CursorChats {
    /// The `…/Cursor/User` directory.
    pub user_dir: PathBuf,
    /// Keep only conversations whose workspace folder names this directory.
    pub project_filter: Option<PathBuf>,
    /// Keep only this composer (conversation) id.
    pub conversation_filter: Option<String>,
    /// Explicit scope override; wins over `cursor/<workspace basename>`.
    pub scope_override: Option<String>,
    /// Chunk cap in bytes.
    pub chunk_bytes: usize,
    filtered: Cell<usize>,
    derived_memo: RefCell<Option<(String, Option<String>)>>,
}

/// One conversation as resolved from the two databases.
struct Composer {
    id: String,
    title: Option<String>,
    created_at: Option<String>,
    workspace: String,
    bubbles: Vec<(String, u8)>, // (bubbleId, type)
}

impl CursorChats {
    /// Open the store rooted at `user_dir`.
    ///
    /// # Errors
    /// `Error::NotFound` if `<user_dir>/globalStorage/state.vscdb` is absent.
    pub fn open(user_dir: impl AsRef<Path>) -> Result<Self> {
        let user_dir = user_dir.as_ref().to_path_buf();
        let global = user_dir.join("globalStorage").join("state.vscdb");
        if !global.is_file() {
            return Err(Error::NotFound { path: global });
        }
        Ok(Self {
            user_dir,
            project_filter: None,
            conversation_filter: None,
            scope_override: None,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            filtered: Cell::new(0),
            derived_memo: RefCell::new(None),
        })
    }

    /// Open a `state.vscdb` read-only without taking SQLite locks
    /// (`immutable=1`), falling back to a temporary copy.
    fn open_db(path: &Path) -> Result<Connection> {
        let uri = format!("file:{}?immutable=1", path.display());
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        match Connection::open_with_flags(&uri, flags) {
            Ok(c) => Ok(c),
            Err(first) => {
                let tmp = std::env::temp_dir().join(format!("singularmem-cursor-{}.vscdb", std::process::id()));
                std::fs::copy(path, &tmp).map_err(|source| Error::Io { path: path.to_path_buf(), source })?;
                Connection::open_with_flags(&tmp, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
                    .map_err(|e| Error::Io { path: path.to_path_buf(), source: std::io::Error::other(format!("{first}; copy fallback: {e}")) })
            }
        }
    }

    /// Enumerate workspaces → composers, using the workspace DBs for the
    /// composer list and the global DB for headers.
    fn composers(&self, global: &Connection) -> Result<Vec<Composer>> {
        let ws_root = self.user_dir.join("workspaceStorage");
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&ws_root) else { return Ok(out) };
        let mut dirs: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
        dirs.sort();
        let mut filter = self.project_filter.as_deref().map(ProjectFilter::new);
        for dir in dirs {
            let Some(folder) = read_workspace_folder(&dir) else {
                self.filtered.set(self.filtered.get() + 1);
                continue;
            };
            if let Some(f) = filter.as_mut() {
                if !f.matches(Some(&folder)) {
                    continue;
                }
            }
            let db_path = dir.join("state.vscdb");
            if !db_path.is_file() {
                continue;
            }
            let ws = Self::open_db(&db_path)?;
            let raw: Option<String> = ws
                .query_row("SELECT value FROM ItemTable WHERE key = 'composer.composerData'", [], |r| r.get(0))
                .ok();
            let Some(raw) = raw else { continue };
            let data: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
            for c in data.get("allComposers").and_then(|a| a.as_array()).into_iter().flatten() {
                let Some(id) = c.get("composerId").and_then(|v| v.as_str()) else { continue };
                if self.conversation_filter.as_deref().is_some_and(|want| want != id) {
                    continue;
                }
                let head: Option<String> = global
                    .query_row("SELECT value FROM cursorDiskKV WHERE key = ?1", [format!("composerData:{id}")], |r| r.get(0))
                    .ok();
                let Some(head) = head else { continue };
                let h: serde_json::Value = serde_json::from_str(&head).unwrap_or(serde_json::Value::Null);
                let bubbles = h
                    .get("fullConversationHeadersOnly")
                    .or_else(|| h.get("conversation"))
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|b| Some((b.get("bubbleId")?.as_str()?.to_string(), u8::try_from(b.get("type")?.as_u64()?).ok()?)))
                            .collect()
                    })
                    .unwrap_or_default();
                let created_ms = h.get("createdAt").and_then(|v| v.as_i64()).or_else(|| c.get("createdAt").and_then(|v| v.as_str()?.parse().ok()));
                out.push(Composer {
                    id: id.to_string(),
                    title: h.get("name").and_then(|v| v.as_str()).map(str::to_string),
                    created_at: created_ms.and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok()).map(|t| t.to_string()),
                    workspace: folder.clone(),
                    bubbles,
                });
            }
        }
        Ok(out)
    }

    fn bubble_items(&self, global: &Connection, c: &Composer) -> Vec<Result<NewItem>> {
        let mut out = Vec::new();
        for (index, (bubble_id, kind)) in c.bubbles.iter().enumerate() {
            let role = match kind { 1 => "user", 2 => "assistant", _ => { self.filtered.set(self.filtered.get() + 1); continue } };
            let raw: Option<String> = global
                .query_row("SELECT value FROM cursorDiskKV WHERE key = ?1", [format!("bubbleId:{}:{bubble_id}", c.id)], |r| r.get(0))
                .ok();
            let text = raw
                .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
                .and_then(|b| b.get("text").and_then(|t| t.as_str()).map(str::to_string))
                .unwrap_or_default();
            let chunks = chunk_text(&text, self.chunk_bytes);
            if chunks.is_empty() {
                self.filtered.set(self.filtered.get() + 1);
                continue;
            }
            let chunk_count = chunks.len();
            for (i, content) in chunks.into_iter().enumerate() {
                let mut tags = vec!["cursor".to_string(), format!("role:{role}"), "transcript".to_string()];
                tags.sort();
                let external_id = if chunk_count == 1 { format!("cursor:{}:{bubble_id}", c.id) } else { format!("cursor:{}:{bubble_id}#{i}", c.id) };
                out.push(Ok(NewItem {
                    content,
                    supersedes: None,
                    tags,
                    source: Some(format!("cursor:{}", c.id)),
                    metadata: serde_json::json!({
                        "composer_id": c.id, "bubble_id": bubble_id, "index": index, "role": role,
                        "title": c.title, "workspace": c.workspace, "composer_created_at": c.created_at,
                        "chunk_index": i, "chunk_count": chunk_count,
                    }),
                    external_id: Some(external_id),
                    scope: None,
                }));
            }
        }
        out
    }
}

/// `folder` from `<workspace dir>/workspace.json`, as a filesystem path.
fn read_workspace_folder(dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(dir.join("workspace.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let folder = v.get("folder")?.as_str()?;
    Some(folder.strip_prefix("file://").map_or(folder, |p| p).to_string())
}

impl Source for CursorChats {
    fn name(&self) -> String {
        self.user_dir.display().to_string()
    }

    fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_> {
        self.filtered.set(0);
        let global_path = self.user_dir.join("globalStorage").join("state.vscdb");
        let global = match Self::open_db(&global_path) {
            Ok(c) => c,
            Err(e) => return Box::new(std::iter::once(Err(e))),
        };
        let composers = match self.composers(&global) {
            Ok(c) => c,
            Err(e) => return Box::new(std::iter::once(Err(e))),
        };
        let mut all = Vec::new();
        for c in &composers {
            all.extend(self.bubble_items(&global, c));
        }
        Box::new(all.into_iter())
    }

    fn filtered_count(&self) -> usize {
        self.filtered.get()
    }

    fn default_scope(&self, item: &NewItem) -> Option<String> {
        if let Some(o) = &self.scope_override {
            match singularmem_core::scope::validate(o) {
                Ok(s) => return Some(s),
                Err(e) => tracing::warn!(r#override = %o, error = %e, "ignoring invalid scope override; using derived scope"),
            }
        }
        let ws = item.metadata.get("workspace")?.as_str()?;
        if let Some((seen, result)) = self.derived_memo.borrow().as_ref() {
            if seen == ws {
                return result.clone();
            }
        }
        let result = derive_scope("cursor", ws);
        *self.derived_memo.borrow_mut() = Some((ws.to_string(), result.clone()));
        result
    }
}

/// Cursor's per-OS user directory.
#[must_use]
pub fn default_cursor_user_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Cursor/User"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Cursor").join("User"))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/Cursor/User"))
    }
}

/// Test-support: a bubble in a fixture conversation.
#[cfg(any(test, feature = "testing"))]
pub struct FixtureBubble { pub id: &'static str, pub kind: u8, pub text: &'static str }

/// Test-support: a fixture workspace.
#[cfg(any(test, feature = "testing"))]
pub struct FixtureWorkspace {
    pub hash: &'static str,
    pub folder: Option<&'static str>,
    /// `(composerId, title, createdAt ms, bubbles)`
    pub composers: Vec<(&'static str, &'static str, i64, Vec<FixtureBubble>)>,
}

/// Test-support: write a miniature Cursor user dir with the real key shapes.
///
/// # Panics
/// On any I/O or SQLite failure (test helper).
#[cfg(any(test, feature = "testing"))]
pub fn write_fixture(user_dir: &Path, workspaces: &[FixtureWorkspace]) {
    std::fs::create_dir_all(user_dir.join("globalStorage")).unwrap();
    let global = Connection::open(user_dir.join("globalStorage/state.vscdb")).unwrap();
    global.execute_batch("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB); CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value BLOB);").unwrap();
    for ws in workspaces {
        let dir = user_dir.join("workspaceStorage").join(ws.hash);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(folder) = ws.folder {
            std::fs::write(dir.join("workspace.json"), format!("{{\"folder\":\"file://{folder}\"}}")).unwrap();
        }
        let wsdb = Connection::open(dir.join("state.vscdb")).unwrap();
        wsdb.execute_batch("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB); CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value BLOB);").unwrap();
        let all: Vec<serde_json::Value> = ws.composers.iter().map(|(id, _, created, _)| serde_json::json!({"type":"head","composerId":id,"createdAt":created.to_string()})).collect();
        wsdb.execute("INSERT INTO ItemTable VALUES ('composer.composerData', ?1)", [serde_json::json!({"allComposers": all}).to_string()]).unwrap();
        for (id, title, created, bubbles) in &ws.composers {
            let headers: Vec<serde_json::Value> = bubbles.iter().map(|b| serde_json::json!({"bubbleId": b.id, "type": b.kind})).collect();
            global.execute("INSERT INTO cursorDiskKV VALUES (?1, ?2)", [format!("composerData:{id}"), serde_json::json!({"composerId": id, "name": title, "createdAt": created, "fullConversationHeadersOnly": headers}).to_string()]).unwrap();
            for b in bubbles {
                global.execute("INSERT INTO cursorDiskKV VALUES (?1, ?2)", [format!("bubbleId:{id}:{}", b.id), serde_json::json!({"bubbleId": b.id, "type": b.kind, "text": b.text}).to_string()]).unwrap();
            }
        }
    }
}
```

`Cargo.toml`: add `rusqlite = { workspace = true }` under `[dependencies]` and `[features] testing = []`. `lib.rs`: `pub mod cursor; pub use cursor::{default_cursor_user_dir, CursorChats};`. Note `jiff::Timestamp::from_millisecond(ms)` renders `2025-03-13T21:17:36.013Z`; if the test's expected string differs by trailing zeros, assert with `starts_with("2025-03-13T21:17:36")` instead — check `jiff`'s Display first.

- [ ] **Step 4: Run, lint, commit**

```bash
git add crates/singularmem-ingest Cargo.lock
git commit -s -m "feat(ingest): Cursor chat source over state.vscdb"
```

---

### Task 4: `singularmem-hooks` crate

**Files:**
- Create: `crates/singularmem-hooks/Cargo.toml`, `src/lib.rs`, `src/editor.rs` (`Editor`, `Event`, `config_path`), `src/input.rs` (`HookInput`, `parse_input`), `src/envelope.rs`, `src/config.rs` (`entries`, `merge`, `remove`, `status`, `HookStatus`, `read_config`, `write_config`), `tests/config_merge.rs`, `tests/input_envelope.rs`
- Modify: `.github/workflows/publish-cargo.yml` (add `singularmem-hooks` after `singularmem-ingest`)

**Interfaces:**
- Produces:

```rust
pub enum Editor { ClaudeCode, Codex, Cursor }   // FromStr: "claude-code"|"codex"|"cursor"; Display same
pub enum Event { SessionStart, Stop, PreCompact, SessionEnd }   // FromStr: "session-start"|"stop"|"pre-compact"|"session-end"
pub struct HookInput { pub cwd: Option<PathBuf>, pub workspace_roots: Vec<PathBuf>, pub transcript_path: Option<PathBuf>, pub session_id: Option<String>, pub conversation_id: Option<String> }
pub fn parse_input(editor: Editor, json: &serde_json::Value) -> HookInput;
pub fn session_start_envelope(editor: Editor, text: &str) -> serde_json::Value;
pub fn entries(editor: Editor, bin: &Path) -> serde_json::Value;   // full hooks object for that editor
pub fn merge(editor: Editor, existing: &serde_json::Value, bin: &Path) -> serde_json::Value;
pub fn remove(editor: Editor, existing: &serde_json::Value) -> serde_json::Value;
pub struct HookStatus { pub installed: bool, pub bin: Option<PathBuf>, pub bin_exists: bool }
pub fn status(editor: Editor, existing: &serde_json::Value) -> HookStatus;
pub fn config_path(editor: Editor, project: Option<&Path>) -> Result<PathBuf>;   // env HOME/APPDATA aware
pub fn read_config(path: &Path) -> Result<serde_json::Value>;   // {} if missing; Error::InvalidJson if unparsable
pub fn write_config(path: &Path, value: &serde_json::Value) -> Result<()>;   // atomic, 2-space indent, trailing newline
pub const MARKER: &str = "singularmem hook ";
pub enum Error { InvalidJson { path, source }, Io { path, source }, NoHome }
```

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** none
**Blocks:** Task 6

- [ ] **Step 1: Failing tests** — `tests/config_merge.rs`:

```rust
use std::path::Path;

use serde_json::json;
use singularmem_hooks::{entries, merge, remove, status, Editor, MARKER};

const BIN: &str = "/opt/bin/singularmem";

#[test]
fn claude_entries_match_spec() {
    let e = entries(Editor::ClaudeCode, Path::new(BIN));
    let hooks = &e["hooks"];
    assert_eq!(hooks["SessionStart"][0]["matcher"], "startup|resume|clear|compact");
    assert_eq!(hooks["SessionStart"][0]["hooks"][0]["command"], format!("\"{BIN}\" hook claude-code session-start"));
    assert_eq!(hooks["SessionStart"][0]["hooks"][0]["timeout"], 30);
    assert_eq!(hooks["Stop"][0]["hooks"][0]["async"], true);
    assert_eq!(hooks["PreCompact"][0]["hooks"][0]["timeout"], 60);
    assert_eq!(hooks["SessionEnd"][0]["hooks"][0]["timeout"], 60);
    assert!(hooks["Stop"][0].get("matcher").is_none());
}

#[test]
fn codex_and_cursor_entries_match_spec() {
    let c = entries(Editor::Codex, Path::new(BIN));
    assert_eq!(c["hooks"]["SessionStart"][0]["matcher"], "*");
    assert_eq!(c["hooks"]["Stop"][0]["hooks"][0]["async"], true);
    assert!(c["hooks"].get("SessionEnd").is_none());
    let u = entries(Editor::Cursor, Path::new(BIN));
    assert_eq!(u["version"], 1);
    assert_eq!(u["hooks"]["stop"][0]["loop_limit"], 1);
    assert_eq!(u["hooks"]["sessionStart"][0]["command"], format!("\"{BIN}\" hook cursor session-start"));
    assert_eq!(u["hooks"]["sessionStart"][0]["timeout"], 30);
}

#[test]
fn merge_preserves_foreign_entries_and_is_idempotent() {
    let existing = json!({
        "permissions": {"allow": ["Bash(ls)"]},
        "hooks": {
            "Stop": [{"hooks": [{"type": "command", "command": "echo other"}]}],
            "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "lint"}]}]
        }
    });
    let once = merge(Editor::ClaudeCode, &existing, Path::new(BIN));
    assert_eq!(once["permissions"]["allow"][0], "Bash(ls)");
    assert_eq!(once["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "lint");
    let stop = once["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop.len(), 2);
    assert_eq!(stop[0]["hooks"][0]["command"], "echo other");
    assert!(stop[1]["hooks"][0]["command"].as_str().unwrap().contains(MARKER));
    let twice = merge(Editor::ClaudeCode, &once, Path::new(BIN));
    assert_eq!(twice, once);
    // Re-installing with a different binary path replaces ours, not theirs.
    let moved = merge(Editor::ClaudeCode, &once, Path::new("/new/singularmem"));
    let stop = moved["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(stop.len(), 2);
    assert!(stop[1]["hooks"][0]["command"].as_str().unwrap().starts_with("\"/new/singularmem\""));
}

#[test]
fn remove_leaves_foreign_entries_byte_identical() {
    let existing = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo other"}]}]}, "x": 1});
    let merged = merge(Editor::ClaudeCode, &existing, Path::new(BIN));
    let removed = remove(Editor::ClaudeCode, &merged);
    assert_eq!(removed, existing);
    // Removing from a config that never had ours is a no-op.
    assert_eq!(remove(Editor::ClaudeCode, &existing), existing);
    // Cursor: empty event arrays are dropped; `version` stays.
    let cur = merge(Editor::Cursor, &json!({}), Path::new(BIN));
    let gone = remove(Editor::Cursor, &cur);
    assert_eq!(gone, json!({"version": 1, "hooks": {}}));
}

#[test]
fn status_detects_ours_and_bin_existence() {
    let none = status(Editor::Codex, &json!({}));
    assert!(!none.installed);
    let d = tempfile::TempDir::new().unwrap();
    let bin = d.path().join("singularmem");
    std::fs::write(&bin, "").unwrap();
    let merged = merge(Editor::Codex, &json!({}), &bin);
    let s = status(Editor::Codex, &merged);
    assert!(s.installed && s.bin_exists);
    assert_eq!(s.bin.as_deref(), Some(bin.as_path()));
    let stale = merge(Editor::Codex, &json!({}), Path::new("/nope/singularmem"));
    assert!(!status(Editor::Codex, &stale).bin_exists);
}
```

`tests/input_envelope.rs`:

```rust
use std::path::PathBuf;

use serde_json::json;
use singularmem_hooks::{config_path, parse_input, session_start_envelope, Editor, Event};

#[test]
fn parses_each_editor_payload() {
    let c = parse_input(Editor::ClaudeCode, &json!({"session_id":"s","transcript_path":"/t.jsonl","cwd":"/w/p","hook_event_name":"Stop"}));
    assert_eq!(c.cwd, Some(PathBuf::from("/w/p")));
    assert_eq!(c.transcript_path, Some(PathBuf::from("/t.jsonl")));
    assert_eq!(c.session_id.as_deref(), Some("s"));
    let k = parse_input(Editor::Codex, &json!({"session_id":"k","transcript_path":null,"cwd":"/w/p"}));
    assert_eq!(k.transcript_path, None);
    assert_eq!(k.session_id.as_deref(), Some("k"));
    let u = parse_input(Editor::Cursor, &json!({"conversation_id":"c1","workspace_roots":["/w/p","/w/q"],"session_id":"x"}));
    assert_eq!(u.conversation_id.as_deref(), Some("c1"));
    assert_eq!(u.workspace_roots, vec![PathBuf::from("/w/p"), PathBuf::from("/w/q")]);
    assert_eq!(u.cwd, Some(PathBuf::from("/w/p")), "first root doubles as cwd");
    let empty = parse_input(Editor::ClaudeCode, &json!("not an object"));
    assert!(empty.cwd.is_none() && empty.transcript_path.is_none());
}

#[test]
fn envelopes() {
    assert_eq!(session_start_envelope(Editor::ClaudeCode, "hi"), json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"hi"}}));
    assert_eq!(session_start_envelope(Editor::Codex, "hi"), json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"hi"}}));
    assert_eq!(session_start_envelope(Editor::Cursor, "hi"), json!({"additional_context":"hi"}));
}

#[test]
fn parses_editor_and_event_names() {
    assert_eq!("claude-code".parse::<Editor>().unwrap(), Editor::ClaudeCode);
    assert_eq!("pre-compact".parse::<Event>().unwrap(), Event::PreCompact);
    assert!("vim".parse::<Editor>().is_err());
}

#[test]
fn config_paths_follow_env_and_project() {
    let d = tempfile::TempDir::new().unwrap();
    temp_env::with_vars([("HOME", Some(d.path().to_str().unwrap())), ("APPDATA", Some(d.path().to_str().unwrap()))], || {
        assert_eq!(config_path(Editor::ClaudeCode, None).unwrap(), d.path().join(".claude/settings.json"));
        assert_eq!(config_path(Editor::Codex, None).unwrap(), d.path().join(".codex/hooks.json"));
        assert_eq!(config_path(Editor::Cursor, None).unwrap(), d.path().join(".cursor/hooks.json"));
        let p = PathBuf::from("/repo");
        assert_eq!(config_path(Editor::ClaudeCode, Some(&p)).unwrap(), PathBuf::from("/repo/.claude/settings.json"));
        assert_eq!(config_path(Editor::Codex, Some(&p)).unwrap(), PathBuf::from("/repo/.codex/hooks.json"));
        assert_eq!(config_path(Editor::Cursor, Some(&p)).unwrap(), PathBuf::from("/repo/.cursor/hooks.json"));
    });
}
```

Add `temp-env = "0.3"` to the crate's dev-dependencies for the env test.

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement**

`Cargo.toml`: deps `serde`, `serde_json`, `thiserror`; dev-deps `tempfile`, `temp-env`. `lib.rs` with `#![forbid(unsafe_code)]`, modules, re-exports, and `pub const MARKER: &str = "singularmem hook ";`.

`editor.rs`: the two enums with `FromStr`/`Display` and

```rust
pub fn config_path(editor: Editor, project: Option<&Path>) -> Result<PathBuf> {
    let (dir, file) = match editor {
        Editor::ClaudeCode => (".claude", "settings.json"),
        Editor::Codex => (".codex", "hooks.json"),
        Editor::Cursor => (".cursor", "hooks.json"),
    };
    let base = match project {
        Some(p) => p.to_path_buf(),
        None => std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).ok_or(Error::NoHome)?,
    };
    Ok(base.join(dir).join(file))
}
```

`input.rs`: `parse_input` reads, per editor, `cwd` (string), `workspace_roots` (array of strings; `cwd` falls back to the first root), `transcript_path` (string or null), `session_id`, `conversation_id`; non-object input yields all-`None`.

`envelope.rs`: `session_start_envelope` per the constraints table.

`config.rs`: `command(editor, event, bin) -> String` = `format!("\"{}\" hook {editor} {event}", bin.display())`; `entries(editor, bin)` builds the exact JSON in the Global Constraints; `merge` = deep-copy `existing` (must be an object; if not, start from `{}`), ensure `hooks` object (Cursor also `version: 1` if absent), for each event key in `entries`: take the existing array (or new), retain elements that are not ours, then append ours. "Ours" for Claude/Codex = a group whose inner `hooks[]` contains a command containing `MARKER` (the group is removed whole; a foreign group with our command mixed in is left intact and a warning is not needed here — the installer never produces mixed groups); for Cursor = an entry whose `command` contains `MARKER`. `remove` = same retain without appending, deleting event arrays that become empty and the `hooks` object if it becomes empty for Claude/Codex (Cursor keeps `"hooks": {}` and `version`). `status` = scan all event arrays for ours, extract the quoted bin path from the first ours command (text between the leading quotes), `bin_exists = Path::new(&bin).exists()`. `read_config` returns `json!({})` when the file is missing, `Error::InvalidJson` when present but unparsable. `write_config` creates parent dirs, writes `serde_json::to_string_pretty` + `\n` to `<path>.tmp` then `rename`s.

- [ ] **Step 4: Run, lint, commit**

```bash
git add crates/singularmem-hooks .github/workflows/publish-cargo.yml Cargo.lock
git commit -s -m "feat(hooks): singularmem-hooks crate — editor envelopes and config merging"
```

---

### Task 5: CLI — `ingest-codex`, `ingest-cursor`, `wake-up`

**Files:**
- Modify: `Cargo.toml` (root deps: `singularmem-ingest = { path = "...", features = ["testing"] }` under dev-deps too, `singularmem-hooks`), `src/main.rs`, `tests/cli.rs`
- Create: `tests/fixtures/codex/2026/09/01/rollout-2026-09-01T10-00-00-sess1.jsonl` (copy of the crate fixture)

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** Tasks 1, 2, 3
**Blocks:** Task 6

- [ ] **Step 1: Failing CLI tests** (append to `tests/cli.rs`):

```rust
fn fixture_codex() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex")
}

#[test]
fn ingest_codex_is_idempotent_and_scoped() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem().args(["--store", db_s, "ingest-codex", fixture_codex().to_str().unwrap(), "--quiet"]).assert().code(1)
        .stderr(predicate::str::contains("ingested 2, skipped 0 existing, 2 filtered, 1 failed across 1 files"));
    singularmem().args(["--store", db_s, "ingest-codex", fixture_codex().to_str().unwrap(), "--quiet"]).assert().code(1)
        .stderr(predicate::str::contains("ingested 0, skipped 2 existing"));
    singularmem().args(["--store", db_s, "scope", "list"]).assert().success().stdout("codex/proj\t2\n");
}

#[test]
fn ingest_cursor_reads_a_fixture_user_dir() {
    use singularmem_ingest::cursor::{write_fixture, FixtureBubble, FixtureWorkspace};
    let dir = TempDir::new().unwrap();
    let user = dir.path().join("User");
    write_fixture(&user, &[FixtureWorkspace { hash: "h1", folder: Some("/w/proj"), composers: vec![("c1", "T", 1_700_000_000_000, vec![
        FixtureBubble { id: "b1", kind: 1, text: "hello cursor" }, FixtureBubble { id: "b2", kind: 2, text: "hi" }])] }]);
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem().args(["--store", db_s, "ingest-cursor", "--cursor-dir", user.to_str().unwrap(), "--quiet"]).assert().success()
        .stderr(predicate::str::contains("ingested 2, skipped 0 existing, 0 filtered, 0 failed across 1 files"));
    singularmem().args(["--store", db_s, "ingest-cursor", "--cursor-dir", user.to_str().unwrap(), "--conversation", "c1", "--quiet"]).assert().success()
        .stderr(predicate::str::contains("ingested 0, skipped 2 existing"));
    singularmem().args(["--store", db_s, "scope", "list"]).assert().success().stdout("cursor/proj\t2\n");
    singularmem().args(["--store", db_s, "ingest-cursor", "--cursor-dir", dir.path().join("nope").to_str().unwrap()]).assert().code(2);
}

#[test]
fn wake_up_text_json_and_hook_formats() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    for (c, sc) in [("first note", "claude-code/proj"), ("second note", "codex/proj"), ("a file", "files/proj"), ("elsewhere", "claude-code/other")] {
        singularmem().args(["--store", db_s, "ingest", "--content", c, "--scope", sc]).assert().success();
    }
    let out = singularmem().args(["--store", db_s, "wake-up", "--project", "/x/proj"]).assert().success().get_output().stdout.clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with("# Singularmem wake-up — claude-code/proj, codex/proj, cursor/proj — 2 items, showing last 2\n"));
    assert!(text.contains("first note") && text.contains("second note"));
    assert!(!text.contains("a file") && !text.contains("elsewhere"));

    let out = singularmem().args(["--store", db_s, "wake-up", "--project", "/x/proj", "--include-files", "--limit", "1"]).assert().success().get_output().stdout.clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("3 items, showing last 1") && text.contains("a file"));

    let out = singularmem().args(["--store", db_s, "wake-up", "--scope", "claude-code/other", "--format", "claude-hook"]).assert().success().get_output().stdout.clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(v["hookSpecificOutput"]["additionalContext"].as_str().unwrap().contains("elsewhere"));
    let out = singularmem().args(["--store", db_s, "wake-up", "--scope", "claude-code/other", "--format", "cursor-hook"]).assert().success().get_output().stdout.clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(v["additional_context"].as_str().unwrap().contains("elsewhere"));
    let out = singularmem().args(["--store", db_s, "wake-up", "--scope", "claude-code/other", "--format", "json"]).assert().success().get_output().stdout.clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["total"], 1);
    assert_eq!(v["blocks"][0]["content"], "elsewhere");

    let out = singularmem().args(["--store", db_s, "wake-up", "--scope", "nothing/here"]).assert().success().get_output().stdout.clone();
    assert_eq!(String::from_utf8(out).unwrap(), "# Singularmem wake-up — nothing/here — 0 items, showing last 0\n");
    singularmem().args(["--store", db_s, "wake-up", "--scope", "a//b"]).assert().code(1);
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement**

Root `Cargo.toml`: `singularmem-hooks = { path = "crates/singularmem-hooks" }` in `[dependencies]`; `singularmem-ingest = { path = "crates/singularmem-ingest", features = ["testing"] }` in `[dev-dependencies]` (the normal dep stays without the feature).

`src/main.rs`:

- Generalise `ingest_each_transcript` into

```rust
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
            eprintln!("{}: +{} ingested, {} skipped", file.display(), r.ingested, r.skipped_existing + r.skipped_filtered);
        }
        accumulate(total, r);
    }
    Ok(())
}
```

  and have `cmd_ingest_transcript` call it with a closure that builds a `ClaudeTranscript` and sets its fields; delete `ingest_each_transcript`.
- `IngestCodexArgs { paths: Vec<PathBuf>, project: Option<PathBuf>, dry_run, quiet, scope: Option<String> }` → `cmd_ingest_codex`: default root `default_codex_root()` (usage error if unknown), `discover_codex_sessions` for dirs, `ingest_files` with `CodexRollout::open` + field setup; same summary/exit logic as transcripts.
- `IngestCursorArgs { cursor_dir: Option<PathBuf>, project: Option<PathBuf>, conversation: Option<String>, dry_run, quiet, scope: Option<String> }` → `cmd_ingest_cursor`: `CursorChats::open(dir)` (NotFound → exit 2 via the existing `Ingest(NotFound)` arm), set filters/override, one `ingest_source` call, per-source line + `print_summary(&r, 1)`, `IngestPartial` on failures.
- `WakeUpArgs { scope: Vec<String> (repeatable --scope), project: Option<PathBuf>, include_files: bool, limit: usize = 20, max_bytes: usize = 8192, adapter: String = "plain", format: WakeUpFormat = Text }` with `enum WakeUpFormat { Text, Json, ClaudeHook, CodexHook, CursorHook }` → `cmd_wake_up`:

```rust
fn cmd_wake_up(store: &Store, args: &WakeUpArgs) -> Result<(), CliError> {
    use singularmem_retrieve::wakeup::{build, render, ScopeSet, WakeupOptions};
    let set = if args.scope.is_empty() {
        let dir = match &args.project { Some(p) => p.clone(), None => std::env::current_dir()? };
        ScopeSet::for_project(&dir, args.include_files)
    } else {
        ScopeSet(args.scope.iter().map(|s| singularmem_core::ScopeFilter::descendants(s)).collect::<Result<_, _>>()?)
    };
    let adapter = find_adapter(&args.adapter)?; // extract from cmd_retrieve's lookup
    let w = build(store, &set, &WakeupOptions { limit: args.limit, max_bytes: args.max_bytes })?;
    let text = render(&w, adapter, args.max_bytes);
    let mut out = io::stdout().lock();
    match args.format {
        WakeUpFormat::Text => write!(out, "{text}")?,
        WakeUpFormat::Json => {
            serde_json::to_writer(&mut out, &serde_json::json!({ "scopes": w.scopes, "total": w.total, "shown": w.shown, "blocks": w.context.blocks, "text": text }))?;
            writeln!(out)?;
        }
        WakeUpFormat::ClaudeHook => { serde_json::to_writer(&mut out, &singularmem_hooks::session_start_envelope(singularmem_hooks::Editor::ClaudeCode, &text))?; writeln!(out)?; }
        WakeUpFormat::CodexHook => { serde_json::to_writer(&mut out, &singularmem_hooks::session_start_envelope(singularmem_hooks::Editor::Codex, &text))?; writeln!(out)?; }
        WakeUpFormat::CursorHook => { serde_json::to_writer(&mut out, &singularmem_hooks::session_start_envelope(singularmem_hooks::Editor::Cursor, &text))?; writeln!(out)?; }
    }
    Ok(())
}
```

  `find_adapter(name) -> Result<&'static dyn Adapter>`: refactor `known_adapters()` lookup shared with `cmd_retrieve` (return a `Box<dyn Adapter>` and use `&*`). Add `Command::IngestCodex`, `Command::IngestCursor`, `Command::WakeUp` (clap name `wake-up`), extend `needs_hook` and the read-only pre-check with the two ingest verbs.
- Copy the crate fixture to `tests/fixtures/codex/2026/09/01/rollout-2026-09-01T10-00-00-sess1.jsonl`.

- [ ] **Step 4: Run, lint, commit**

`cargo test --test cli ingest_codex ingest_cursor wake_up`, workspace tests, clippy, fmt.

```bash
git add Cargo.toml Cargo.lock src/main.rs tests
git commit -s -m "feat(cli): ingest-codex, ingest-cursor, and wake-up"
```

---

### Task 6: CLI — `hook <editor> <event>` and `hooks install|uninstall|status`; docs

**Files:**
- Modify: `src/main.rs`, `tests/cli.rs`, `README.md`; Create: `docs/hooks.md`, `tests/fixtures/hooks/claude-stop.json`, `tests/fixtures/hooks/cursor-stop.json`
- Modify: `docs/superpowers/specs/2026-09-04-hooks-wakeup-13-design.md` only if deviations are recorded

**Assigned skill:** `rust-best-practices`, `test-driven-development`, `verification-before-completion`
**Blocked-by:** Tasks 4, 5
**Blocks:** none

- [ ] **Step 1: Failing CLI tests**

`tests/fixtures/hooks/claude-stop.json`: `{"session_id":"11111111-2222-3333-4444-555555555555","transcript_path":"<REPLACED IN TEST>","cwd":"/home/me/proj","hook_event_name":"Stop"}` — the test writes this file itself with the real fixture transcript path, so no static fixture is needed; drop the static files and generate inline.

```rust
#[test]
fn hook_claude_stop_ingests_transcript_and_session_start_prints_envelope() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let transcript = fixture_transcripts().join("proj/session.jsonl");
    let payload = serde_json::json!({"session_id":"s","transcript_path":transcript,"cwd":"/home/me/proj","hook_event_name":"Stop"}).to_string();
    singularmem().args(["--store", db_s, "hook", "claude-code", "stop"]).write_stdin(payload.clone()).assert().success().stdout("");
    singularmem().args(["--store", db_s, "scope", "list"]).assert().success().stdout("claude-code/other\t1\nclaude-code/proj\t3\n");
    // Idempotent on the second stop; still exit 0 even though the fixture has a malformed line.
    singularmem().args(["--store", db_s, "hook", "claude-code", "pre-compact"]).write_stdin(payload).assert().success();

    let start = serde_json::json!({"session_id":"s","cwd":"/home/me/proj","hook_event_name":"SessionStart","source":"startup"}).to_string();
    let out = singularmem().args(["--store", db_s, "hook", "claude-code", "session-start"]).write_stdin(start).assert().success().get_output().stdout.clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
    assert!(ctx.starts_with("# Singularmem wake-up — claude-code/proj, codex/proj, cursor/proj — 3 items"));
    assert!(ctx.contains("Run cargo test."));
}

#[test]
fn hook_never_fails_the_editor() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem().args(["--store", db_s, "hook", "claude-code", "stop"]).write_stdin("{not json").assert().success().stdout("")
        .stderr(predicate::str::contains("hook input"));
    singularmem().args(["--store", db_s, "hook", "claude-code", "stop"]).write_stdin(r#"{"cwd":"/x"}"#).assert().success()
        .stderr(predicate::str::contains("transcript_path"));
    singularmem().args(["--store", db_s, "hook", "codex", "stop"]).write_stdin(r#"{"session_id":"k","transcript_path":"/definitely/missing.jsonl","cwd":"/x"}"#).assert().success();
    let out = singularmem().args(["--store", db_s, "hook", "cursor", "session-start"]).write_stdin(r#"{"workspace_roots":["/x/proj"],"session_id":"q"}"#).assert().success().get_output().stdout.clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(v["additional_context"].as_str().unwrap().contains("0 items"));
}

#[test]
fn hook_cursor_stop_ingests_only_that_conversation() {
    use singularmem_ingest::cursor::{write_fixture, FixtureBubble, FixtureWorkspace};
    let dir = TempDir::new().unwrap();
    let user = dir.path().join("User");
    write_fixture(&user, &[FixtureWorkspace { hash: "h1", folder: Some("/w/proj"), composers: vec![
        ("c1", "A", 1_700_000_000_000, vec![FixtureBubble { id: "b1", kind: 1, text: "one" }]),
        ("c2", "B", 1_700_000_001_000, vec![FixtureBubble { id: "b2", kind: 1, text: "two" }]),
    ] }]);
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem().env("SINGULARMEM_CURSOR_DIR", user.to_str().unwrap())
        .args(["--store", db_s, "hook", "cursor", "stop"]).write_stdin(r#"{"conversation_id":"c2","workspace_roots":["/w/proj"]}"#).assert().success();
    singularmem().args(["--store", db_s, "list", "--format", "table"]).assert().success().stdout(predicate::str::contains("two")).stdout(predicate::str::contains("one").not());
}

#[test]
fn hooks_install_status_uninstall_round_trip() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let original = "{\n  \"permissions\": {\n    \"allow\": [\n      \"Bash(ls)\"\n    ]\n  },\n  \"hooks\": {\n    \"Stop\": [\n      {\n        \"hooks\": [\n          {\n            \"type\": \"command\",\n            \"command\": \"echo other\"\n          }\n        ]\n      }\n    ]\n  }\n}\n";
    std::fs::write(&settings, original).unwrap();
    let h = home.path().to_str().unwrap();

    singularmem().env("HOME", h).args(["hooks", "status"]).assert().success()
        .stdout(predicate::str::contains("claude-code\tabsent"));
    singularmem().env("HOME", h).args(["hooks", "install", "claude-code"]).assert().success()
        .stdout(predicate::str::contains(settings.to_str().unwrap()));
    let after: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(after["permissions"]["allow"][0], "Bash(ls)");
    assert_eq!(after["hooks"]["Stop"].as_array().unwrap().len(), 2);
    assert!(after["hooks"]["SessionStart"][0]["hooks"][0]["command"].as_str().unwrap().contains("hook claude-code session-start"));
    singularmem().env("HOME", h).args(["hooks", "install", "claude-code"]).assert().success();
    let again = std::fs::read_to_string(&settings).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&again).unwrap(), after, "idempotent");
    singularmem().env("HOME", h).args(["hooks", "status", "claude-code"]).assert().success()
        .stdout(predicate::str::contains("claude-code\tinstalled")).stdout(predicate::str::contains("bin ok"));
    let printed = singularmem().env("HOME", h).args(["hooks", "install", "codex", "--print"]).assert().success().get_output().stdout.clone();
    let v: serde_json::Value = serde_json::from_slice(&printed).unwrap();
    assert!(v["hooks"]["SessionStart"].is_array());
    assert!(!home.path().join(".codex/hooks.json").exists(), "--print writes nothing");
    singularmem().env("HOME", h).args(["hooks", "uninstall", "claude-code"]).assert().success();
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), original, "foreign entries byte-identical");
    std::fs::write(&settings, "{ not json").unwrap();
    singularmem().env("HOME", h).args(["hooks", "install", "claude-code"]).assert().code(1)
        .stderr(predicate::str::contains("settings.json"));
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ not json", "never overwrites invalid JSON");
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement**

`src/main.rs`:

- `Command::Hook(HookArgs { editor: String, event: String })` and `Command::Hooks(HooksCommand)` with `enum HooksAction { Install { editor: String, #[arg(long)] project: bool, #[arg(long)] print: bool }, Uninstall { editor: String, #[arg(long)] project: bool }, Status { editor: Option<String> } }`.
- `hooks` subcommands must not open the store: in `run()`, dispatch `Command::Hooks(_)` BEFORE `Store::open_with_options` (return early with `cmd_hooks(&cmd)`). `Command::Hook(_)` needs the store with hooks wired (add it to `needs_hook`); wrap the whole `cmd_hook` body so any `Err` becomes a warning and `Ok(())`:

```rust
fn cmd_hook(store: &Store, args: &HookArgs) -> Result<(), CliError> {
    if let Err(e) = run_hook(store, args) {
        tracing::warn!(error = %e, editor = %args.editor, event = %args.event, "hook failed; editor continues");
    }
    Ok(())
}

fn run_hook(store: &Store, args: &HookArgs) -> Result<(), CliError> {
    use singularmem_hooks::{parse_input, session_start_envelope, Editor, Event};
    let editor: Editor = args.editor.parse().map_err(|_| CliError::Usage(format!("unknown editor '{}'", args.editor)))?;
    let event: Event = args.event.parse().map_err(|_| CliError::Usage(format!("unknown event '{}'", args.event)))?;
    let mut raw = String::new();
    io::stdin().read_to_string(&mut raw)?;
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "could not parse hook input JSON; proceeding with empty input");
        serde_json::Value::Null
    });
    let input = parse_input(editor, &json);
    match event {
        Event::SessionStart => {
            let dir = input.cwd.clone().unwrap_or(std::env::current_dir()?);
            let set = singularmem_retrieve::wakeup::ScopeSet::for_project(&dir, false);
            let opts = singularmem_retrieve::wakeup::WakeupOptions::default();
            let text = match singularmem_retrieve::wakeup::build(store, &set, &opts) {
                Ok(w) => singularmem_retrieve::wakeup::render(&w, &singularmem_retrieve::PlainAdapter, opts.max_bytes),
                Err(e) => { tracing::warn!(error = %e, "wake-up failed; emitting empty context"); String::new() }
            };
            let mut out = io::stdout().lock();
            serde_json::to_writer(&mut out, &session_start_envelope(editor, &text))?;
            writeln!(out)?;
        }
        Event::Stop | Event::PreCompact | Event::SessionEnd => {
            let report = match editor {
                Editor::ClaudeCode => {
                    let Some(p) = input.transcript_path else { return Err(CliError::Usage("hook input has no transcript_path".into())) };
                    let src = singularmem_ingest::ClaudeTranscript::open(&p)?;
                    singularmem_ingest::ingest_source(store, &src, false)?
                }
                Editor::Codex => {
                    let files: Vec<PathBuf> = match input.transcript_path {
                        Some(p) => vec![p],
                        None => {
                            let root = singularmem_ingest::default_codex_root().ok_or_else(|| CliError::Usage("cannot determine home directory".into()))?;
                            let want = input.session_id.clone().unwrap_or_default();
                            singularmem_ingest::discover_codex_sessions(&root)?.into_iter().filter(|f| f.to_string_lossy().contains(&want)).collect()
                        }
                    };
                    let mut total = singularmem_ingest::Report::default();
                    for f in files { let src = singularmem_ingest::CodexRollout::open(&f)?; accumulate(&mut total, singularmem_ingest::ingest_source(store, &src, false)?); }
                    total
                }
                Editor::Cursor => {
                    let user = std::env::var_os("SINGULARMEM_CURSOR_DIR").map(PathBuf::from).or_else(singularmem_ingest::default_cursor_user_dir)
                        .ok_or_else(|| CliError::Usage("cannot determine Cursor user directory".into()))?;
                    let mut src = singularmem_ingest::CursorChats::open(&user)?;
                    src.conversation_filter = input.conversation_id.clone();
                    if src.conversation_filter.is_none() { src.project_filter = input.cwd.clone(); }
                    singularmem_ingest::ingest_source(store, &src, false)?
                }
            };
            tracing::info!(ingested = report.ingested, skipped = report.skipped_existing, failed = report.failed, "hook ingest complete");
        }
    }
    Ok(())
}
```

  `SINGULARMEM_CURSOR_DIR` is a test/override env var; document it in `docs/hooks.md`. The Codex session-id file match uses the rollout filename's trailing UUID; if `session_id` does not appear in any filename, all files are skipped (zero ingested, warn).

- `cmd_hooks`:

```rust
fn cmd_hooks(cmd: &HooksCommand) -> Result<(), CliError> {
    use singularmem_hooks::{config_path, merge, read_config, remove, status, write_config, Editor};
    let bin = std::env::current_exe()?;
    let project_dir = std::env::current_dir()?;
    let parse = |s: &str| s.parse::<Editor>().map_err(|_| CliError::Usage(format!("unknown editor '{s}'; expected claude-code, codex, or cursor")));
    let mut out = io::stdout().lock();
    match &cmd.action {
        HooksAction::Install { editor, project, print } => {
            let editor = parse(editor)?;
            let path = config_path(editor, project.then_some(project_dir.as_path()))?;
            let existing = read_config(&path)?;
            let merged = merge(editor, &existing, &bin);
            if *print { serde_json::to_writer_pretty(&mut out, &merged)?; writeln!(out)?; }
            else { write_config(&path, &merged)?; writeln!(out, "{}", path.display())?; }
        }
        HooksAction::Uninstall { editor, project } => {
            let editor = parse(editor)?;
            let path = config_path(editor, project.then_some(project_dir.as_path()))?;
            let existing = read_config(&path)?;
            if path.exists() { write_config(&path, &remove(editor, &existing))?; }
            writeln!(out, "{}", path.display())?;
        }
        HooksAction::Status { editor } => {
            let editors: Vec<Editor> = match editor { Some(e) => vec![parse(e)?], None => vec![Editor::ClaudeCode, Editor::Codex, Editor::Cursor] };
            for e in editors {
                let path = config_path(e, None)?;
                let cfg = read_config(&path).unwrap_or(serde_json::Value::Null);
                let s = status(e, &cfg);
                writeln!(out, "{e}\t{}\t{}\t{}", if s.installed { "installed" } else { "absent" }, path.display(), if s.installed && s.bin_exists { "bin ok" } else if s.installed { "bin missing" } else { "-" })?;
            }
        }
    }
    Ok(())
}
```

  `write_config` must reproduce the original formatting for the uninstall round-trip test: the test's original is 2-space pretty JSON with a trailing newline, which `serde_json::to_string_pretty` + `"\n"` produces, so the round trip is byte-identical as long as key order is preserved (`serde_json` with the `preserve_order` feature — enable it in `singularmem-hooks`' `Cargo.toml`: `serde_json = { workspace = true, features = ["preserve_order"] }`). Note that enabling `preserve_order` in one crate enables it workspace-wide (feature unification); check that no existing test depends on alphabetical key order (grep tests for `to_string(` comparisons on maps; the CLI `--json` outputs are struct-derived and unaffected).
  `CliError` gains `#[error("{0}")] Hooks(#[from] singularmem_hooks::Error)`; `InvalidJson` maps to exit 1 with the path in the message (default arm).

- README: add an "Editor integration" section (three `hooks install` lines, what each hook does, `wake-up` example). `docs/hooks.md`: per-editor event table, config paths, the exact entries, `SINGULARMEM_STORE`/`SINGULARMEM_CURSOR_DIR`, troubleshooting (`hooks status`, `RUST_LOG=info`), and the note that Codex/Cursor formats are reverse-engineered.

- [ ] **Step 4: Run, lint, smoke, commit**

`cargo test --test cli hook`, workspace tests, clippy, fmt. Smoke on this machine with the release binary: `hooks install claude-code --print`, `hooks status`, `ingest-cursor --dry-run --quiet` (expect `0 failed` across ~188 workspaces), `ingest-codex --dry-run` (expect a "path not found" exit 2 if `~/.codex/sessions` is absent — that is correct), `wake-up --project .`.

```bash
git add src/main.rs tests README.md docs/hooks.md crates/singularmem-hooks/Cargo.toml Cargo.lock
git commit -s -m "feat(cli): hook verb, hooks install/uninstall/status, editor docs"
```

Then the PR: base `main`, title `feat: session hooks and wake-up (sub-project 13)`, body listing the constitution amendment, the three editors, the two parsers, and the verification output.
