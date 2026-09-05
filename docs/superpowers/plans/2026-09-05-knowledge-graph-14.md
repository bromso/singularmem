---
title: Temporal knowledge graph (Sub-project 14)
date: 2026-09-05
status: draft
sub-project: 14-knowledge-graph
spec: ../specs/2026-09-05-knowledge-graph-14-design.md
---

# Temporal Knowledge Graph (Sub-project 14) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An agent-maintained, append-only temporal knowledge graph (entities + facts with validity windows, provenance, scope) in the store, with `graph` CLI verbs and six MCP tools; store format v4 and `export-v2`; `src/main.rs` split into `src/commands/`.

**Architecture:** Two new tables (`entities`, `facts`) shaped like `items` — ULID ids, `scope`, supersedes chains, `recorded_at` minted by the store clock. A `graph` module in `singularmem-core` owns every read/write; the CLI and MCP are thin shells. Task 1 is a behaviour-neutral split of the CLI binary, verified by `--help` snapshots.

**Tech Stack:** Rust 1.80, rusqlite 0.32, jiff, serde_json (`preserve_order`), clap 4, rmcp, assert_cmd.

## Global Constraints

- Normalisation: entity = NFC, trim, lowercase, whitespace runs → `_`, strip `'`; 1–256 bytes after normalisation. Predicate = same, then must match `[a-z0-9_]+`, 1–64 bytes. `Validation { field: "entity" | "predicate" }` on failure.
- Timestamps stored as `jiff::Timestamp` display (RFC 3339 UTC); inputs accept `YYYY-MM-DD` (→ `T00:00:00Z`) or a full RFC 3339 timestamp. `valid_from` NULL = since unknown; `valid_to` NULL = open.
- Head revision = row not superseded by any other. Open = head with `valid_to IS NULL`. As-of T: `(valid_from IS NULL OR valid_from <= T) AND (valid_to IS NULL OR T < valid_to)`. Recorded-at R: per chain the newest revision with `recorded_at <= R`; chains starting after R invisible.
- `add_fact` is idempotent on an identical open head in the same scope; `invalidate` inserts a revision with `valid_to` and `supersedes`; `supersede` = invalidate old (tolerated missing) + add new with `valid_from = at`, one transaction; entity `kind` immutable once set (`Validation { field: "kind" }` on conflict).
- Migration 3→4 is exactly the spec's DDL under the existing `run_migration`; `FORMAT_VERSION = "4"`; `EXPORT_FORMAT = "export-v2"`; export order meta, items, entities, facts; loaders ignore unknown `_kind`s.
- MCP: `memory_graph_add|query|invalidate|supersede|timeline|stats`; writers omitted from `tools/list` and rejected in read-only mode.
- CLI exit codes: 0; 1 usage/validation; 2 `NotFound`/read-only; 3 unsupported format. `src/main.rs` ≤ 400 lines after Task 1 with `--help` byte-identical for every subcommand.
- Clippy pedantic + nursery `-D warnings`; fmt; no network in tests; signed-off commits; branch `knowledge-graph-14`.

## File Structure

| Path | Responsibility |
|---|---|
| `src/main.rs` | `main`, `run`, `run_command`, `CliError` + exit mapping only |
| `src/commands/mod.rs` | `Cli`, `Command`, shared arg types (`ScopeArgs`), `resolve_store_path`, `default_store_path` |
| `src/commands/index.rs` | `wire_index_hooks`, `open_index_with_retry`, `is_index_lock_error`, `derive_*_path`, `resolve_search_mode`, `open_or_rebuild_index` |
| `src/commands/items.rs` | `ingest`, `get`, `list`, `revisions`, `export`, `scope` |
| `src/commands/bulk.rs` | `ingest-transcript`, `ingest-codex`, `ingest-dir`, `ingest-cursor`, `ingest_files`, `print_summary`, `codex_root`, `cursor_user_dir` |
| `src/commands/search.rs` | `search`, `retrieve`, `semantic-search`, `reindex`, `known_adapters`, `find_adapter`, `render_search_results` |
| `src/commands/wakeup.rs` | `wake-up`, `write_hook_envelope` |
| `src/commands/hooks.rs` | `hook` entry + save/session-start, `hooks install/uninstall/status` |
| `src/commands/graph.rs` (Task 5) | `graph *` verbs |
| `tests/snapshots/help/*.txt` | `--help` snapshots |
| `crates/singularmem-core/src/id.rs` (new) | `ulid_id!` macro; `EntityId`, `FactId` |
| `crates/singularmem-core/src/graph/{mod,types,normalise,time,write,read}.rs` (new) | graph API |
| `crates/singularmem-core/src/schema.rs`, `store.rs`, `format.rs`, `export.rs`, `error.rs` | v4, export-v2 |
| `docs/formats/store-v4.md` | format spec |
| `crates/singularmem-mcp/src/tools/graph.rs` (new), `server.rs`, `README.md`, `docs/mcp-server.md` | MCP tools |

---

### Task 1: Split `src/main.rs` into `src/commands/` (behaviour-neutral)

**Files:**
- Create: `src/commands/mod.rs`, `index.rs`, `items.rs`, `bulk.rs`, `search.rs`, `wakeup.rs`, `hooks.rs`; `tests/help_snapshots.rs`; `tests/snapshots/help/<name>.txt` (one per subcommand + root)
- Modify: `src/main.rs`

**Interfaces:**
- Produces: the same binary; `run_command(command: Command, store: &Store, store_path: &Path) -> Result<(), CliError>` in `main.rs`; each `commands::<module>::cmd_*` keeps its current signature; `CliError` stays in `main.rs` and is `pub(crate)`.

**Assigned skill:** `rust-best-practices`, `verification-before-completion`
**Blocked-by:** none
**Blocks:** Task 5

- [ ] **Step 1: Snapshot `--help` BEFORE touching anything**

Create `tests/help_snapshots.rs`:

