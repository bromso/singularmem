---
title: Transcript ingestion (Sub-project 11)
date: 2026-09-03
status: draft
sub-project: 11-transcript-ingestion
supersedes: none
---

# Transcript ingestion (Sub-project 11) — Design Spec

**Date:** 2026-09-03
**Status:** Draft (awaiting user review of written spec)
**Sub-project:** 11 (transcript ingestion — Claude Code JSONL sessions + source-tree mining)
**Builds on:** 1 (memory store v0), 2a/2b/2c (search), 4b (MCP ingest hook wiring).
**Blocks:** 12 (scoping), 13 (session hooks + wake-up), 15 (retrieval benchmark).

## Summary

Adds bulk, idempotent ingestion of two real-world sources into the store:
Claude Code conversation transcripts (`~/.claude/projects/**/*.jsonl`) and
source trees. Ships a new library crate `singularmem-ingest` with a `Source`
trait and two implementations, two new CLI verbs (`ingest-transcript`,
`ingest-dir`), and store format **v2**, which adds a nullable, unique
`external_id` column so re-running either verb ingests nothing already
present. This is the first step of the parity programme against
mempalace: without it, the store only fills one item at a time.

## Problem & motivation

Today the only way content enters a store is `singularmem ingest` with
`--content`, `--file`, or `--stdin`: one item per invocation, no parsing,
no chunking, no notion of "already ingested". A developer who wants their
past agent sessions searchable has no path at all. Every downstream
parity feature (scoping by project, session hooks, wake-up context,
recall benchmarks) needs a populated store, so this lands first.

## Goals & non-goals

### Goals

1. `singularmem ingest-transcript` turns every Claude Code session under a
   path into one verbatim item per user or assistant message.
2. `singularmem ingest-dir` turns a source tree into one item per text
   file, honouring `.gitignore`.
3. Both verbs are idempotent: a second run over unchanged input ingests
   zero items. A changed file ingests a new item that supersedes the old.
4. Long messages and files are chunked so each item fits the embedding
   window well enough to be found.
5. Ingested items are immediately searchable (Tantivy + USearch hooks
   auto-wired, exactly as `ingest` does today).
6. Store format v2 is documented and third-party readable; v1 stores
   migrate in place on open.

### Non-goals

- Codex CLI, Cursor, Gemini CLI transcript formats (future sub-project;
  the `Source` trait is the extension point).
- MCP or Node SDK exposure of the new verbs (13 calls the CLI; SDK
  exposure is additive later).
- Namespaces / project scoping (sub-project 12). `--project` here is a
  filter on the transcript's `cwd`, not a stored scope.
- Storing tool inputs, tool results, or thinking blocks (user decision:
  text only, tool names in metadata).
- Preserving the message timestamp as `created_at`. `created_at` stays
  the ingest time (ledger semantics); the original timestamp is kept in
  metadata as `occurred_at`.

## Recommended approach

A new crate `singularmem-ingest` yields `NewItem`s from a source; the
CLI batches them through `Store::ingest_many` per session file (or per
directory batch), with the hook wiring already used by `ingest`.
Idempotency is enforced by the database: `items.external_id TEXT UNIQUE`
(nullable). Before ingesting a batch, the CLI asks the store which
external ids already exist and drops those; the unique index is the
backstop against races.

### Approaches discarded

- **Metadata JSON + `json_extract` expression index.** Still a DDL change
  that has to be documented, but hides the uniqueness rule inside JSON.
  Nothing gained over a real column.
- **Sidecar ledger of seen ids.** No schema change, but it is a private
  format (forbidden by the Open/Closed hard boundary rules) and drifts
  from the store on any manual edit.

## Architecture

```
singularmem (CLI)
  ├── ingest-transcript ─┐
  └── ingest-dir ────────┤
                         ▼
             singularmem-ingest (new crate)
               ├── Source trait: iterate → Result<NewItem>
               ├── claude::ClaudeTranscript  (JSONL parser)
               ├── dir::DirectoryWalker      (ignore-crate walker)
               └── chunk::chunk_text         (shared chunker)
                         │
                         ▼
             singularmem-core::Store::{existing_external_ids, ingest_many}
                         │ IndexHook fan-out (unchanged)
                         ▼
             Tantivy + USearch sidecars
```

`singularmem-ingest` depends only on `singularmem-core`, `serde_json`,
`ignore`, `sha2`, `jiff`, `tracing`. It has no knowledge of search.

