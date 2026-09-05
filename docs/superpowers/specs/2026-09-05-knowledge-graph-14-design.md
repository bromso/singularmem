---
title: Temporal knowledge graph (Sub-project 14)
date: 2026-09-05
status: draft
sub-project: 14-knowledge-graph
supersedes: none
---

# Temporal knowledge graph (Sub-project 14) — Design Spec

**Date:** 2026-09-05
**Status:** Draft (awaiting user review of written spec)
**Sub-project:** 14 (entities and temporal facts with validity windows; CLI `graph` verbs; six MCP tools; store format v4; `src/main.rs` split)
**Builds on:** 12 (scopes), 13 (`src/main.rs` shape, MCP server layout).
**Blocks:** 16 (MCP surface: Node/SDK exposure of the graph; wake-up facts section).

## Summary

Adds an agent-maintained knowledge graph to the store: **entities**
(`singularmem`, `jonas`, `tantivy`) and **facts** (`singularmem uses tantivy`,
valid from 2026-05-16) with validity windows, provenance to the memory item
a fact came from, and the same scope paths items use. Facts are
append-only: invalidating or superseding writes a new revision linked to
the old one, so "what was true on date X" and "what did we believe on date
Y" are both answerable. Operations mirror mempalace's six graph tools
(add, query, invalidate, supersede, timeline, stats) and add a CLI mempalace
lacks. Extraction is the agent's job; nothing here calls an LLM.

## Problem & motivation

Verbatim memory answers "what did we say"; it cannot answer "what is the
current database" or "who owned the release process last spring" without
re-reading transcripts. Mempalace's graph closes that gap and is the last
capability row still marked Missing after 11–13. The store already has the
primitives — ULIDs, supersedes chains, scopes, export — so a graph is a
second ledger, not a new system.

## Goals & non-goals

### Goals

1. `graph add` records a fact with optional validity window, confidence,
   source item, and scope; entities are created on demand.
2. `graph invalidate` and `graph supersede` change beliefs without mutating
   history; `graph query --as-of` and `--recorded-at` read either axis.
3. `graph timeline`, `graph stats`, `graph entities`, `graph predicate`
   give the agent and the user the overview views mempalace has.
4. Six MCP tools expose the same operations; writers respect `--read-only`.
5. Store format v4 and the export format are documented; v1–v3 stores
   migrate in place.
6. `src/main.rs` is split into `src/commands/` before the graph verbs land.

### Non-goals

- Automatic fact extraction (rule-based or LLM). The agent decides.
- Entity aliases, merging, or fuzzy matching; identity is the normalised
  name.
- Indexing fact text into Tantivy/USearch, or a facts section in
  `wake-up` (16 may add one).
- Node binding exposure (16/17).
- Graph visualisation (proprietary tier by constitution).
- Per-scope entity identity: entities are store-global (one `tantivy`
  everywhere); facts carry scope, so scoped queries still narrow results.

## Recommended approach

Two new tables in the existing SQLite store, shaped like `items`: ULID
ids, `scope`, append-only rows, supersedes chains, `recorded_at` minted by
the store clock. A `Graph` API on `Store` (module `graph.rs` in core) does
all reads and writes; the CLI and MCP are thin shells. Export gains two
line kinds under a bumped `export-v2` marker whose only new rule is that
loaders ignore unknown kinds.

### Approaches discarded

