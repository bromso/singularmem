---
title: MCP server — write + utility tools (sub-project 4b)
date: 2026-05-18
status: draft
sub-project: 4b-mcp-server-write-tools
supersedes: none
---

# MCP server — write + utility tools (sub-project 4b)

This sub-project completes the MCP server's tool surface for v0.
Building on sub-project 4a (v0.9.0), it adds four tools to the
existing `singularmem-mcp` crate: `memory_ingest` (write),
`memory_get`, `memory_list`, `memory_revisions` (utility reads).
After 4b merges, the MCP server's tool surface matches the existing
`singularmem` CLI's verbs, giving MCP clients (Claude Code, Cursor,
custom agents) full read+write parity. A new `--read-only` flag
disables write tools for shared-memory deployments.

## Problem & motivation

Sub-project 4a shipped the MCP foundation + `memory_retrieve`. That
gave MCP clients a way to fetch relevant memories from a pre-populated
store, but every ingest still required the CLI. For LLM agents to
maintain memory autonomously — saving conversation summaries,
decisions, follow-ups — the MCP server needs a write path.

Three companion read tools (`memory_get`, `memory_list`,
`memory_revisions`) round out the surface. They mirror the existing
CLI verbs of the same names. Without them, an LLM that ingested a
memory has no way to verify it, list what's in the store, or walk
the supersedes chain when revising prior memories.

The `--read-only` flag is the safety valve. Shared-memory deployments
(team knowledge bases, demo sandboxes) want the LLM to read but not
write. Defense-in-depth: discovery-layer filtering keeps writes off
the menu, enforcement-layer rejection catches misbehaving clients.

After 4b, sub-project 4 is complete. Future MCP work (HTTP transport,
resources, prompts) becomes separate sub-projects.

## Goals & non-goals

### Goals

1. Four new tools in `singularmem-mcp`: `memory_ingest`, `memory_get`,
   `memory_list`, `memory_revisions`.
2. Each tool returns a single text content block with a shape
   documented in Interfaces (plain ASCII, mirroring CLI output where
   sensible).
3. `memory_ingest` accepts the full CLI parity surface: `content`
   (required), `tags` (array), `source` (string), `supersedes` (ULID
   string), `metadata` (JSON object).
4. `--read-only` flag (+ `SINGULARMEM_READ_ONLY` env var) excludes
   `memory_ingest` from `tools/list` AND rejects direct calls.