```rust
//! `--help` must be byte-identical across refactors of the CLI binary.
//! Regenerate with `UPDATE_HELP_SNAPSHOTS=1 cargo test --test help_snapshots`.
use assert_cmd::Command;
use std::path::PathBuf;

const SUBCOMMANDS: &[&[&str]] = &[
    &[], &["ingest"], &["ingest-transcript"], &["ingest-codex"], &["ingest-cursor"], &["ingest-dir"],
    &["get"], &["list"], &["revisions"], &["export"], &["search"], &["reindex"], &["retrieve"],
    &["semantic-search"], &["scope"], &["scope", "list"], &["scope", "move"], &["wake-up"],
    &["hook"], &["hooks"], &["hooks", "install"], &["hooks", "uninstall"], &["hooks", "status"],
];

fn snapshot_path(args: &[&str]) -> PathBuf {
    let name = if args.is_empty() { "root".to_string() } else { args.join("-") };
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/help").join(format!("{name}.txt"))
}

#[test]
fn help_output_matches_snapshots() {
    let update = std::env::var_os("UPDATE_HELP_SNAPSHOTS").is_some();
    let mut failures = Vec::new();
    for args in SUBCOMMANDS {
        let mut cmd = Command::cargo_bin("singularmem").unwrap();
        cmd.args(*args).arg("--help");
        // Fixed width so clap's wrapping is deterministic.
        cmd.env("COLUMNS", "100").env("NO_COLOR", "1");
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "--help failed for {args:?}");
        let text = String::from_utf8(out.stdout).unwrap();
        let path = snapshot_path(args);
        if update {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &text).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing snapshot {}", path.display()));
        if expected != text {
            failures.push(format!("{args:?}: --help changed (run with UPDATE_HELP_SNAPSHOTS=1 to accept)"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
```

Run `UPDATE_HELP_SNAPSHOTS=1 cargo test --test help_snapshots` to generate the 23 files, then `cargo test --test help_snapshots` (must pass). Commit: `git add tests && git commit -s -m "test(cli): --help snapshots to guard the main.rs split"`.

- [ ] **Step 2: Move code, module by module**

Create `src/commands/mod.rs` with `pub mod bulk; pub mod hooks; pub mod index; pub mod items; pub mod search; pub mod wakeup;` and move into it: `Cli`, `Command`, every `*Args`/`*Command`/`*Action`/`*Format`/`SearchMode` enum, `ScopeArgs` + its `impl`, `resolve_store_path`, `default_store_path`, `canonicalize_project`. Make items `pub(crate)` as needed. Then move functions per the File Structure table, keeping bodies unchanged; `use crate::CliError;` in each module; `use super::*` is discouraged — import explicitly. `main.rs` keeps `main()`, `CliError`, `run()`, `run_command()`, and `mod commands;`. Move the existing `#[cfg(test)] mod tests` unit tests into the module that owns the code they test (`is_index_lock_error` tests → `commands/index.rs`).

Rules: no behaviour change, no renamed flags, no reordered clap definitions (clap's help order follows declaration order; keep it). If `run_command`'s match grows past clippy's `too_many_lines`, that is acceptable via the existing allow.

- [ ] **Step 3: Verify**

`cargo build`, `cargo test --test help_snapshots` (must pass WITHOUT updating), `cargo test --workspace --all-targets`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all`. `wc -l src/main.rs` ≤ 400.

- [ ] **Step 4: Commit**

```bash
git add src tests
git commit -s -m "refactor(cli): split main.rs into src/commands modules (no behaviour change)"
```

---

### Task 2: Ids, normalisation, time parsing, format v4

**Files:**
- Create: `crates/singularmem-core/src/id.rs`, `crates/singularmem-core/src/graph/mod.rs`, `graph/types.rs`, `graph/normalise.rs`, `graph/time.rs`
- Modify: `crates/singularmem-core/src/lib.rs`, `item.rs` (use the macro for `ItemId`), `format.rs`, `schema.rs`, `store.rs`, `ingest.rs` (`mint_ulid` → `pub(crate) fn mint_raw_ulid(store, now) -> Result<Ulid>`), `error.rs` (no new variants needed)
- Create: `crates/singularmem-core/tests/graph_normalise.rs`; Modify: `tests/migration.rs`

**Interfaces:**
- Produces: `singularmem_core::{EntityId, FactId}` (ULID newtypes with `Display`/`FromStr`/serde transparent, same as `ItemId`); `graph::normalise::{entity_name, predicate}`; `graph::time::parse_point(&str) -> Result<Timestamp>`; the types in the spec (`Entity`, `Fact`, `FactObject`, `NewFact`, `NewObject`, `Direction`, `GraphQuery`, `TimelineEntry`, `GraphStats`, `EntitySummary`) re-exported at `singularmem_core::graph::*`; `FORMAT_VERSION = "4"`; `schema::migrate_3_to_4`.

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** none
**Blocks:** Tasks 3, 4

- [ ] **Step 1: Failing tests**

`crates/singularmem-core/tests/graph_normalise.rs`:

```rust
use singularmem_core::graph::normalise::{entity_name, predicate};
use singularmem_core::graph::time::parse_point;
use singularmem_core::Error;

#[test]
fn entity_names_normalise() {
    assert_eq!(entity_name("  Singular Mem ").unwrap(), "singular_mem");
    assert_eq!(entity_name("Jonas's  Laptop").unwrap(), "jonass_laptop");
    assert_eq!(entity_name("Tantivy").unwrap(), "tantivy");
    assert_eq!(entity_name("café").unwrap(), "café"); // NFC, lowercase, non-ASCII allowed
    assert!(matches!(entity_name("   "), Err(Error::Validation { field: "entity", .. })));
    assert!(matches!(entity_name(&"x".repeat(257)), Err(Error::Validation { field: "entity", .. })));
}

#[test]
fn predicates_normalise_and_restrict() {
    assert_eq!(predicate("Uses").unwrap(), "uses");
    assert_eq!(predicate("Works At").unwrap(), "works_at");
    assert!(matches!(predicate("uses-db"), Err(Error::Validation { field: "predicate", .. })));
    assert!(matches!(predicate("café"), Err(Error::Validation { field: "predicate", .. })));
    assert!(matches!(predicate(&"p".repeat(65)), Err(Error::Validation { field: "predicate", .. })));
}

#[test]
fn time_points_accept_dates_and_timestamps() {
    assert_eq!(parse_point("2026-05-16").unwrap().to_string(), "2026-05-16T00:00:00Z");
    assert_eq!(parse_point("2026-05-16T10:20:30Z").unwrap().to_string(), "2026-05-16T10:20:30Z");
    assert!(matches!(parse_point("yesterday"), Err(Error::Validation { field: "timestamp", .. })));
}
```

Append to `tests/migration.rs` (reuse `make_v1`/`make_v2`; add `make_v3` = `make_v2` + the exact 2→3 statements + version `'3'`):

```rust
#[test]
fn v3_store_migrates_to_v4_with_graph_tables() {
    let dir = TempDir::new().unwrap();
    let path = make_v3(&dir);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.format_version().unwrap(), "4");
    drop(store);
    let conn = Connection::open(&path).unwrap();
    for t in ["entities", "facts"] {
        let n: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1", [t], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "table {t}");
    }
    let n: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_entities_identity'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn v1_store_migrates_through_the_chain_to_v4() {
    let dir = TempDir::new().unwrap();
    let path = make_v1(&dir);
    assert_eq!(Store::open(&path).unwrap().format_version().unwrap(), "4");
}

