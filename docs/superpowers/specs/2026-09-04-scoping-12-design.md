---
title: Scoping (Sub-project 12)
date: 2026-09-04
status: draft
sub-project: 12-scoping
supersedes: none
---

# Scoping (Sub-project 12) — Design Spec

**Date:** 2026-09-04
**Status:** Draft (awaiting user review of written spec)
**Sub-project:** 12 (scoping — hierarchical scope path on every item, filters on every read surface)
**Builds on:** 11 (transcript ingestion, PR #21; store format v2, `singularmem-ingest`). Branch `scoping-12` is stacked on `transcript-ingestion-11` until #21 merges.
**Blocks:** 13 (session hooks + wake-up, which loads context by scope), 16 (MCP surface).

## Summary

Gives every memory item an optional **scope**: a `/`-separated path such
as `claude-code/singularmem` or `files/singularmem`. A scope filter on any
read surface (CLI, MCP, Node SDK, library) returns items in that scope
and all its descendants, or exactly that scope with `--scope-exact`.
Ingest verbs accept `--scope` and derive sensible defaults from their
source. Ships store format **v3**, a Tantivy schema bump with a
`scope_ancestors` field, a `scope` CLI group, and one new MCP tool. This
is mempalace's wings/rooms/drawers flattened into one path string.

## Problem & motivation

After sub-project 11 a store fills with every session and every file the
user has, and there is no way to ask "what do I know about *this*
project?" except by convention on tags or `source`. Tags cannot express
hierarchy and the `source` label is free-form. Wake-up (13) needs to load
context for the current project, and the graph (14) needs to know which
project an entity belongs to. Scope is the primitive both depend on.

## Goals & non-goals

### Goals

1. Every item MAY carry one scope path; the format is validated and
   normalised at ingest.
2. `list`, `search`, `retrieve` (CLI, MCP, Node, library) accept a scope
   filter with descendant-inclusive semantics by default and exact
   semantics on request. Unscoped queries behave exactly as today.
3. `ingest`, `ingest-transcript`, `ingest-dir` accept `--scope`; the two
   bulk verbs derive a default per item when the flag is absent.
4. A user can list scopes with counts and move a single item between
   scopes without creating a revision.
5. Store format v3 and the Tantivy schema change are documented; v1 and
   v2 stores migrate in place; a stale Tantivy sidecar fails with a
   message naming `singularmem reindex`.

### Non-goals

- Access control or per-scope encryption.
- Bulk rename of a scope subtree (one item at a time is enough for now;
  a `scope rename` verb is a follow-up if demand appears).
- Using scope for wake-up or hooks (13).
- Scope on the vector sidecar's own metadata; semantic hits are filtered
  through the store.

## Recommended approach

A real nullable `scope` column in SQLite (format v3) plus a multi-valued
`scope_ancestors` STRING field in Tantivy that holds every prefix of the
path, so "scope and descendants" is one exact term query. Semantic
results are post-filtered by loading the hit's scope from the store,
with the overfetch multiplier raised while a filter is active. The
filter is a typed value (`ScopeFilter`) threaded through every option
struct, so it is a contract in each layer rather than a convention.

### Approaches discarded

- **Tag convention (`scope:a/b`).** No prefix semantics with AND-tag
  filtering; scope would be a naming convention rather than a validated
  field. Rejected during brainstorming.
- **One store file per scope.** Zero code, but no cross-scope queries and
  no way to move an item; defeats a shared memory.

## Architecture

```
CLI / MCP / Node
   │  ScopeFilter { path, exact }
   ▼
singularmem-retrieve::Retriever ──► HybridSearcher (search)
   │                                   ├── Index (Tantivy): required clause on scope_ancestors
   │                                   └── EmbedderIndex: overfetch, then Store::scope_of(id) post-filter
   ▼
singularmem-core::Store
   ├── items.scope (v3)      list / list_by_tags accept ScopeFilter
   ├── scopes()              distinct scope + count
   └── set_scope(id, scope)  second sanctioned in-place mutation
singularmem-ingest
   └── Source::default_scope(&NewItem) -> Option<String>; driver fills scope when None
```

## Data model

### Scope string

- 1 to 8 segments separated by `/`; each segment 1 to 64 bytes matching
  `[A-Za-z0-9._-]+`; total ≤ 512 bytes.
- Normalised at validation: lowercased, no leading/trailing `/`, no
  empty segments, no `.` or `..` segments.
- `scope::validate(&str) -> Result<String>` returns the normalised form;
  `scope::ancestors("a/b/c") -> ["a", "a/b", "a/b/c"]`.

### Store format v3

Migration `2 → 3`, same shape as `1 → 2` (`BEGIN IMMEDIATE`, in-transaction
re-check, three statements):

```sql
ALTER TABLE items ADD COLUMN scope TEXT;
CREATE INDEX idx_items_scope ON items(scope) WHERE scope IS NOT NULL;
UPDATE singularmem_meta SET value = '3' WHERE key = 'format_version';
```

A v1 store runs `1 → 2` then `2 → 3` in sequence, each in its own
transaction; a failure in the second leaves the store at 2 (which is
still readable by the new binary). Read-only opens of a v1 or v2 store
refuse with `Error::Migration`. `FORMAT_VERSION = "3"`.

`Item`/`NewItem` gain `scope: Option<String>`. Export gains an optional
`scope` field per item line; `_singularmem_format` stays `export-v1`;
`store_format_version` reports `"3"`. `docs/formats/store-v3.md`
documents the column, the migration chain, the validation rule, and the
second in-place mutation (`set_scope`). `store-v2.md` gets a superseded
note.

Descendant query in SQL: `scope = ?1 OR scope LIKE ?1 || '/%'`. Exact:
`scope = ?1`.

### Tantivy schema (index format 0.3.0)

Two new fields: `scope` (STRING | STORED) and `scope_ancestors`
(STRING, indexed, multi-valued — one term per ancestor prefix, so a
document with scope `a/b/c` carries terms `a`, `a/b`, `a/b/c`). A
descendant filter is `TermQuery(scope_ancestors, path)`; an exact filter
is `TermQuery(scope, path)`. Both are added as `Occur::Must` clauses
around the existing query. Tantivy refuses to open an existing index
directory with a different schema; that error is mapped to
`singularmem_search::Error::IndexSchemaMismatch { path }` whose message
tells the user to run `singularmem reindex`. `EmbedderIndex` is
unchanged.

## Interfaces

### Library

```rust
// singularmem-core
pub mod scope { pub fn validate(s: &str) -> Result<String>; pub fn ancestors(s: &str) -> Vec<String>; }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeFilter { pub path: String, pub exact: bool }
impl Store {
    pub fn list_scoped(&self, filter: Option<&ScopeFilter>) -> Result<ItemIter<'_>>;
    pub fn list_by_tags_scoped(&self, tags: &[&str], filter: Option<&ScopeFilter>) -> Result<ItemIter<'_>>;
    pub fn scopes(&self) -> Result<Vec<(String, usize)>>;          // sorted by path
    pub fn scope_of(&self, id: ItemId) -> Result<Option<String>>;   // cheap point read
    pub fn set_scope(&self, id: ItemId, scope: Option<&str>) -> Result<Item>;
}
// list()/list_by_tags() keep their signatures and delegate with None.

// singularmem-search
pub struct SearchOptions { /* existing */ pub scope: Option<ScopeFilter> }
pub struct HybridSearchOptions { /* existing */ pub scope: Option<ScopeFilter> }
// HybridSearcher::search applies the lexical clause and, for semantic hits,
// asks a `ScopeLookup` (trait: fn scope_of(&self, id) -> Option<String>) that the
// caller provides; Store implements it. When a filter is active the semantic
// side fetches limit * fetch_multiplier * 2 candidates.

// singularmem-retrieve
pub struct RetrieveOptions { /* existing */ pub scope: Option<ScopeFilter> }

// singularmem-ingest
pub trait Source { /* existing */ fn default_scope(&self, item: &NewItem) -> Option<String> { None } }
// ingest_source: if item.scope.is_none() { item.scope = source.default_scope(&item) }
```

Defaults: `ClaudeTranscript::default_scope` = `claude-code/<basename of
metadata.cwd>` (None if cwd absent); `DirectoryWalker::default_scope` =
`files/<basename of canonical root>`. Both pass through
`scope::validate`; a basename that fails validation (e.g. non-ASCII)
yields `None` with a warn, never an error. `ClaudeTranscript` and
`DirectoryWalker` gain a `scope_override: Option<String>` field that the
CLI sets from `--scope`, which wins over the default.

### CLI

```
singularmem list      [--scope <PATH>] [--scope-exact]
singularmem search    [--scope <PATH>] [--scope-exact] ...
singularmem retrieve  [--scope <PATH>] [--scope-exact] ...
singularmem ingest            [--scope <PATH>]
singularmem ingest-transcript [--scope <PATH>]   # overrides claude-code/<cwd basename>
singularmem ingest-dir        [--scope <PATH>]   # overrides files/<root basename>
singularmem scope list                            # "<path>\t<count>" per line, sorted
singularmem scope move <ID> <PATH|->              # "-" clears the scope
```

`--scope-exact` without `--scope` is a usage error (exit 1). An invalid
scope string is a usage error (exit 1) before any store access. `get`
and `list --format table/jsonl/json` include the scope. Exit codes are
otherwise unchanged.

### Wire (MCP)

`memory_list`, `memory_retrieve`: new optional `scope` (string) and
`scope_exact` (bool, default false). `memory_ingest`: new optional
`scope`. New read tool `memory_scopes` (no args) returning `path` and
`count` per scope. All three schemas documented in
`crates/singularmem-mcp/README.md` and `docs/mcp-server.md`.

### Node binding

`scope?: string` on `Item`, `NewItem`, `ListOptions`, `SearchOptions`,
`RetrieveOptions`; `scopeExact?: boolean` on the three option types;
`Store.scopes(): Promise<Array<{ path: string; count: number }>>`;
`Store.setScope(id, scope | null): Promise<Item>`.

## Error handling

- Invalid scope string → `Error::Validation { field: "scope", reason }`;
  nothing touched.
- `set_scope` on an unknown id → `Error::NotFound`; on a read-only store
  → `Error::ReadOnly`.
- Migration failures leave the store at its prior version with the
  same `Error::Migration { from, to, reason }` contract as v2.
- Stale Tantivy sidecar → `IndexSchemaMismatch`, exit 2 from the CLI with
  the reindex hint; no automatic rebuild (Principle VII: no silent
  fallbacks).
- A semantic hit whose id is missing from the store (sidecar drift) is
  dropped from scoped results with a `tracing::warn!`, matching how the
  retriever already treats missing ids.

## Testing strategy

- **Unit (core):** `scope::validate` accepts/rejects the boundary cases
  (8 segments, 64-byte segment, 512 total, `..`, empty segment, uppercase
  normalisation); `ancestors` ordering.
- **Migration (core):** v2 fixture → v3; v1 fixture → v3 via the chain;
  a failing `2 → 3` (pre-existing `idx_items_scope`) leaves the store at
  2 and readable; read-only v2 refuses; export after migration.
- **Store:** exact vs descendant filtering, `scopes()` counts, `set_scope`
  round-trip and NULL clearing, `scope_of`.
- **Search:** Tantivy filter excludes a sibling (`a/b` vs `a/c`) and
  includes a grandchild; exact filter excludes the child; opening an
  index built with the previous schema returns `IndexSchemaMismatch`.
- **Hybrid:** with `MockEmbedder`, a semantic-only hit outside the scope
  is dropped and one inside survives.
- **Ingest:** both defaults, `--scope` override, invalid basename → None
  with warn.
- **CLI:** `ingest-transcript` on the fixture files items under
  `claude-code/proj`; `ingest-dir` under `files/<tmp basename>`; `list
  --scope files` finds them and `--scope-exact files` does not; `scope
  list` output; `scope move` then `get` shows the new scope; a stale
  sidecar gives exit 2 with the reindex hint.
- **MCP and Node:** the new argument on each tool/method, and
  `memory_scopes`/`scopes()`.
- All offline (`SINGULARMEM_TEST_EMBEDDER=mock`), deterministic.

## Open questions

None blocking. Whether `scope rename` (subtree) is wanted is left to
demand; `set_scope` covers single items.

## Acceptance criteria

1. `cargo test --workspace --all-targets` passes offline; clippy and fmt
   clean.
2. On a fresh store, `ingest-transcript` over the fixture yields items
   whose `scope` is `claude-code/proj`; `ingest-dir .` on this repo
   yields `files/singularmem`.
3. `search --scope claude-code "cargo"` returns transcript hits and
   `search --scope files "cargo"` returns file hits, with no overlap.
4. A v0.16.0 (v1) store and a v2 store both open under the new binary,
   report format version 3, and export cleanly.
5. Opening a Tantivy sidecar built before this change exits 2 and prints
   the reindex hint; after `singularmem reindex` scoped search works.
6. `docs/formats/store-v3.md` exists; a raw-`rusqlite` loader test reads
   scope from a migrated fixture.
7. `memory_scopes` appears in `tools/list`; `memory_list` with `scope`
   returns only scoped items.
8. Ingest throughput stays ≥ 50 items/s in `perf-budgets`.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | No network; all filtering local. |
| **II — Provider-Agnostic by Contract** | Scope is provider-neutral; the `claude-code/` prefix is a default the user can override, not a dependency. |
| **III — Open Core with a Stable Boundary** | Format v3 and the index schema are documented; the second in-place mutation (`set_scope`) is specified alongside the first. |
| **IV — CLI-First, GUI-Visible** | Every capability has a CLI verb or flag before any GUI. |
| **V — Composable Library Architecture** | `ScopeFilter` is a plain value; `ScopeLookup` is a trait so search does not depend on core's `Store`. |
| **VI — Deterministic and Offline-Testable** | Fixtures and `MockEmbedder`; no network. |
| **VII — Honest Failure Modes** | Stale sidecar fails loudly with the fix named; invalid scope rejected before any write; migration transactional. |
| **VIII — Privacy Telemetry Boundary** | No telemetry. |
| **IX — Accessible by Default** | CLI plain text. |
| **X — Performance Budgets, Enforced in CI** | One nullable column and one partial index; `ingest_many` statement shape unchanged. Scoped semantic search overfetches ×2, documented. |