## Data model

### Store format v2

Migration `1 → 2`, run inside one transaction on open (per the ratchet
in `docs/formats/store-v1.md`):

```sql
ALTER TABLE items ADD COLUMN external_id TEXT;
CREATE UNIQUE INDEX idx_items_external_id ON items(external_id)
  WHERE external_id IS NOT NULL;
UPDATE singularmem_meta SET value = '2' WHERE key = 'format_version';
```

`external_id` limits: ≤ 512 bytes, no NUL, non-empty when present.
`Item` and `NewItem` gain `external_id: Option<String>`. Export gains an
optional `external_id` field per item line; `_singularmem_format` stays
`export-v1` (additive, ignorable by v1 loaders) and `store_format_version`
reports `"2"`. `docs/formats/store-v2.md` documents the column, the
migration, and the external-id conventions below. A binary at v2 opening
a v3+ store refuses, as today.

### External id conventions

| Source | `external_id` | `source` |
|---|---|---|
| Claude Code message | `claude-code:<sessionId>:<uuid>` | `claude-code:<sessionId>` |
| Claude Code message chunk n of N | `claude-code:<sessionId>:<uuid>#<n>` | same |
| File | `file:<absolute path>` | `dir:<absolute root>` |
| File chunk n of N | `file:<absolute path>#<n>` | same |

### Transcript item shape

Kept lines: `type` is `user` or `assistant`, `isMeta` is not true,
`isSidechain` is not true (unless `--include-sidechains`), and the
message has non-empty text after filtering. Text is the concatenation
of `content` when it is a string, or of every block with
`type == "text"` joined by a blank line. `<system-reminder>…</system-reminder>`
spans are removed from user text. Tool-result-only user lines and
thinking-only or tool-use-only assistant lines are skipped.

Tags: `transcript`, `claude-code`, `role:user` or `role:assistant`,
plus `sidechain` when applicable.

Metadata:

```json
{
  "session_id": "…", "uuid": "…", "parent_uuid": "…" | null,
  "role": "user" | "assistant",
  "cwd": "…" | null, "git_branch": "…" | null,
  "occurred_at": "<RFC3339 from line timestamp>" | null,
  "tool_names": ["Read", "Bash"],
  "chunk_index": 0, "chunk_count": 1
}
```

### Directory item shape

One item per file. Tags: `file`, `ext:<lowercase extension>` when
present. Metadata: `path` (absolute), `rel_path` (from the walked root),
`sha256`, `size_bytes`, `chunk_index`, `chunk_count`. If an item with the
same `external_id` exists but a different `sha256`, the new item is
ingested with `supersedes` pointing at the existing one **and** the old
item's external id must be freed: because the column is unique, the
migration-safe rule is that the *new* item carries the external id and
the old one is updated to `external_id = NULL` in the same transaction.
This is the one and only in-place mutation the store performs, exposed
as `Store::ingest_replacing(new: NewItem, replaces: ItemId)`, and it is
documented in `store-v2.md`.

### Chunking

`chunk_text(text, max_bytes = 4096) -> Vec<String>`. Split at blank-line
boundaries (`\n\n`) greedily; if a single paragraph exceeds `max_bytes`,
hard-split at the last char boundary ≤ `max_bytes`. Chunks are trimmed
of trailing whitespace and never empty; `chunks.join("\n\n")` normalises
back to the trimmed input. A message ≤ `max_bytes` is exactly one chunk.

## Interfaces

### Library (`singularmem-ingest`)

```rust
pub trait Source {
    /// Human label for progress/summary output.
    fn name(&self) -> String;
    /// Yield items in source order. Errors are per-item; the iterator
    /// continues after an `Err`.
    fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_>;
}

pub struct ClaudeTranscript { pub path: PathBuf, pub include_sidechains: bool, pub project_filter: Option<PathBuf> }
impl ClaudeTranscript { pub fn open(path: impl AsRef<Path>) -> Result<Self>; }
impl Source for ClaudeTranscript {}

pub fn discover_transcripts(root: impl AsRef<Path>) -> Result<Vec<PathBuf>>; // *.jsonl, recursive, sorted

pub struct DirectoryWalker { pub root: PathBuf, pub max_file_bytes: u64 /* default 1 MiB */ }
impl Source for DirectoryWalker {}

pub fn chunk_text(text: &str, max_bytes: usize) -> Vec<String>;

pub struct Report { pub ingested: usize, pub skipped_existing: usize, pub skipped_filtered: usize, pub failed: usize }
pub fn ingest_source(store: &Store, source: &dyn Source, dry_run: bool) -> Result<Report>;
```