- **Separate graph database file** (mempalace's shape). Breaks single-file
  export and adds a second migration story for no gain.
- **Facts encoded as tagged items.** No schema change, but as-of queries
  and supersession become string parsing and the ledger semantics are
  lost.

## Architecture

```
CLI graph *  ─┐
MCP memory_graph_* ─┼──► singularmem_core::graph (Store methods)
                    │        ├── entities  (get-or-create by normalised name; store-global)
                    │        ├── facts     (add / invalidate / supersede / query / timeline / stats)
                    │        └── export    (entity + fact lines)
                    └──► scope::ScopeFilter (reused)
src/commands/{graph,ingest,search,scope,hooks,wakeup,store}.rs  ← Task 0 split
```

## Data model

### Normalisation

`entity::normalise(name) -> String`: NFC, trim, lowercase, internal
whitespace runs → `_`, strip `'`; must be 1–256 bytes after
normalisation, else `Validation { field: "entity" }`. `predicate::normalise`
is the same rule with the additional constraint `[a-z0-9_]+` after
normalisation (1–64 bytes), else `Validation { field: "predicate" }`.
Display keeps the original `name` as first written.

### Store format v4

Migration `3 → 4` (same runner as 1→2 and 2→3):

```sql
CREATE TABLE entities (
    id               TEXT PRIMARY KEY NOT NULL,
    name             TEXT NOT NULL,
    normalised_name  TEXT NOT NULL,
    kind             TEXT,
    created_at       TEXT NOT NULL,
    CHECK (length(name) > 0)
) STRICT;
CREATE UNIQUE INDEX idx_entities_identity ON entities(normalised_name);

CREATE TABLE facts (
    id              TEXT PRIMARY KEY NOT NULL,
    subject_id      TEXT NOT NULL,
    predicate       TEXT NOT NULL,
    object_id       TEXT,
    object_value    TEXT,
    valid_from      TEXT,
    valid_to        TEXT,
    confidence      REAL NOT NULL DEFAULT 1.0,
    source_item_id  TEXT,
    scope           TEXT,
    supersedes      TEXT,
    recorded_at     TEXT NOT NULL,
    FOREIGN KEY (subject_id) REFERENCES entities(id),
    FOREIGN KEY (object_id) REFERENCES entities(id),
    FOREIGN KEY (source_item_id) REFERENCES items(id) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (supersedes) REFERENCES facts(id) DEFERRABLE INITIALLY DEFERRED,
    CHECK ((object_id IS NULL) <> (object_value IS NULL)),
    CHECK (confidence >= 0.0 AND confidence <= 1.0),
    CHECK (valid_to IS NULL OR valid_from IS NULL OR valid_to >= valid_from)
) STRICT;
CREATE INDEX idx_facts_subject   ON facts(subject_id);
CREATE INDEX idx_facts_object    ON facts(object_id) WHERE object_id IS NOT NULL;
CREATE INDEX idx_facts_predicate ON facts(predicate);
CREATE INDEX idx_facts_supersedes ON facts(supersedes) WHERE supersedes IS NOT NULL;
CREATE INDEX idx_facts_scope     ON facts(scope) WHERE scope IS NOT NULL;
UPDATE singularmem_meta SET value = '4' WHERE key = 'format_version';
```

Timestamps are RFC 3339 UTC strings (`jiff::Timestamp` display), so
lexical comparison equals temporal comparison. `valid_from`/`valid_to`
accept a date (`2026-05-16`, expanded to `T00:00:00Z`) or a full
timestamp on input; stored canonical. `NULL valid_from` means "since
unknown"; `NULL valid_to` means "still valid" (open).

### Revisions and the two time axes

A fact's chain is its history; the **head** is the newest revision (no
other row supersedes it). Reads use heads unless `--recorded-at` is given.

- **Open fact:** head with `valid_to IS NULL`.
- **Valid at T (`--as-of T`):** head where `(valid_from IS NULL OR valid_from <= T) AND (valid_to IS NULL OR T < valid_to)` — half-open.
- **Believed at R (`--recorded-at R`):** for each chain, the newest
  revision with `recorded_at <= R`; then apply the as-of rule if also
  given, else the open rule. Chains whose first revision is after R are
  invisible.

### Operations (all on `Store`, module `graph`)

| Op | Semantics |
|---|---|
| `add_fact(NewFact)` | `get_or_create` subject (and object when an entity). If an **open head** with identical `(subject, predicate, object)` in the same scope exists, return it unchanged (idempotent). Else insert with `valid_from` (default `None`), `confidence` (default 1.0). Returns `Fact`. |
| `invalidate(subject, predicate, object, at)` | Find the open head; insert a revision copying it with `valid_to = at` and `supersedes = head.id`. `at` defaults to now; `at < valid_from` → `Validation`. No open head → `NotFound`. |
| `supersede(subject, predicate, old_object, new_object, at)` | One transaction: `invalidate(old, at)` (tolerated missing: warn, continue) then `add_fact(new, valid_from = at)`. Returns both facts. |
| `query_entity(name, scope, direction, as_of, recorded_at)` | Facts where the entity is subject (`outgoing`), object (`incoming`), or either (`both`). |
| `query_predicate(predicate, scope, as_of, recorded_at)` | All facts with that predicate. |
| `timeline(entity: Option, scope)` | Head revisions ordered by `valid_from` ascending (NULLs last), then `recorded_at`; each row flagged `current` (open). Cap 500. |
| `stats(scope)` | entities, open facts, closed facts, distinct predicates. |
| `entities(scope, kind)` | Entities sorted by name with fact counts. Entities are store-global; `scope` filters on **fact** scope, so it narrows both the counts and which entities appear. |
| `fact_history(id)` | The chain oldest → newest. |