#[test]
fn failing_3_to_4_leaves_store_at_v3_and_readable() {
    let dir = TempDir::new().unwrap();
    let path = make_v3(&dir);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE INDEX idx_facts_subject ON items(source);").unwrap();
    drop(conn);
    let err = Store::open(&path).unwrap_err();
    assert!(matches!(err, Error::Migration { ref from, to: "4", .. } if from == "3"), "{err:?}");
    let conn = Connection::open(&path).unwrap();
    let v: String = conn.query_row("SELECT value FROM singularmem_meta WHERE key='format_version'", [], |r| r.get(0)).unwrap();
    assert_eq!(v, "3");
}

#[test]
fn read_only_v3_refuses_to_migrate() {
    let dir = TempDir::new().unwrap();
    let path = make_v3(&dir);
    assert!(matches!(Store::open_with_options(&path, StoreOptions { read_only: true }), Err(Error::Migration { .. })));
}
```

Existing tests asserting `"3"` after migration become `"4"`; `newer_store_still_refused` probes `'5'`; the already-migrated race test builds a v3 fixture with the 3→4 statements pre-applied.

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Ids**

`crates/singularmem-core/src/id.rs`:

```rust
//! ULID-backed identifier newtypes. Each is a distinct type so an item id
//! cannot be passed where a fact id is expected.

/// Define a ULID newtype with `Display` (uppercase), `FromStr`
/// (case-insensitive Crockford base32), and transparent serde.
macro_rules! ulid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(ulid::Ulid);

        impl $name {
            /// Wrap a raw `Ulid`. Crate-internal — the store mints ids.
            #[must_use]
            pub(crate) const fn from_ulid(u: ulid::Ulid) -> Self { Self(u) }
            /// Underlying ULID.
            #[must_use]
            pub const fn as_ulid(&self) -> ulid::Ulid { self.0 }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { std::fmt::Display::fmt(&self.0, f) }
        }
        impl std::str::FromStr for $name {
            type Err = ulid::DecodeError;
            fn from_str(s: &str) -> Result<Self, Self::Err> { ulid::Ulid::from_string(s).map(Self) }
        }
    };
}
pub(crate) use ulid_id;

ulid_id!(/// Stable identifier of a graph entity.
    EntityId);
ulid_id!(/// Stable identifier of one fact revision.
    FactId);