`ingest_source` collects items, calls `store.existing_external_ids(&ids)`
to drop duplicates, handles the changed-file supersede case, and writes
the remainder with `ingest_many` in batches of 500.

### Library (`singularmem-core` additions)

```rust
pub struct NewItem { /* existing */ pub external_id: Option<String> }
pub struct Item    { /* existing */ pub external_id: Option<String> }
impl Store {
    pub fn get_by_external_id(&self, id: &str) -> Result<Option<Item>>;
    pub fn existing_external_ids(&self, ids: &[&str]) -> Result<HashSet<String>>;
    pub fn ingest_replacing(&self, item: NewItem, replaces: ItemId) -> Result<Item>;
}
pub const FORMAT_VERSION: &str = "2";
```

`Error` gains `ExternalIdConflict { external_id }` (unique violation) and
`Migration { from, to, source }`.

### CLI

```
singularmem ingest-transcript [PATH ...]
    --project <DIR>          only sessions whose cwd == DIR (canonicalised)
    --include-sidechains     keep subagent (isSidechain) messages
    --dry-run                parse + report, write nothing
    --quiet                  suppress per-file progress on stderr
PATH defaults to ~/.claude/projects. A PATH that is a directory is
searched recursively for *.jsonl.

singularmem ingest-dir <PATH>
    --max-file-bytes <N>     default 1048576
    --dry-run
    --quiet
```

Both print, to stderr, one line per file (`<file>: +N ingested, M skipped`)
unless `--quiet`, and a final summary
`ingested N, skipped M existing, K filtered, F failed across P files`.
stdout is empty (scriptable). Exit codes: 0 all good; 1 if any file
failed (summary still printed); 2 if the store is missing or read-only.
The existing global flags `--store`, `--read-only`, `--no-index` apply.

### Wire

No MCP or HTTP changes.

## Error handling

- **Malformed JSONL line:** `tracing::warn!` with file and line number,
  counted as `failed`, parsing continues.
- **Unreadable file or directory:** counted as a failed file, run
  continues, exit 1 at the end.
- **Unique-index violation** (race between `existing_external_ids` and
  the write): the batch transaction rolls back, the CLI retries that
  batch once after re-filtering; a second failure is reported per item
  as `ExternalIdConflict`.
- **Hook failure** after a successful SQLite write: unchanged from today
  (warn, advise `singularmem reindex`).
- **Migration failure:** transaction rolls back, store stays at v1, error
  reports `from`, `to`, and the SQLite cause. Nothing is half-migrated.
- **Read-only store:** refused before any parsing, exit 2.

## Testing strategy

- **Unit (`singularmem-ingest`):** fixture JSONL under `tests/fixtures/`
  covering every line type observed in a real session (`user` string,
  `user` text blocks, `user` tool_result-only, `assistant` text,
  `assistant` tool_use-only, `assistant` thinking-only, `attachment`,
  `system`, `last-prompt`, `mode`, `file-history-snapshot`, sidechain,
  isMeta, malformed line). Assert exact kept set, tags, metadata.
- **Property (`proptest`):** `chunk_text` never yields empty chunks, every
  chunk ≤ `max_bytes`, joined chunks equal trimmed input.
- **Migration (`singularmem-core`):** open a checked-in v1 fixture store,
  assert `format_version() == "2"`, `external_id` column present, all
  items intact, export succeeds. Opening a `"3"` store still refuses.
- **CLI (`tests/cli.rs`):** run `ingest-transcript` on a fixture dir
  twice; second run reports `ingested 0`. `ingest-dir` on a temp tree,
  modify one file, re-run: exactly one new item, `revisions` shows the
  chain. `--dry-run` writes nothing. `search` finds an ingested message.
- **Offline:** no network in any test. Embedding tests use
  `SINGULARMEM_TEST_EMBEDDER=mock`.
- **Open-core round trip:** the existing `open_core_only_round_trip` test
  is extended with an `external_id`-bearing item.

## Open questions

None blocking. Timestamp-as-`created_at` was considered and rejected
(see non-goals); sub-project 14 may add a queryable `occurred_at` if the
timeline needs it.

## Acceptance criteria

1. `cargo test --workspace` passes offline; new crate has ≥ 90% line
   coverage on parser + chunker.