Entities are store-global: the normalised name alone is their identity, so
`tantivy` in `claude-code/a` and `claude-code/b` is one node. Scope lives on
facts, which is what every scoped read filters on.

Entities are never deleted. Entity `kind` is free text, set on first
creation (`--subject-kind` / `--object-kind`) and immutable afterwards; a
differing kind on an existing entity is a `Validation { field: "kind" }`
error, and omitting the kind on later adds is fine.

### Export — `export-v2`

Marker `_singularmem_format: "export-v2"`. Line kinds: `meta`, `item`
(unchanged shape), `entity` (`{ "_kind":"entity", id, name, kind, created_at }`),
`fact` (`{ "_kind":"fact", id, subject, predicate, object: {entity: name} | {value: text}, valid_from, valid_to, confidence, source_item_id, scope, supersedes, recorded_at }`).
Order: meta, items, entities, facts (each `created_at`/`recorded_at`
ascending). Rule for loaders: ignore unknown `_kind`s. `store_format_version`
reports `"4"`.

## Interfaces

### Library (`singularmem-core`)

```rust
pub struct Entity { pub id: ItemId /* ULID newtype reused as EntityId alias */, pub name: String, pub normalised_name: String, pub kind: Option<String>, pub created_at: Timestamp }
pub enum FactObject { Entity { id: EntityId, name: String }, Value(String) }
pub struct Fact { pub id: FactId, pub subject: EntityRef, pub predicate: String, pub object: FactObject, pub valid_from: Option<Timestamp>, pub valid_to: Option<Timestamp>, pub confidence: f32, pub source_item_id: Option<ItemId>, pub scope: Option<String>, pub supersedes: Option<FactId>, pub recorded_at: Timestamp }
pub struct NewFact { pub subject: String, pub subject_kind: Option<String>, pub predicate: String, pub object: NewObject, pub valid_from: Option<Timestamp>, pub valid_to: Option<Timestamp>, pub confidence: f32, pub source_item_id: Option<ItemId>, pub scope: Option<String> }
pub enum NewObject { Entity { name: String, kind: Option<String> }, Value(String) }
pub enum Direction { Outgoing, Incoming, Both }
pub struct GraphQuery { pub scope: Option<ScopeFilter>, pub as_of: Option<Timestamp>, pub recorded_at: Option<Timestamp>, pub direction: Direction }
impl Store {
    pub fn add_fact(&self, f: NewFact) -> Result<Fact>;
    pub fn invalidate_fact(&self, subject: &str, predicate: &str, object: &NewObject, scope: Option<&str>, at: Option<Timestamp>) -> Result<Fact>;
    pub fn supersede_fact(&self, subject: &str, predicate: &str, old: &NewObject, new: NewObject, scope: Option<&str>, at: Option<Timestamp>) -> Result<(Option<Fact>, Fact)>;
    pub fn query_entity(&self, name: &str, q: &GraphQuery) -> Result<Vec<Fact>>;
    pub fn query_predicate(&self, predicate: &str, q: &GraphQuery) -> Result<Vec<Fact>>;
    pub fn timeline(&self, entity: Option<&str>, scope: Option<&ScopeFilter>) -> Result<Vec<TimelineEntry>>;   // Fact + current: bool
    pub fn graph_stats(&self, scope: Option<&ScopeFilter>) -> Result<GraphStats>;
    pub fn entities(&self, scope: Option<&ScopeFilter>, kind: Option<&str>) -> Result<Vec<EntitySummary>>;    // Entity + fact_count
    pub fn fact_history(&self, id: FactId) -> Result<Vec<Fact>>;
}
pub const FORMAT_VERSION: &str = "4";  pub const EXPORT_FORMAT: &str = "export-v2";
```