5. Auto-wiring of Tantivy + USearch index hooks on `memory_ingest`
   (mirrors the root binary's `open_store()` logic) so new memories
   are immediately discoverable via `memory_retrieve`.
6. 18 new unit tests + 2 new integration tests + 1 updated
   integration test (`mcp_handshake`'s `tools/list` count).
7. README + `docs/mcp-server.md` updated to cover the five-tool
   surface.
8. Workspace version bumps to `v0.10.0` on merge.

### Non-goals

- **No HTTP / SSE transport.** Still stdio-only; deferred to a later
  sub-project.
- **No MCP resources or prompts.** Capabilities continue to declare
  only `tools: {}`.
- **No `--enable-tools=...` per-tool flag.** `--read-only` is the
  only granularity. Per-tool gating is YAGNI for v0.
- **No streaming responses or progress notifications.** All five
  tools return a single complete text block.
- **No shared "store opener" library extraction.** The auto-wiring
  logic in `handle_memory_ingest` duplicates the root binary's
  `open_store()`. ~30 lines. A future third consumer (e.g., the
  TS SDK in sub-project 5) can prompt extraction; for now YAGNI.
- **No structured (JSON) content blocks.** Text only. Callers who
  want structured data have the CLI's `--format jsonl` and `--json`
  flags.
- **No `--format` knob on MCP tool outputs.** The MCP server picks
  one human-readable format per tool; clients that want machine-
  parsable output can parse the text or use the CLI directly.
- **No `--no-index` MCP equivalent.** The CLI has a `--no-index`
  flag that skips hook wiring on `Store::open`. The MCP server always
  attempts to wire hooks for writes (otherwise new memories wouldn't
  be retrievable, breaking the integration with `memory_retrieve`).
- **No long-lived store handle.** Per 4a's precedent, store opens
  are per-request. Cost is dominated by `FastembedEmbedder` model
  load for ingest in production; that's a future optimization
  problem if it surfaces.
- **No on-disk changes.** `format_version` stays `"1"`; the four new
  tools are pure dispatch over existing `singularmem-core` APIs.

## Recommended approach

Four new handler modules under `crates/singularmem-mcp/src/tools/`,
each following the same shape as `retrieve.rs` from 4a: a serde
`*Args` struct, a `tool_descriptor()` function returning the rmcp
`Tool` value with its JSON schema, a `handle_*` function returning
`Result<*Output>`, and a `mod tests` block with handler-level unit
tests using the existing `seeded()` fixture pattern.

Server registration (`src/server.rs`) extends `list_tools` to emit up
to five descriptors (filtering out `memory_ingest` when
`config.read_only`) and `call_tool` to dispatch by name. The
read-only guard appears in two places: at `list_tools` so well-behaved
clients never see ingest, and at `call_tool` as enforcement-layer
defense for misbehaved clients.

`memory_ingest` is uniquely complex among the four because it needs
the same index auto-wiring the root binary's `open_store()` does:
Tantivy hook when `.tantivy/` sidecar exists, EmbedderIndex hook when
`.vectors/` exists, MockEmbedder vs FastembedEmbedder selection via
`SINGULARMEM_TEST_EMBEDDER` env var. The duplication is intentional
for v0 (YAGNI).

`Config` gains a `read_only: bool` field. `main.rs` adds a `--read-only`
clap arg with `SINGULARMEM_READ_ONLY` env binding. All read handlers
open the store with `Store::open_with_options(.., StoreOptions {
read_only: true })` when `config.read_only` is set, propagating
SQLite's read-only mode as a third safety layer.

### Approaches discarded

- **Approach B — bundle the four tools into one big sub-project
  alongside 4a.** Rejected at the 4a brainstorm; same rationale
  applies. Splitting kept 4a in the ~10-task budget; 4b stays in
  the ~7-task budget. Reviewable per-PR.

- **Approach C — split read and write further (4b = `memory_ingest`
  only; 4c = read utilities).** Rejected. The four tools are
  naturally related; deferring `memory_get`/`list`/`revisions`
  leaves an awkward gap (LLM can ingest but can't verify or
  enumerate). One PR delivers them together.

- **Approach D — extract a shared "store opener" library now.**
  Rejected per YAGNI. ~30 lines of duplication between root binary
  and MCP server. A third consumer (sub-project 5 TS SDK) would
  justify extraction; for now, two duplicates is fine.

- **Approach E — `--enable-tools=ingest,get,list` granularity.**
  Rejected. Per-tool gating is overkill; the read-only / read+write
  binary is the meaningful distinction. Power users can wrap the
  server (e.g., spawn it then proxy via their own MCP frontend).

- **Approach F — structured JSON content blocks instead of text.**
  Rejected. MCP clients render text blocks directly to the LLM;
  structured blocks require client-side parsing. Text mirrors what
  LLMs consume effectively. CLI users who want JSON have `--json`
  / `--format jsonl`.

- **Approach G — long-lived store handle for performance.**
  Rejected per 4a's precedent. Per-request opens cost microseconds
  for SQLite; FastembedEmbedder model load is the only meaningful
  cost and a future optimization concern.

## Architecture

Components (additive — none of 4a's files are renamed or
restructured):

- **`crates/singularmem-mcp/src/error.rs`** — gains `Error::ReadOnly`
  (MCP `InvalidParams`) and `Error::InvalidId(String)` (for
  `supersedes` parse failures).
- **`crates/singularmem-mcp/src/config.rs`** — `Config` gains
  `read_only: bool`. `Config::new` takes a fourth parameter.
- **`crates/singularmem-mcp/src/main.rs`** — clap `Args` gains
  `--read-only` (with `SINGULARMEM_READ_ONLY` env). Passed to
  `Config::new`.
- **`crates/singularmem-mcp/src/server.rs`** — `list_tools` extends
  to emit up to five descriptors (filter `memory_ingest` when
  `read_only`). `call_tool` extends to dispatch all five by name
  (reject `memory_ingest` when `read_only`).
- **`crates/singularmem-mcp/src/tools/ingest.rs`** — new file.
  `MemoryIngestArgs` serde struct, `tool_descriptor()`,
  `handle_memory_ingest()`, 6 unit tests.
- **`crates/singularmem-mcp/src/tools/get.rs`** — new file.
  `MemoryGetArgs`, `tool_descriptor()`, `handle_memory_get()`,
  3 unit tests.
- **`crates/singularmem-mcp/src/tools/list.rs`** — new file.
  `MemoryListArgs`, `tool_descriptor()`, `handle_memory_list()`,
  4 unit tests.
- **`crates/singularmem-mcp/src/tools/revisions.rs`** — new file.
  `MemoryRevisionsArgs`, `tool_descriptor()`,
  `handle_memory_revisions()`, 4 unit tests.
- **`crates/singularmem-mcp/src/tools/mod.rs`** — re-export the
  new handler functions and `Args` types.
- **`crates/singularmem-mcp/tests/mcp_handshake.rs`** — updated to
  assert 5 tools in `tools/list`.
- **`crates/singularmem-mcp/tests/full_write_read_cycle.rs`** — new
  end-to-end test exercising ingest → get → list → revisions →
  retrieve.
- **`crates/singularmem-mcp/tests/read_only_mode.rs`** — new test
  verifying `--read-only` semantics.
- **`crates/singularmem-mcp/README.md`** — status banner + tools
  list + config table + troubleshooting + roadmap updated per §5.
- **`docs/mcp-server.md`** — extended layering diagram + tools list
  + read-only section + roadmap updated per §5.

Layering stays: `core ← search ← retrieve ← adapters` (unchanged).
The MCP server remains a thin shell composing libraries. The four
new tools dispatch directly to `singularmem-core::Store` operations
(no `singularmem-retrieve` involvement for get/list/revisions/ingest;
only `memory_retrieve` from 4a uses Retriever + adapters).

## Data model

**No changes.** All four new tools consume existing types:
`singularmem-core::{Store, Item, ItemId, NewItem}` from sub-project 1
unchanged. No persistent data, no on-disk artefacts, no
`format_version` bump.

## Interfaces

### CLI (binary surface)

One new flag added to `singularmem-mcp`:

```
      --read-only          Open the store in read-only mode. Excludes memory_ingest
                           from tools/list AND rejects direct calls.
                           [env: SINGULARMEM_READ_ONLY] [default: false]
```

All four other flags from 4a unchanged.

### MCP tools

#### `memory_ingest`

Tool descriptor:

```json
{
  "name": "memory_ingest",
  "description": "Add a new memory to the user's local Singularmem store. Returns the new memory's ID and timestamp. Memories are private to this user.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "content":    { "type": "string", "description": "Memory body text. Non-empty, max 1 MiB." },
      "tags":       { "type": "array", "items": { "type": "string" }, "description": "Optional tag labels (non-empty strings, max 64 bytes each, deduplicated)." },
      "source":     { "type": "string", "description": "Optional provenance label. Max 256 bytes." },
      "supersedes": { "type": "string", "description": "Optional ULID of an existing memory this one corrects. Must exist in the store." },
      "metadata":   { "type": "object", "description": "Optional user-defined JSON object. Soft warning threshold 64 KiB." }
    },
    "required": ["content"]
  }
}
```

Output (text content block):

```
Ingested memory 01ARZ3NDEKTSV4RRFFQ69G5FAV at 2026-05-18T14:30:00Z
```

Handler (`src/tools/ingest.rs`):

```rust
pub fn handle_memory_ingest(
    args: MemoryIngestArgs,
    config: &Config,
) -> Result<MemoryIngestOutput> {
    if config.read_only {
        return Err(Error::ReadOnly);
    }
    let supersedes = args.supersedes
        .as_deref()
        .map(|s| s.parse::<ItemId>())
        .transpose()
        .map_err(|e| Error::InvalidId(e.to_string()))?;
    let mut item = NewItem::text(args.content);
    item.tags = args.tags.unwrap_or_default();
    item.source = args.source;
    item.supersedes = supersedes;
    if let Some(meta) = args.metadata {
        item.metadata = meta;
    }
    let store = open_store_with_hooks(&config.store_path)?;
    let stored = store.ingest(item)?;
    tracing::info!(id = %stored.id, "memory ingested via MCP");
    Ok(MemoryIngestOutput {
        id: stored.id,
        created_at: stored.created_at,
    })
}
```

`open_store_with_hooks` mirrors the root binary's `open_store()`:
opens `.tantivy/` sidecar → Tantivy hook; opens `.vectors/` sidecar
→ EmbedderIndex hook; MockEmbedder vs FastembedEmbedder via
`SINGULARMEM_TEST_EMBEDDER` env var.

#### `memory_get`

Tool descriptor:

```json
{
  "name": "memory_get",
  "description": "Fetch a single memory by ID. Returns the memory's content and metadata as text.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "id": { "type": "string", "description": "ULID of the memory to fetch (26 characters, Crockford base32)." }
    },
    "required": ["id"]
  }
}
```

Output (text content block):

```
Memory 01ARZ3NDEKTSV4RRFFQ69G5FAV
Created: 2026-05-18T14:30:00Z
Source: claude-conversation:abc-123
Tags: fox, animals

the quick brown fox jumps over the lazy dog
```

`Source:` line omitted when source is `None`; `Tags:` line omitted
when tags are empty. Content body follows a blank line.

#### `memory_list`

Tool descriptor:

```json
{
  "name": "memory_list",
  "description": "Enumerate memories in the store, optionally filtered by tag (AND-semantics). Returns a compact listing with IDs and content snippets.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "tags":  { "type": "array",   "items": { "type": "string" }, "description": "AND-filter tags." },
      "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 50, "description": "Maximum number of items to return." }
    },
    "required": []
  }
}
```

Output (text content block):

```
Found 3 memories (limit 50):

01ARZ3NDEKTSV4RRFFQ69G5FAV: the quick brown fox jumps over the lazy dog
01BX5ZZKBKACTAV9WEVGEMMVRZ: lazy dogs sleep all day
01CW8BZ7FQRJM4HCVCV9ABCDEF: another memory with longer content trunc...
```

Content truncated to ~80 chars per item. Newlines in content
replaced with spaces (per existing CLI `list --format table`
convention).

#### `memory_revisions`

Tool descriptor:

```json
{
  "name": "memory_revisions",
  "description": "Walk the supersedes chain for a memory, newest-first. Returns each revision in the chain with ID and content snippet.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "id": { "type": "string", "description": "ULID of any item in the chain." }
    },
    "required": ["id"]
  }
}
```

Output (text content block):

```
Revisions of 01CW8BZ7FQRJM4HCVCV9ABCDEF (3 items, newest first):

01CW8BZ7FQRJM4HCVCV9ABCDEF: latest content here
01BX5ZZKBKACTAV9WEVGEMMVRZ: revised content
01ARZ3NDEKTSV4RRFFQ69G5FAV: original content
```

Same content-truncation convention as `memory_list`. Chain ordering
matches `singularmem revisions` CLI verb (newest-first).

### Library

`crates/singularmem-mcp/src/tools/mod.rs` extends its re-exports:

```rust
pub mod get;
pub mod ingest;
pub mod list;
pub mod retrieve;
pub mod revisions;

pub use crate::tools::get::{handle_memory_get, MemoryGetArgs, MemoryGetOutput};
pub use crate::tools::ingest::{handle_memory_ingest, MemoryIngestArgs, MemoryIngestOutput};
pub use crate::tools::list::{handle_memory_list, MemoryListArgs, MemoryListOutput};
pub use crate::tools::retrieve::{handle_memory_retrieve, MemoryRetrieveArgs, MemoryRetrieveOutput};
pub use crate::tools::revisions::{handle_memory_revisions, MemoryRevisionsArgs, MemoryRevisionsOutput};
```

`crates/singularmem-mcp/src/lib.rs` extends its re-export list to
include the new types, alphabetically sorted.

### Wire (MCP / HTTP / IPC)

stdio only (unchanged from 4a). MCP wire protocol is JSON-RPC 2.0 per
the spec; rmcp 1.7.0 handles framing.

## Error handling

Per Principle VII, every error variant maps to a specific MCP error
code with a human-readable message.

| Source / variant | MCP error code | Message |
|---|---|---|
| `Error::ReadOnly` | `InvalidParams` | "server is read-only; memory_ingest is disabled" |
| `Error::InvalidId(_)` | `InvalidParams` | "invalid item ID: \<err\>" |
| `Error::Core(NotFound { id })` | `InvalidParams` | "memory not found: \<id\>" |
| `Error::Core(SupersedesNotFound { id })` | `InvalidParams` | "supersedes target \<id\> not found" |
| `Error::Core(Validation { field, reason })` | `InvalidParams` | "\<field\>: \<reason\>" |
| `Error::Core(other)` | `InternalError` | underlying message |
| `Error::Search(_)` (hook open failures) | `InternalError` | underlying message; logged via `tracing::warn!` first |
| `Error::Retrieve(_)` (only from `memory_retrieve`, unchanged from 4a) | per 4a's table | per 4a's table |

**No silent failures.** Hook open failures during `memory_ingest`'s
auto-wiring are logged at `warn` level but do NOT abort the ingest —
the memory still lands in SQLite. This matches the root binary's
auto-wiring behavior (Principle VII's "what state was preserved":
the SQLite write committed; the affected sidecar didn't get updated;
re-running `singularmem reindex` would catch it up).

**stdio purity (continued from 4a):** `tracing` continues to write
to stderr only; stdout is reserved for JSON-RPC. The new
`tracing::info!` log line for ingest events (with the ULID only, no
content) is on stderr.

## Testing strategy

### Unit tests (per-tool, in `src/tools/*.rs`)

Eighteen new tests across the four tools + one new `Config` test.

| Tool | Tests |
|---|---|
| `memory_ingest` | `ingest_succeeds_returns_id_and_timestamp`, `ingest_empty_content_returns_validation_error`, `ingest_with_supersedes_links_to_existing`, `ingest_with_unknown_supersedes_returns_error`, `ingest_with_tags_and_source_persists_them`, `ingest_rejected_in_read_only_mode` |
| `memory_get` | `get_returns_full_item`, `get_invalid_ulid_returns_error`, `get_not_found_returns_error` |
| `memory_list` | `list_returns_all_when_no_filter`, `list_respects_tag_filter`, `list_caps_limit_at_100`, `list_default_limit_50` |
| `memory_revisions` | `revisions_walks_chain_newest_first`, `revisions_for_single_item_returns_one`, `revisions_invalid_ulid_returns_error`, `revisions_not_found_returns_error` |
| `config.rs` | `config_new_preserves_read_only_flag` |

All use the existing `seeded()` fixture pattern from 4a's
`tools/retrieve.rs` — `MockEmbedder` via
`SINGULARMEM_TEST_EMBEDDER=mock`, no network.

### Integration tests (`crates/singularmem-mcp/tests/`)

Three tests total:

**Updated**: `mcp_handshake.rs` — the existing `tools/list`
assertion expects one tool; update to expect five (or four when
`memory_ingest` is conditionally registered — the updated test runs
without `--read-only` so expects five).

**New**: `full_write_read_cycle.rs` — single end-to-end test
exercising the LLM workflow:

1. Spawn `singularmem-mcp` against a fresh tempdir.
2. `initialize` handshake.
3. `memory_ingest` with content + tags + source → assert response
   has an ID.
4. `memory_get` with that ID → assert response contains original
   content.
5. `memory_ingest` with `supersedes: <first-id>` → assert response
   has a new ID.
6. `memory_revisions` with the new ID → assert 2-item chain in
   response.
7. `memory_list` → assert 2 items in response.
8. `memory_retrieve` matching content → assert response contains
   content.

This proves the full read+write story works end-to-end through MCP,
including hook auto-wiring (step 8 retrieving step 3's ingest
requires the hooks to have fired).

**New**: `read_only_mode.rs` — verifies `--read-only` semantics:

1. Pre-seed a store via the `singularmem` CLI.
2. Spawn `singularmem-mcp --read-only` against the seeded store.
3. `initialize` + `tools/list` → assert exactly four tools
   (`memory_retrieve`, `memory_get`, `memory_list`,
   `memory_revisions`); `memory_ingest` absent.
4. `tools/call memory_ingest` anyway → assert MCP `InvalidParams`
   error with "read-only" in the message.
5. `tools/call memory_get` → assert success (reads still work).

Defense-in-depth confirmation: discovery-layer filtering AND
enforcement-layer rejection both verified.

### Perf budget

**No new budget.** Read tools are sub-millisecond. `memory_ingest`
is bounded by `Store::ingest` + hook costs; FastembedEmbedder
production behavior is implicitly tracked by sub-project 2b's
`semantic_search_latency` budget (same model code path).

### Offline guarantee

All tests use `MockEmbedder` via `SINGULARMEM_TEST_EMBEDDER=mock`.
The integration tests pass the env var when spawning subprocesses
(both the `singularmem` seed step and the `singularmem-mcp` server
step). No network for any test. The `tests-offline` advisory CI job
picks up the new tests automatically.

### Lint + fmt + doc gates

Same as 4a + all prior sub-projects. Watch for `clippy::doc_markdown`
on tool names (backtick `memory_ingest`, `memory_get`, etc.) and
`clippy::module_name_repetitions` on type names like
`MemoryGetArgs` in the `get` module.

## Open questions

None at spec time. Three notes for the implementation plan:

1. **Auto-wiring duplication** is YAGNI-intentional. The plan should
   tell the implementer not to extract a shared helper. ~30 lines of
   duplication between `singularmem-mcp::tools::ingest::open_store_with_hooks`
   and the root binary's `src/main.rs::open_store`.

2. **`tools/list` ordering**: descriptors should appear in a stable
   order (matches what the integration tests assert). Recommended:
   `memory_retrieve`, `memory_get`, `memory_list`, `memory_revisions`,
   `memory_ingest`. Reads first, write last. The plan should pin
   this so future additions don't accidentally re-order existing
   tools.

3. **Content truncation width**: `memory_list` and `memory_revisions`
   truncate content to ~80 chars per line. Implementation should
   handle multi-byte UTF-8 boundaries correctly (use `.chars().take(80)`
   not `.bytes().take(80)`). Same convention as the existing CLI's
   `list --format table` output.

## Acceptance criteria

1. Four new source files under `crates/singularmem-mcp/src/tools/`:
   `ingest.rs`, `get.rs`, `list.rs`, `revisions.rs`. Each contains a
   `*Args` serde struct, a `tool_descriptor()` function, a
   `handle_*` function, and a `mod tests` block with unit tests.
2. `src/tools/mod.rs` re-exports the four new handler functions and
   `Args`/`Output` types alongside the existing `retrieve` exports.
3. `src/error.rs` gains `Error::ReadOnly` and `Error::InvalidId(String)`.
4. `src/config.rs` gains a `read_only: bool` field on `Config`.
   `Config::new` takes it as a fourth parameter. New unit test
   `config_new_preserves_read_only_flag` passes.
5. `src/main.rs` gains a `--read-only` clap arg with
   `SINGULARMEM_READ_ONLY` env var, default `false`. Passes through
   to `Config::new`.
6. `src/server.rs::list_tools` returns 5 tool descriptors when
   `config.read_only == false`, 4 when `true` (omits `memory_ingest`).
   Ordering: `memory_retrieve`, `memory_get`, `memory_list`,
   `memory_revisions`, `memory_ingest`.
7. `src/server.rs::call_tool` dispatches all 5 tools by name. When
   `config.read_only == true` AND `name == "memory_ingest"`, returns
   `InvalidParams` with the standard read-only message (without
   reaching the handler).
8. `handle_memory_ingest` uses `open_store_with_hooks` (a private
   helper in `tools/ingest.rs`) that mirrors the root binary's
   `open_store` auto-wiring: Tantivy hook when `.tantivy/` exists,
   EmbedderIndex hook when `.vectors/` exists, MockEmbedder vs
   FastembedEmbedder via `SINGULARMEM_TEST_EMBEDDER` env var. The
   duplication is intentional for v0.
9. All four read handlers open the store via `Store::open_with_options`
   with `StoreOptions { read_only: config.read_only }` so SQLite's
   read-only mode is propagated as a third safety layer.
10. Output formats per the Interfaces section: text content blocks
    with the shapes specified there. Content truncation in
    `memory_list` and `memory_revisions` uses character-boundary
    truncation (`.chars().take(N)`), not byte truncation.
11. Error mapping per the Error Handling table.
12. 17 new unit tests across the four tool modules + 1 new
    `Config` test = 18 new unit tests total. All pass on
    `ubuntu-latest` and `macos-latest`.
13. `tests/mcp_handshake.rs` updated to assert 5 tools in
    `tools/list`.
14. New `tests/full_write_read_cycle.rs` exercises ingest → get →
    list → revisions → retrieve through MCP.
15. New `tests/read_only_mode.rs` verifies `--read-only` excludes
    `memory_ingest` from `tools/list` AND rejects direct calls.
16. No new perf budget.
17. `crates/singularmem-mcp/README.md` updated: status banner,
    expanded "Available tools" section (all five), new `--read-only`
    row in config table, new troubleshooting entry, "What's coming
    next" section reframed for post-4b roadmap.
18. `docs/mcp-server.md` updated: extended layering diagram with
    five tool branches, expanded tools list, new "Read-only mode"
    section, promoted roadmap.
19. `cargo fmt --check`, `cargo clippy --workspace --all-targets
    --tests --benches -- -D warnings`, and `RUSTDOCFLAGS='-D
    missing-docs -D warnings' cargo doc --workspace --no-deps`
    all clean.
20. `docs/formats/store-v1.md` unchanged; `format_version` stays
    `"1"`.
21. Tagged on merge as `v0.10.0` (additive MINOR bump per
    Principle V).
22. Project memory updated post-merge with 4b deliverables section
    + forward-pointer to sub-project 5 (TS SDK binding via napi-rs).

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | No new network calls. The server runs locally; all four new tools dispatch to the local SQLite store + local sidecars. |
| **II — Provider-Agnostic by Contract** | No adapter changes. The four new tools don't render through adapters (only `memory_retrieve` from 4a does); they emit plain-text formatted output directly. |
| **III — Open Core with a Stable Boundary** | III.a: pure additive surface (4 new tools, 1 new flag, 1 new field on `Config`, 2 new error variants). Nothing removed. III.b: no on-disk changes; `format_version` unchanged. The MCP server is named in the constitution's Open / Closed Split (line 245) as an open-core deliverable. |
| **IV — CLI-First** | All MCP tool surface mirrors existing CLI verbs (`ingest`, `get`, `list`, `revisions`). The `--read-only` flag mirrors the existing `singularmem --read-only`. CLI users retain power-user features (--metadata JSON, --format jsonl, etc.) not surfaced in MCP. |
| **V — Composable Library Architecture** | The MCP server remains a thin shell. Domain logic stays in `singularmem-core` (Store), `singularmem-search` (hooks), `singularmem-retrieve` (Retriever, only used by `memory_retrieve`). MCP crate owns dispatch + the (intentional) auto-wiring duplication. |
| **VI — Deterministic and Offline-Testable** | All unit tests use `MockEmbedder` via the existing fixture pattern. All three integration tests set `SINGULARMEM_TEST_EMBEDDER=mock`. No network. |
| **VII — Honest Failure Modes** | Validation errors map to MCP `InvalidParams` with the underlying singularmem-core message. Supersedes-not-found maps to `InvalidParams` (callable error, not a server fault). Read-only rejection maps to `InvalidParams` with a clear message. Write-path I/O failures map to `InternalError`. Hook open failures during ingest auto-wiring log at `warn` but don't abort the ingest (state preserved: SQLite write committed; sidecar didn't update; `singularmem reindex` recovers). No silent failures. |
| **VIII — Privacy Telemetry** | No telemetry. The server logs ingest events at `info` level with the new memory's ULID only — no content, no source, no tags. Adheres to "no PII in logs" rule. |
| **IX — Accessible by Default** | All four new tool outputs are plain ASCII text. No Unicode-only characters in new output formats (revisions uses ASCII parens, list uses ASCII colons). The U+2014 em-dash from GeminiAdapter only appears in `memory_retrieve`'s output via that adapter — already covered in sub-project 3d's accessibility note. |
| **X — Performance Budgets, Enforced in CI** | No new budget. Read tools are sub-millisecond. Write tool bounded by `Store::ingest` + hook costs (implicitly tracked by sub-project 2b's `semantic_search_latency` budget which exercises FastembedEmbedder via the same code path). |