2. `singularmem ingest-transcript ~/.claude/projects/-Users-x-proj` on a
   real project ingests every user/assistant text message exactly once;
   running it again prints `ingested 0`.
3. `singularmem ingest-dir .` on this repository ingests source files
   and skips `target/`, `.git/`, and binaries; re-run ingests 0; editing
   one file and re-running ingests 1 and `revisions <new-id>` shows 2.
4. `singularmem search "<phrase from an ingested message>"` returns it.
5. A v1 store from the `v0.16.0` binary opens under the new binary,
   reports format version 2, and exports cleanly.
6. `docs/formats/store-v2.md` exists and a third-party loader following
   it can read the fixture store (verified by the migration test using
   raw `rusqlite`, no `Store`).
7. Ingest throughput via `ingest_many` stays ≥ 50 items/s in the
   `perf-budgets` CI job.

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | Reads local files only. No network. |
| **II — Provider-Agnostic by Contract** | The Claude Code parser is one `Source` impl; the trait is the extension point for Codex/Cursor/Gemini formats. Nothing provider-specific leaks into core. |
| **III — Open Core with a Stable Boundary** | Wholly open. Format v2 is documented in `store-v2.md`; the one in-place mutation (`ingest_replacing`) is specified. |
| **IV — CLI-First, GUI-Visible** | Two new verbs expose every new capability; scriptable, empty stdout, defined exit codes. |
| **V — Composable Library Architecture** | `singularmem-ingest` is a standalone crate with a trait-based API; the CLI is a thin shell. |
| **VI — Deterministic and Offline-Testable** | Fixtures + proptest; no network. ULIDs come from the injected clock/rng as today. |
| **VII — Honest Failure Modes** | Per-line and per-file failures are counted and surfaced; nothing is silently dropped; migration is transactional. |
| **VIII — Privacy Telemetry Boundary** | No telemetry. |
| **IX — Accessible by Default** | CLI plain text only. |
| **X — Performance Budgets, Enforced in CI** | Batched `ingest_many` keeps the ≥ 50 items/s budget; benchmark unchanged. |

## Deviations recorded during implementation

The shipped implementation differs from this design in the following
ways. Each is deliberate; none changes the acceptance criteria.

1. **WAL pragma is scoped to write-mode opens.** `journal_mode = WAL` is
   a persistent, file-mutating pragma, so a read-only open cannot set it
   (and must not try). It is issued only when the store is opened
   writable.

2. **The 1 → 2 migration uses `BEGIN IMMEDIATE` plus an in-transaction
   version re-check.** A deferred transaction would upgrade to a write
   lock only at the first DDL statement, racing a second writer that
   already migrated. Taking the write lock up front and re-reading
   `format_version` inside the transaction makes the losing side a
   no-op rollback that returns `Ok(())`.

3. **The chunker's reassembly property holds only up to whitespace.**
   `chunks.join("\n\n")` reproduces the input only when no paragraph
   needed a hard split: a hard split drops the whitespace run that
   straddles the boundary. The proptest therefore asserts the weaker,
   true property — the non-whitespace characters survive in order —
   rather than byte-for-byte reassembly.

4. **`ingest_replacing` is used only when BOTH sha256 hashes are
   present.** An item already in the store without a
   `metadata.sha256` (ingested by hand, say) is never replaced by a
   hashed candidate: with nothing to compare, "changed" cannot be
   established, so the candidate is counted as `skipped_existing`.

5. **Superseded items remain in the Tantivy/`USearch` indexes until
   `singularmem reindex`.** `IndexHook` has no removal path, so repeated
   `ingest-dir` runs over a changing tree accumulate stale search hits
   pointing at superseded revisions. Tracked for sub-project 12; see
   `docs/formats/store-v2.md` § "Known limitations".

6. **A change in a file's chunk count orphans its previous items.** The
   `external_id` shape depends on the count (`file:<path>` for a single
   chunk, `file:<path>#n` for several), so crossing that boundary
   produces ids the store has never seen: the new items are inserted
   fresh and the old ones are left in place holding their old ids rather
   than being superseded. Tracked for sub-project 12.

7. **The plan's CLI tests assert `.code(1)`, not `.success()`.** The
   shared transcript fixture deliberately contains one malformed JSON
   line so the per-line failure path is covered. That line counts as a
   failure on every run, including idempotent re-runs and `--dry-run`,
   and the exit code honestly reflects it.