`EntityId`/`FactId` are ULID newtypes (share the `ItemId` implementation
via a small macro or type aliases; the point is they are not
interchangeable with item ids at the type level).

### CLI

```
singularmem graph add <SUBJECT> <PREDICATE> <OBJECT>
    [--value]                # OBJECT is a literal value, not an entity
    [--subject-kind K] [--object-kind K]
    [--from TS] [--to TS] [--confidence 0..1] [--source ITEM_ID] [--scope PATH]
    [--json]                 # prints the Fact; default prints the fact id
singularmem graph query <ENTITY> [--direction outgoing|incoming|both] [--as-of TS] [--recorded-at TS]
    [--scope PATH] [--scope-exact] [--with-sources] [--json]
singularmem graph predicate <PREDICATE> [--as-of TS] [--recorded-at TS] [--scope ...] [--json]
singularmem graph invalidate <SUBJECT> <PREDICATE> <OBJECT> [--value] [--at TS] [--scope PATH]
singularmem graph supersede  <SUBJECT> <PREDICATE> <OLD> <NEW> [--value] [--at TS] [--scope PATH]
singularmem graph timeline [ENTITY] [--scope ...] [--json]
singularmem graph stats [--scope ...] [--json]
singularmem graph entities [--kind K] [--scope ...] [--json]
singularmem graph history <FACT_ID> [--json]
```

Human output, one fact per line: `<fact id>  <subject> —<predicate>→ <object>  [<valid_from>, <valid_to>)  conf=0.9  scope=…  src=<item>` with `open` for a null `valid_to`. Exit codes: 0; 1 usage/validation; 2 `NotFound` or read-only; 3 unsupported format.

### Wire (MCP)

| Tool | Args | Read-only? |
|---|---|---|
| `memory_graph_add` | `subject`, `predicate`, `object`, `object_is_value?`, `subject_kind?`, `object_kind?`, `valid_from?`, `valid_to?`, `confidence?`, `source_item_id?`, `scope?` | writer |
| `memory_graph_query` | `entity?` XOR `predicate?`, `direction?`, `as_of?`, `recorded_at?`, `scope?`, `scope_exact?` | read |
| `memory_graph_invalidate` | `subject`, `predicate`, `object`, `object_is_value?`, `at?`, `scope?` | writer |
| `memory_graph_supersede` | `subject`, `predicate`, `old_object`, `new_object`, `object_is_value?`, `at?`, `scope?` | writer |
| `memory_graph_timeline` | `entity?`, `scope?`, `scope_exact?` | read |
| `memory_graph_stats` | `scope?` | read |

Text output mirrors the CLI human format; writers are omitted from
`tools/list` and rejected in read-only mode like `memory_ingest`. The
`memory_retrieve` tool description gains one sentence telling the model to
call `memory_graph_query` for current facts before answering.

## Error handling

- `Validation { field: "entity" | "predicate" | "confidence" | "valid_window" | "kind" }` for bad input; nothing written.
- `FactNotFound { subject, predicate, object }` (new `Error` variant, exit 2) for invalidate/supersede-old with no open head (supersede tolerates it and reports `old: null`).
- `FactIdNotFound { id }` (new `Error` variant, exit 2) when a fact **id** — `get_fact`, `fact_history` — is not in the store. Distinct from `FactNotFound`, which addresses a triple.
- `ReadOnly` for any writer on a read-only store; exit 2 from the CLI, `invalid_params` from MCP.
- `source_item_id` that does not exist → `SupersedesNotFound`-style `Validation { field: "source_item_id" }` (checked in the transaction).
- Migration 3→4 failure leaves the store at 3 (same runner and tests as before).

## Testing strategy