```

Replace `ItemId`'s hand-written struct/impls in `item.rs` with `crate::id::ulid_id!(/// … ItemId);` keeping its doc comment and the existing `item_id_orders_by_ulid_bytes` test. `lib.rs`: `pub mod id; pub mod graph; pub use id::{EntityId, FactId};`. In `ingest.rs` rename `mint_ulid` to `pub(crate) fn mint_raw_ulid(store: &Store, now: Timestamp) -> Result<Ulid>` and adapt the three callers (`ItemId::from_ulid(mint_raw_ulid(..)?)`).

- [ ] **Step 4: Graph types, normalisation, time**

`graph/mod.rs`: `pub mod normalise; pub mod time; pub mod types; pub use types::*;` (write/read modules arrive in Task 3).

`graph/types.rs` — exactly the spec's types with serde derives:

```rust
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use crate::{EntityId, FactId, ItemId, ScopeFilter};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity { pub id: EntityId, pub name: String, pub normalised_name: String, pub kind: Option<String>, pub scope: Option<String>, pub created_at: Timestamp }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRef { pub id: EntityId, pub name: String }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactObject { Entity(EntityRef), Value(String) }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub id: FactId, pub subject: EntityRef, pub predicate: String, pub object: FactObject,
    pub valid_from: Option<Timestamp>, pub valid_to: Option<Timestamp>, pub confidence: f32,
    pub source_item_id: Option<ItemId>, pub scope: Option<String>, pub supersedes: Option<FactId>, pub recorded_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewObject { Entity { name: String, kind: Option<String> }, Value(String) }

#[derive(Debug, Clone, PartialEq)]
pub struct NewFact {
    pub subject: String, pub subject_kind: Option<String>, pub predicate: String, pub object: NewObject,
    pub valid_from: Option<Timestamp>, pub valid_to: Option<Timestamp>, pub confidence: f32,
    pub source_item_id: Option<ItemId>, pub scope: Option<String>,
}
impl NewFact {
    /// Entity-object fact with confidence 1.0 and no window.
    #[must_use]
    pub fn triple(subject: &str, predicate: &str, object: &str) -> Self {
        Self { subject: subject.into(), subject_kind: None, predicate: predicate.into(), object: NewObject::Entity { name: object.into(), kind: None }, valid_from: None, valid_to: None, confidence: 1.0, source_item_id: None, scope: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction { Outgoing, Incoming, #[default] Both }

#[derive(Debug, Clone, Default)]
pub struct GraphQuery { pub scope: Option<ScopeFilter>, pub as_of: Option<Timestamp>, pub recorded_at: Option<Timestamp>, pub direction: Direction }

#[derive(Debug, Clone, Serialize)]
pub struct TimelineEntry { #[serde(flatten)] pub fact: Fact, pub current: bool }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct GraphStats { pub entities: usize, pub open_facts: usize, pub closed_facts: usize, pub predicates: usize }

#[derive(Debug, Clone, Serialize)]
pub struct EntitySummary { #[serde(flatten)] pub entity: Entity, pub fact_count: usize }
```

`graph/normalise.rs` (add `unicode-normalization = "0.1"` to core's `[dependencies]`):

```rust
//! Entity and predicate name normalisation (spec § "Normalisation").
use unicode_normalization::UnicodeNormalization;
use crate::error::{Error, Result};

pub const MAX_ENTITY_BYTES: usize = 256;
pub const MAX_PREDICATE_BYTES: usize = 64;

fn base(raw: &str) -> String {
    let nfc: String = raw.nfc().collect();
    let lowered = nfc.trim().to_lowercase().replace('\'', "");
    lowered.split_whitespace().collect::<Vec<_>>().join("_")
}

/// Normalise an entity name.
///
/// # Errors
/// `Validation { field: "entity" }` when empty or over [`MAX_ENTITY_BYTES`].
pub fn entity_name(raw: &str) -> Result<String> {
    let n = base(raw);
    if n.is_empty() { return Err(Error::Validation { field: "entity", reason: "must be non-empty".into() }); }
    if n.len() > MAX_ENTITY_BYTES { return Err(Error::Validation { field: "entity", reason: format!("exceeds {MAX_ENTITY_BYTES} bytes after normalisation") }); }
    Ok(n)
}

/// Normalise a predicate.
///
/// # Errors
/// `Validation { field: "predicate" }` when empty, over [`MAX_PREDICATE_BYTES`], or outside `[a-z0-9_]`.
pub fn predicate(raw: &str) -> Result<String> {
    let n = base(raw);
    if n.is_empty() { return Err(Error::Validation { field: "predicate", reason: "must be non-empty".into() }); }
    if n.len() > MAX_PREDICATE_BYTES { return Err(Error::Validation { field: "predicate", reason: format!("exceeds {MAX_PREDICATE_BYTES} bytes") }); }
    if !n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
        return Err(Error::Validation { field: "predicate", reason: "must match [a-z0-9_]+ after normalisation".into() });
    }
    Ok(n)
}
```

`graph/time.rs`:

```rust
//! Parse user-supplied time points: `YYYY-MM-DD` (midnight UTC) or RFC 3339.
use jiff::Timestamp;
use crate::error::{Error, Result};

/// # Errors
/// `Validation { field: "timestamp" }` when neither form parses.
pub fn parse_point(raw: &str) -> Result<Timestamp> {
    let s = raw.trim();
    if let Ok(t) = s.parse::<Timestamp>() { return Ok(t); }
    if let Ok(d) = s.parse::<jiff::civil::Date>() {
        return d.to_zoned(jiff::tz::TimeZone::UTC).map(|z| z.timestamp()).map_err(|e| Error::Validation { field: "timestamp", reason: e.to_string() });
    }
    Err(Error::Validation { field: "timestamp", reason: format!("{s:?} is neither YYYY-MM-DD nor RFC 3339") })
}
```

- [ ] **Step 5: Format v4**

`format.rs`: `FORMAT_VERSION = "4"` (leave `EXPORT_FORMAT` for Task 4), docs → `store-v4.md`. `schema.rs`: `DDL_V4` = `DDL_V3` + the two tables and indexes from the spec (verbatim DDL in the spec § "Store format v4"), `MIGRATE_3_TO_4_DDL` = the same statements minus the meta update, `pub fn migrate_3_to_4(conn) -> Result<()> { run_migration(conn, "3", "4", MIGRATE_3_TO_4_DDL) }`; `apply_current` runs `DDL_V4`. `store.rs`: chain `"1" → 1→2, 2→3, 3→4`; `"2" → 2→3, 3→4`; `"3" → 3→4`; read-only refuses `"1" | "2" | "3"`.

- [ ] **Step 6: Run, lint, commit**

`cargo test -p singularmem-core`, workspace tests, clippy, fmt.

```bash
git add crates/singularmem-core Cargo.lock
git commit -s -m "feat(core): graph types, normalisation, entity/fact ids, store format v4"
```

---

### Task 3: Graph operations on `Store`

**Files:**
- Create: `crates/singularmem-core/src/graph/write.rs`, `graph/read.rs`; Modify: `graph/mod.rs`
- Create: `crates/singularmem-core/tests/graph_ops.rs`

**Interfaces:**
- Produces: the `impl Store` methods from the spec: `add_fact`, `invalidate_fact`, `supersede_fact`, `query_entity`, `query_predicate`, `timeline`, `graph_stats`, `entities`, `fact_history`, plus `get_entity(name, scope) -> Result<Option<Entity>>`, `get_fact(id) -> Result<Fact>`.

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** Task 2
**Blocks:** Tasks 4, 5, 6

- [ ] **Step 1: Failing tests** — `tests/graph_ops.rs` (uses a `FixedClock` like the retrieve crate's wakeup tests; look at `crates/singularmem-core/src/clock.rs` and reuse/replicate a constant-time `Clock`):

```rust
use jiff::Timestamp;
use singularmem_core::graph::{Direction, GraphQuery, NewFact, NewObject};
use singularmem_core::graph::time::parse_point;
use singularmem_core::{Clock, Error, NewItem, OsRng, ScopeFilter, Store};
use tempfile::TempDir;

struct FixedClock(Timestamp);
impl Clock for FixedClock { fn now(&self) -> Timestamp { self.0 } }
fn ts(s: &str) -> Timestamp { parse_point(s).unwrap() }
fn store() -> (TempDir, Store) { let d = TempDir::new().unwrap(); let s = Store::open(d.path().join("s.db")).unwrap(); (d, s) }
fn store_at(d: &TempDir, at: &str) -> Store { Store::open_with(d.path().join("s.db"), Box::new(FixedClock(ts(at))), Box::new(OsRng)).unwrap() }

#[test]
fn add_creates_entities_once_and_is_idempotent() {
    let (_d, s) = store();
    let a = s.add_fact(NewFact::triple("Singularmem", "uses", "Tantivy")).unwrap();
    let b = s.add_fact(NewFact::triple("singularmem", "Uses", "tantivy")).unwrap();
    assert_eq!(a.id, b.id, "identical open fact is a no-op");
    assert_eq!(a.subject.name, "Singularmem", "display name is the first spelling");
    assert_eq!(s.entities(None, None).unwrap().len(), 2);
    assert_eq!(s.graph_stats(None).unwrap().open_facts, 1);
}

#[test]
fn value_objects_and_kinds() {
    let (_d, s) = store();
    let mut f = NewFact::triple("singularmem", "written_in", "rust");
    f.object = NewObject::Value("Rust 1.80".into());
    f.subject_kind = Some("project".into());
    let stored = s.add_fact(f).unwrap();
    assert!(matches!(stored.object, singularmem_core::graph::FactObject::Value(ref v) if v == "Rust 1.80"));
    assert_eq!(s.get_entity("singularmem", None).unwrap().unwrap().kind.as_deref(), Some("project"));
    let mut g = NewFact::triple("singularmem", "has_author", "jonas");
    g.subject_kind = Some("person".into());
    assert!(matches!(s.add_fact(g), Err(Error::Validation { field: "kind", .. })), "kind is immutable");
    let mut h = NewFact::triple("singularmem", "has_author", "jonas");
    h.subject_kind = None;
    s.add_fact(h).unwrap();
}

#[test]
fn invalidate_appends_a_revision_and_never_mutates() {
    let d = TempDir::new().unwrap();
    let s = store_at(&d, "2026-06-01");
    let f = s.add_fact(NewFact::triple("singularmem", "uses", "tantivy")).unwrap();
    let closed = s.invalidate_fact("singularmem", "uses", &NewObject::Entity { name: "tantivy".into(), kind: None }, None, Some(ts("2026-09-01"))).unwrap();
    assert_ne!(closed.id, f.id);
    assert_eq!(closed.supersedes, Some(f.id));
    assert_eq!(closed.valid_to, Some(ts("2026-09-01")));
    let original = s.get_fact(f.id).unwrap();
    assert_eq!(original.valid_to, None, "the original row is untouched");
    let hist = s.fact_history(closed.id).unwrap();
    assert_eq!(hist.iter().map(|x| x.id).collect::<Vec<_>>(), vec![f.id, closed.id]);
    assert!(matches!(s.invalidate_fact("singularmem", "uses", &NewObject::Entity { name: "tantivy".into(), kind: None }, None, None), Err(Error::NotFound { .. })));
}

#[test]
fn as_of_and_recorded_at_answer_both_axes() {
    let d = TempDir::new().unwrap();
    let s1 = store_at(&d, "2026-06-01T00:00:00Z");
    s1.add_fact(NewFact::triple("singularmem", "uses", "tantivy")).unwrap();
    drop(s1);
    let s2 = store_at(&d, "2026-09-01T00:00:00Z");
    let (old, new) = s2.supersede_fact("singularmem", "uses", &NewObject::Entity { name: "tantivy".into(), kind: None }, NewObject::Entity { name: "meilisearch".into(), kind: None }, None, Some(ts("2026-09-01"))).unwrap();
    assert!(old.is_some());
    assert_eq!(new.valid_from, Some(ts("2026-09-01")));
    let names = |facts: Vec<singularmem_core::graph::Fact>| facts.into_iter().map(|f| match f.object { singularmem_core::graph::FactObject::Entity(e) => e.name, singularmem_core::graph::FactObject::Value(v) => v }).collect::<Vec<_>>();
    let q = |as_of: Option<&str>, rec: Option<&str>| GraphQuery { as_of: as_of.map(ts), recorded_at: rec.map(ts), direction: Direction::Outgoing, scope: None };
    assert_eq!(names(s2.query_entity("singularmem", &q(None, None)).unwrap()), vec!["meilisearch"], "open facts only by default");
    assert_eq!(names(s2.query_entity("singularmem", &q(Some("2026-08-01"), None)).unwrap()), vec!["tantivy"]);
    assert_eq!(names(s2.query_entity("singularmem", &q(Some("2026-09-01"), None)).unwrap()), vec!["meilisearch"], "half-open: valid_to excluded, valid_from included");
    assert_eq!(names(s2.query_entity("singularmem", &q(None, Some("2026-07-01"))).unwrap()), vec!["tantivy"], "what we believed before the supersede");
    assert!(s2.query_entity("singularmem", &q(Some("2025-01-01"), None)).unwrap().is_empty());
}

#[test]
fn supersede_is_atomic_and_tolerates_missing_old() {
    let (_d, s) = store();
    s.add_fact(NewFact::triple("a", "p", "old")).unwrap();
    let bad_new = NewObject::Entity { name: "   ".into(), kind: None };
    assert!(s.supersede_fact("a", "p", &NewObject::Entity { name: "old".into(), kind: None }, bad_new, None, None).is_err());
    assert_eq!(s.graph_stats(None).unwrap().open_facts, 1, "old fact still open after a failed supersede");
    let (old, new) = s.supersede_fact("b", "p", &NewObject::Entity { name: "nothing".into(), kind: None }, NewObject::Entity { name: "new".into(), kind: None }, None, None).unwrap();
    assert!(old.is_none());
    assert_eq!(new.subject.name, "b");
}

#[test]
fn directions_scopes_predicates_timeline_entities() {
    let (_d, s) = store();
    let mut f = NewFact::triple("jonas", "owns", "singularmem"); f.scope = Some("claude-code/singularmem".into()); s.add_fact(f).unwrap();
    let mut g = NewFact::triple("singularmem", "uses", "tantivy"); g.scope = Some("claude-code/singularmem".into()); g.valid_from = Some(ts("2026-05-16")); s.add_fact(g).unwrap();
    let mut h = NewFact::triple("other", "uses", "tantivy"); h.scope = Some("claude-code/other".into()); s.add_fact(h).unwrap();
    let q = |dir| GraphQuery { direction: dir, ..Default::default() };
    assert_eq!(s.query_entity("singularmem", &q(Direction::Outgoing)).unwrap().len(), 1);
    assert_eq!(s.query_entity("singularmem", &q(Direction::Incoming)).unwrap().len(), 1);
    assert_eq!(s.query_entity("singularmem", &q(Direction::Both)).unwrap().len(), 2);
    let scoped = GraphQuery { scope: Some(ScopeFilter::descendants("claude-code/singularmem").unwrap()), ..Default::default() };
    assert_eq!(s.query_predicate("uses", &scoped).unwrap().len(), 1);
    assert_eq!(s.query_predicate("uses", &GraphQuery::default()).unwrap().len(), 2);
    let tl = s.timeline(Some("tantivy"), None).unwrap();
    assert_eq!(tl.len(), 2);
    assert!(tl.iter().all(|e| e.current));
    assert_eq!(tl[0].fact.valid_from, Some(ts("2026-05-16")), "dated first, NULL valid_from last");
    let ents = s.entities(None, None).unwrap();
    assert_eq!(ents.iter().map(|e| e.entity.name.as_str()).collect::<Vec<_>>(), vec!["jonas", "other", "singularmem", "tantivy"]);
    assert_eq!(ents.iter().find(|e| e.entity.name == "tantivy").unwrap().fact_count, 2);
    let st = s.graph_stats(None).unwrap();
    assert_eq!((st.entities, st.open_facts, st.closed_facts, st.predicates), (4, 3, 0, 2));
}

#[test]
fn provenance_and_read_only() {
    let d = TempDir::new().unwrap();
    let p = d.path().join("s.db");
    let item = { let s = Store::open(&p).unwrap(); s.ingest(NewItem::text("we picked tantivy")).unwrap() };
    let s = Store::open(&p).unwrap();
    let mut f = NewFact::triple("singularmem", "uses", "tantivy"); f.source_item_id = Some(item.id);
    assert_eq!(s.add_fact(f).unwrap().source_item_id, Some(item.id));
    let mut g = NewFact::triple("x", "y", "z"); g.source_item_id = Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap());
    assert!(matches!(s.add_fact(g), Err(Error::Validation { field: "source_item_id", .. })));
    let mut bad = NewFact::triple("x", "y", "z"); bad.valid_from = Some(ts("2026-09-01")); bad.valid_to = Some(ts("2026-08-01"));
    assert!(matches!(s.add_fact(bad), Err(Error::Validation { field: "valid_window", .. })));
    let ro = Store::open_with_options(&p, singularmem_core::StoreOptions { read_only: true }).unwrap();
    assert!(matches!(ro.add_fact(NewFact::triple("q", "r", "s")), Err(Error::ReadOnly { .. })));
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement `graph/write.rs`**

Key pieces (all inside `impl Store`, using `self.conn` lock, `self.clock`, `mint_raw_ulid`, `validate` helpers from Task 2):

```rust
/// Resolve or create an entity within a transaction. Returns (id, display name).
fn get_or_create_entity(&self, tx: &rusqlite::Transaction<'_>, now: Timestamp, name: &str, kind: Option<&str>, scope: Option<&str>) -> Result<EntityRef> {
    let norm = normalise::entity_name(name)?;
    let existing: Option<(String, String, Option<String>)> = tx
        .query_row("SELECT id, name, kind FROM entities WHERE normalised_name = ?1 AND IFNULL(scope,'') = IFNULL(?2,'')", params![norm, scope], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional().map_err(|e| Error::Sqlite { context: "looking up entity", source: e })?;
    if let Some((id, display, existing_kind)) = existing {
        if let (Some(k), Some(e)) = (kind, existing_kind.as_deref()) {
            if k != e { return Err(Error::Validation { field: "kind", reason: format!("entity {display:?} already has kind {e:?}") }); }
        }
        if kind.is_some() && existing_kind.is_none() {
            tx.execute("UPDATE entities SET kind = ?1 WHERE id = ?2", params![kind, id]).map_err(|e| Error::Sqlite { context: "setting entity kind", source: e })?;
        }
        return Ok(EntityRef { id: id.parse()?, name: display });
    }
    let id = EntityId::from_ulid(crate::ingest::mint_raw_ulid(self, now)?);
    tx.execute("INSERT INTO entities (id, name, normalised_name, kind, scope, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id.to_string(), name.trim(), norm, kind, scope, now.to_string()])
        .map_err(|e| Error::Sqlite { context: "inserting entity", source: e })?;
    Ok(EntityRef { id, name: name.trim().to_string() })
}
```

(`rusqlite::OptionalExtension` gives `.optional()`.) Note "kind is immutable once set": setting a kind on an entity that has none is allowed (first assignment), conflicting kinds error.

`add_fact`: `assert_writable`; validate predicate, confidence (`0.0..=1.0` else `Validation{"confidence"}`), window (`valid_to >= valid_from` else `Validation{"valid_window"}`), scope (`scope::validate`), `source_item_id` exists (`SELECT 1 FROM items WHERE id=?` else `Validation{"source_item_id"}`); begin tx; subject + object entities via `get_or_create_entity`; idempotency check:

```sql
SELECT f.id FROM facts f
WHERE f.subject_id = ?1 AND f.predicate = ?2
  AND IFNULL(f.object_id,'') = IFNULL(?3,'') AND IFNULL(f.object_value,'') = IFNULL(?4,'')
  AND IFNULL(f.scope,'') = IFNULL(?5,'')
  AND f.valid_to IS NULL
  AND NOT EXISTS (SELECT 1 FROM facts g WHERE g.supersedes = f.id)
LIMIT 1
```

if found → commit nothing new, return `get_fact(id)`; else insert the row (`recorded_at = now`), commit, return it.

`invalidate_fact(subject, predicate, object, scope, at)`: writable; find the open head by the same query (entity ids resolved WITHOUT creating — use a read-only lookup; missing entity → `NotFound { id: ... }`? `NotFound` carries an `ItemId`; add a new error variant `Error::FactNotFound { subject: String, predicate: String }` to `error.rs` rather than abusing `NotFound`, and update the spec's error table to name it); `at` default `self.clock.now()`; `at < valid_from` → `Validation{"valid_window"}`; insert a copy of the head with new id, `valid_to = at`, `supersedes = head.id`, `recorded_at = now`; return it.

`supersede_fact(...)`: one transaction: try the invalidate (on `FactNotFound` → `None` + `tracing::warn!`), then insert the new fact via the same code path as `add_fact` but with `valid_from = at` (refactor `add_fact` into `add_fact_in_tx(&self, tx, now, NewFact) -> Result<Fact>` so both share it). Any error rolls back everything.

- [ ] **Step 4: Implement `graph/read.rs`**

Shared SQL: a `fn fact_where(q: &GraphQuery) -> (String, Vec<String>)` producing the head/as-of/recorded-at predicate:

```sql
-- head rows (default): NOT EXISTS (SELECT 1 FROM facts g WHERE g.supersedes = f.id)
-- recorded_at R:      f.recorded_at <= ?R AND NOT EXISTS (SELECT 1 FROM facts g WHERE g.supersedes = f.id AND g.recorded_at <= ?R)
-- as_of T:            (f.valid_from IS NULL OR f.valid_from <= ?T) AND (f.valid_to IS NULL OR ?T < f.valid_to)
-- default (no as_of): f.valid_to IS NULL
-- scope:              ScopeFilter::sql_clause() with "scope" → "f.scope"
```

`load_fact(conn, row)` joins `entities s ON f.subject_id = s.id LEFT JOIN entities o ON f.object_id = o.id` and builds `Fact` (object = entity ref when `object_id` is set, else value). `query_entity`: resolve entity by normalised name (+ scope filter applies to facts, not the entity lookup — entities are looked up by name across scopes and the fact filter narrows); direction → `f.subject_id = ?` / `f.object_id = ?` / OR of both; order `recorded_at ASC`. `query_predicate`: `f.predicate = ?`. `timeline`: heads matching entity (either side) or all, `ORDER BY f.valid_from IS NULL, f.valid_from ASC, f.recorded_at ASC LIMIT 500`, `current = valid_to IS NULL`. `graph_stats`: counts over heads (+ scope). `entities`: `SELECT e.*, (SELECT COUNT(*) FROM facts f WHERE (f.subject_id = e.id OR f.object_id = e.id) AND <head>) ...` filtered by kind/scope, `ORDER BY name`. `fact_history(id)`: walk `supersedes` backwards to the root then forward: simplest is collect the chain by repeatedly following `supersedes` down from `id`, then any rows that supersede `id` upward (there is at most one per revision), return oldest → newest. `get_fact`, `get_entity`.

`graph/mod.rs`: `mod read; mod write;` plus the existing modules.

- [ ] **Step 5: Run, lint, commit**

```bash
git add crates/singularmem-core
git commit -s -m "feat(core): temporal fact graph — add, invalidate, supersede, query, timeline, stats"
```

---

### Task 4: Export v2 and `store-v4.md`

**Files:**
- Modify: `crates/singularmem-core/src/export.rs`, `format.rs` (`EXPORT_FORMAT = "export-v2"`), `tests/format.rs`, `tests/migration.rs` (loader test)
- Create: `docs/formats/store-v4.md`; Modify: `docs/formats/store-v3.md` (superseded note), `README.md` status line

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** Task 3
**Blocks:** none

- [ ] **Step 1: Failing tests** — in `tests/format.rs` extend the round trip: after ingesting items, `add_fact` two facts (one value object, one with `source_item_id`), `invalidate` one; export; assert the meta line says `export-v2` and `"4"`; the line kinds in order are `meta`, `item`×N, `entity`×M, `fact`×K; each `fact` line parses into a struct with fields `id, subject, predicate, object, valid_from?, valid_to?, confidence, source_item_id?, scope?, supersedes?, recorded_at` where `object` is `{"entity": "<name>"}` or `{"value": "<text>"}`; a loader that skips unknown kinds still reads all items. In `tests/migration.rs`, `third_party_loader_reads_graph_from_migrated_store`: migrate a v3 fixture, add a fact via the API, drop the store, then with raw `rusqlite` run the documented head/as-of SQL and assert one row with the expected subject name.

- [ ] **Step 2: Implement** — `export.rs`: after items, iterate `entities(None, None)` and a new crate-private `all_facts_chronological()` (every revision, `recorded_at ASC`), writing `ExportEntity { _kind: "entity", ... }` and `ExportFact { _kind: "fact", ..., object: {"entity": name} | {"value": v} }` with `skip_serializing_if` on optional fields. Bump `EXPORT_FORMAT`.

- [ ] **Step 3: Docs** — `store-v4.md` = `store-v3.md` + "Graph tables" section (DDL verbatim, column semantics, normalisation rule, revision/axes rules with the exact SQL for head, as-of, recorded-at), "Migration 3 → 4", export section rewritten for `export-v2` (new kinds, the ignore-unknown-kinds rule, order), loader walkthrough extended. `store-v3.md` gets the superseded banner. README status mentions the graph.

- [ ] **Step 4: Run, lint, commit**

```bash
git add crates/singularmem-core docs/formats README.md
git commit -s -m "feat(core): export-v2 with entity and fact lines; store-v4 format doc"
```

---

### Task 5: CLI `graph` verbs

**Files:**
- Create: `src/commands/graph.rs`; Modify: `src/commands/mod.rs` (`Command::Graph`), `src/main.rs` (dispatch + `FactNotFound` → exit 2), `tests/cli.rs`, `tests/snapshots/help/` (add `graph*` snapshots via `UPDATE_HELP_SNAPSHOTS=1` and extend `SUBCOMMANDS`)

**Assigned skill:** `rust-best-practices`, `test-driven-development`
**Blocked-by:** Tasks 1, 3
**Blocks:** none

- [ ] **Step 1: Failing CLI tests** (append to `tests/cli.rs`):

```rust
#[test]
fn graph_add_query_supersede_history_and_axes() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    let item = String::from_utf8(singularmem().args(["--store", db_s, "ingest", "--content", "we picked tantivy"]).assert().success().get_output().stdout.clone()).unwrap().trim().to_string();

    let fact = String::from_utf8(singularmem().args(["--store", db_s, "graph", "add", "Singularmem", "uses", "Tantivy", "--source", &item, "--scope", "claude-code/singularmem", "--from", "2026-05-16"]).assert().success().get_output().stdout.clone()).unwrap().trim().to_string();
    assert_eq!(fact.len(), 26);
    singularmem().args(["--store", db_s, "graph", "add", "singularmem", "uses", "tantivy", "--scope", "claude-code/singularmem"]).assert().success().stdout(format!("{fact}\n"));

    singularmem().args(["--store", db_s, "graph", "query", "singularmem", "--with-sources"]).assert().success()
        .stdout(predicate::str::contains("Singularmem —uses→ Tantivy")).stdout(predicate::str::contains("open")).stdout(predicate::str::contains("we picked tantivy"));

    singularmem().args(["--store", db_s, "graph", "supersede", "singularmem", "uses", "tantivy", "meilisearch", "--at", "2026-09-01", "--scope", "claude-code/singularmem"]).assert().success();
    singularmem().args(["--store", db_s, "graph", "query", "singularmem", "--as-of", "2026-08-01"]).assert().success().stdout(predicate::str::contains("Tantivy")).stdout(predicate::str::contains("meilisearch").not());
    singularmem().args(["--store", db_s, "graph", "query", "singularmem", "--as-of", "2026-09-02"]).assert().success().stdout(predicate::str::contains("meilisearch")).stdout(predicate::str::contains("Tantivy").not());
    singularmem().args(["--store", db_s, "graph", "query", "singularmem", "--recorded-at", "2020-01-01"]).assert().success().stdout("");

    let hist = String::from_utf8(singularmem().args(["--store", db_s, "graph", "history", &fact, "--format", "ids"]).assert().success().get_output().stdout.clone()).unwrap();
    assert_eq!(hist.lines().count(), 2);
    let tl = String::from_utf8(singularmem().args(["--store", db_s, "graph", "timeline", "singularmem", "--json"]).assert().success().get_output().stdout.clone()).unwrap();
    let v: serde_json::Value = serde_json::from_str(&tl).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    singularmem().args(["--store", db_s, "graph", "stats"]).assert().success().stdout(predicate::str::contains("open facts: 1")).stdout(predicate::str::contains("closed facts: 1"));
    singularmem().args(["--store", db_s, "graph", "entities", "--json"]).assert().success().stdout(predicate::str::contains("\"fact_count\""));
}

#[test]
fn graph_errors_and_read_only() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("store.db");
    let db_s = db.to_str().unwrap();
    singularmem().args(["--store", db_s, "graph", "add", "a", "bad-pred", "b"]).assert().code(1).stderr(predicate::str::contains("predicate"));
    singularmem().args(["--store", db_s, "graph", "invalidate", "a", "p", "b"]).assert().code(2).stderr(predicate::str::contains("no open fact"));
    singularmem().args(["--store", db_s, "graph", "add", "a", "p", "b", "--confidence", "1.5"]).assert().code(1);
    singularmem().args(["--store", db_s, "graph", "add", "a", "p", "b"]).assert().success();
    singularmem().args(["--store", db_s, "--read-only", "graph", "add", "a", "p", "c"]).assert().code(2).stderr(predicate::str::contains("read-only"));
    singularmem().args(["--store", db_s, "graph", "add", "a", "p", "Rust 1.80", "--value", "--json"]).assert().success().stdout(predicate::str::contains("\"value\":\"Rust 1.80\""));
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement** — `src/commands/graph.rs`:

```rust
#[derive(Args, Debug)]
pub struct GraphCommand { #[command(subcommand)] pub action: GraphAction }

#[derive(Subcommand, Debug)]
pub enum GraphAction {
    /// Record a fact: SUBJECT PREDICATE OBJECT (entities are created on demand).
    Add { subject: String, predicate: String, object: String,
        /// OBJECT is a literal value, not an entity.
        #[arg(long)] value: bool,
        #[arg(long)] subject_kind: Option<String>, #[arg(long)] object_kind: Option<String>,
        #[arg(long, value_name = "TS")] from: Option<String>, #[arg(long, value_name = "TS")] to: Option<String>,
        #[arg(long, default_value_t = 1.0)] confidence: f32,
        #[arg(long, value_name = "ITEM_ID")] source: Option<String>,
        #[arg(long, value_name = "PATH")] scope: Option<String>,
        #[arg(long)] json: bool },
    /// Facts about an entity.
    Query { entity: String, #[arg(long, value_enum, default_value_t = DirectionArg::Both)] direction: DirectionArg,
        #[arg(long, value_name = "TS")] as_of: Option<String>, #[arg(long, value_name = "TS")] recorded_at: Option<String>,
        #[command(flatten)] scope: ScopeArgs, #[arg(long)] with_sources: bool, #[arg(long)] json: bool },
    /// Facts with a predicate.
    Predicate { predicate: String, #[arg(long, value_name = "TS")] as_of: Option<String>, #[arg(long, value_name = "TS")] recorded_at: Option<String>, #[command(flatten)] scope: ScopeArgs, #[arg(long)] json: bool },
    /// Close an open fact (append-only: writes a new revision).
    Invalidate { subject: String, predicate: String, object: String, #[arg(long)] value: bool, #[arg(long, value_name = "TS")] at: Option<String>, #[arg(long, value_name = "PATH")] scope: Option<String> },
    /// Replace OLD with NEW at one instant, in one transaction.
    Supersede { subject: String, predicate: String, old: String, new: String, #[arg(long)] value: bool, #[arg(long, value_name = "TS")] at: Option<String>, #[arg(long, value_name = "PATH")] scope: Option<String> },
    /// Chronological facts, optionally for one entity.
    Timeline { entity: Option<String>, #[command(flatten)] scope: ScopeArgs, #[arg(long)] json: bool },
    /// Counts.
    Stats { #[command(flatten)] scope: ScopeArgs, #[arg(long)] json: bool },
    /// Entities with fact counts.
    Entities { #[arg(long)] kind: Option<String>, #[command(flatten)] scope: ScopeArgs, #[arg(long)] json: bool },
    /// All revisions of a fact, oldest first.
    History { fact_id: String, #[arg(long, value_enum, default_value_t = HistoryFormat::Table)] format: HistoryFormat },
}
```

`render_fact(&Fact) -> String` = `"{id}  {subject} —{predicate}→ {object}  [{from}, {to})  conf={:.2}  scope={}  src={}"` with `open` for a null `valid_to`, `?` for a null `valid_from`, `-` for absent scope/src; `--with-sources` appends `"    ↳ {first line of the source item content}"`. `stats` human output: `entities: N\nopen facts: N\nclosed facts: N\npredicates: N`. Add `Command::Graph(GraphCommand)` to the enum; writers (`Add|Invalidate|Supersede`) go in the read-only pre-check list and `needs_hook` is NOT extended (facts are not indexed). `main.rs` exit mapping: `CliError::Lib(Error::FactNotFound { .. })` → 2 with message `no open fact …` (from the variant's `Display`). Regenerate help snapshots (`UPDATE_HELP_SNAPSHOTS=1`) after adding `["graph"]`, `["graph","add"]`, … to `SUBCOMMANDS`.

- [ ] **Step 4: Run, lint, commit**

```bash
git add src tests
git commit -s -m "feat(cli): graph add/query/predicate/invalidate/supersede/timeline/stats/entities/history"
```

---

### Task 6: MCP `memory_graph_*` tools and docs

**Files:**
- Create: `crates/singularmem-mcp/src/tools/graph.rs`; Modify: `tools/mod.rs`, `server.rs`, `tests/mcp_handshake.rs`, `tests/read_only_mode.rs`, `crates/singularmem-mcp/README.md`, `docs/mcp-server.md`

**Assigned skill:** `mcp-builder`, `test-driven-development`
**Blocked-by:** Task 3
**Blocks:** none

- [ ] **Step 1: Failing tests** — unit tests in `tools/graph.rs` `mod tests` with a `seeded()` store: `add` returns text containing the fact id and `—uses→`; `query` by entity / by predicate / with `as_of`; `invalidate` then `query` empty; `supersede` then `as_of` before/after; `timeline` and `stats` text; `add` on a read-only config → `Error::ReadOnly`; `query` with both `entity` and `predicate` → `Validation`; wire tests: `tools/list` has 12 tools normally and 9 in read-only mode, `memory_graph_add` rejected read-only.

- [ ] **Step 2: Implement** — `MemoryGraph{Add,Query,Invalidate,Supersede,Timeline,Stats}Args` (`#[serde(default)]` optionals; `object_is_value: Option<bool>`; timestamps as strings parsed with `graph::time::parse_point`), `tool_descriptor()` per tool with JSON schemas mirroring the spec's table, `handle_*` returning text (`render_fact` duplicated minimally in the MCP crate — or move `render_fact` into `singularmem_core::graph::render` and use it from both; prefer the move), writers opened via the existing `open_store_with_hooks` pattern (hooks are irrelevant for facts; use a plain writable open helper `open_store_for_writing`), `Validation`/`FactNotFound`/`ReadOnly` → `invalid_params`. `list_tools`: push the three readers always, the three writers when `!read_only`. `memory_retrieve`'s description gains: "For current facts (who owns what, which tool is used), call `memory_graph_query` first." Docs: tool table in the README and `docs/mcp-server.md` (12 tools / 9 read-only).

- [ ] **Step 3: Run, lint, commit**

```bash
git add crates/singularmem-mcp docs/mcp-server.md
git commit -s -m "feat(mcp): memory_graph_* tools (add, query, invalidate, supersede, timeline, stats)"
```

Then the PR: base `main`, title `feat: temporal knowledge graph (sub-project 14)`; body lists format v4, export-v2, the CLI verbs, the six tools, the main.rs split, and the verification output.