- **Unit:** normalisation (unicode, whitespace, apostrophes, length caps); timestamp parsing (date vs full); interval validation.
- **Migration:** v3 fixture → v4; v1 → v4 chain; failing 3→4 (pre-existing `idx_facts_subject`) leaves v3; read-only v3 refuses; raw-`rusqlite` loader reads an entity and a fact from a migrated store and runs the documented as-of SQL.
- **Store:** add idempotency; add creates entities once per scope; invalidate creates a revision and never mutates (assert the old row is byte-identical via raw SQL); supersede atomic (inject a failure on the second insert with an invalid `new` and assert the old fact is still open); as-of boundaries (`T == valid_from` included, `T == valid_to` excluded); recorded-at hides later revisions and later chains; direction filters; scope filtering; timeline order and `current` flag; stats; entities with counts; history.
- **Export:** round trip of items + entities + facts through `export-v2`; a v1-loader-style parse ignoring unknown kinds.
- **CLI:** every verb, `--json` shapes, exit codes, read-only refusal.
- **MCP:** each tool; writers hidden in read-only mode (wire tests).
- **Task 0:** the split is behaviour-neutral — the whole existing `tests/cli.rs` suite passes unchanged and `--help` output is byte-identical before/after (snapshot in a test).
- All offline.

## Open questions

None blocking. Whether `wake-up` should include a "current facts" section
is deferred to 16.

## Acceptance criteria

1. `cargo test --workspace --all-targets` offline; clippy and fmt clean; `src/main.rs` under 400 lines after Task 0 with `--help` byte-identical.
2. `graph add singularmem uses tantivy --source <id> --scope claude-code/singularmem` then `graph query singularmem` shows the fact as open with its source.
3. `graph supersede singularmem uses tantivy meilisearch --at 2026-09-01` then `graph query singularmem --as-of 2026-08-01` shows tantivy and `--as-of 2026-09-02` shows meilisearch; `graph history <old id>` shows two revisions; the original row is unchanged in SQLite.
4. `graph query singularmem --recorded-at <before the supersede>` shows tantivy open.
5. A v0.16.0 (v1) store opens, reports format version 4, and exports as `export-v2` with zero entity/fact lines.
6. `docs/formats/store-v4.md` exists; the raw loader test passes.
7. `tools/list` shows six `memory_graph_*` tools normally and three in read-only mode.

## Deviations recorded during implementation

1. **Entities are store-global** (human ruling, 2026-09-05). The original
   non-goal made the same normalised name in two scopes two entities. The
   `entities` table therefore has no `scope` column and its unique index is
   on `normalised_name` alone; `get_entity`, `find_entity`, and
   `get_or_create_entity` take no scope. `entities(scope, kind)` keeps its
   `scope` parameter, which filters on **fact** scope. The format was
   unreleased when this landed, so v4 was amended in place — there is no
   4 → 5 migration.
2. **As-of before a NULL `valid_from`.** A closed revision that inherited a
   `NULL valid_from` is valid at *any* instant before its `valid_to`,
   because the spec defines `NULL valid_from` as "since unknown" and the
   as-of rule as `(valid_from IS NULL OR valid_from <= T)`. This is
   spec-consistent; the implementation plan's test expected an empty result
   and was corrected, not the code.
3. **`AmbiguousFactRevision { candidates }`**, an error variant beyond the
   set this spec's § "Error handling" first listed. `fact_history`'s
   forward walk returns it when more than one revision supersedes the same
   one — a forked chain the library refuses to resolve by guessing
   (Principle VII). Reachable only from a hand-edited or externally-written
   store; the graph's own writes cannot fork a chain.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | All local; no extraction service. |
| **II — Provider-Agnostic by Contract** | Graph API is provider-neutral; MCP tools are the only agent surface and are editor-agnostic. |
| **III — Open Core with a Stable Boundary** | Format v4 + export-v2 documented and loader-tested; visualisation stays proprietary, the data is open. |
| **IV — CLI-First, GUI-Visible** | Every operation has a CLI verb before MCP. |
| **V — Composable Library Architecture** | `graph` is a core module with a typed API; CLI/MCP are shells; Task 0 restores "main.rs is dispatch only". |
| **VI — Deterministic and Offline-Testable** | Fixed-clock tests for recorded-at; no network. |
| **VII — Honest Failure Modes** | Append-only; validation before write; migration transactional; NotFound is explicit. |
| **VIII — Privacy Telemetry Boundary** | No telemetry. |
| **IX — Accessible by Default** | CLI plain text. |
| **X — Performance Budgets, Enforced in CI** | Items path untouched; graph reads are indexed point queries. |
