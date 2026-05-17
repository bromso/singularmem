---
title: Provider adapters & retrieval (sub-project 3a — foundation)
date: 2026-05-18
status: draft
sub-project: 3a-provider-adapters-foundation
supersedes: none
---

# Provider adapters & retrieval — sub-project 3a (foundation)

This sub-project establishes the typed adapter contract Principle II
requires and the retrieval layer that adapters compose. It ships a new
crate `singularmem-retrieve` containing `Retriever`, the `Adapter`
trait, `PlainAdapter` (a usable real adapter satisfying the
"one fully local runtime" half of Principle II), `MockAdapter` for
cross-crate testing, and a `singularmem retrieve` CLI verb. Sub-projects
3b, 3c, 3d each add one cloud-provider adapter (Claude, OpenAI/Codex,
Gemini) by implementing the trait and registering with the CLI.

## Problem & motivation

The constitution's Principle II ("Provider-Agnostic by Contract") requires
the system to integrate with at minimum Claude, OpenAI/Codex, Gemini, and
one fully local runtime through a single typed adapter contract. The
Open / Closed Split names "LLM provider adapters" as an open-core
deliverable. After sub-project 2c (v0.4.0), the project has all the
retrieval primitives — hybrid lexical+semantic search returns ranked
`ItemId`s — but nothing that turns those into prompt-ready context for
an LLM workflow.

Building all four adapters in one sub-project would dwarf sub-projects
2a/2b/2c (about 25-35 tasks vs the 15 tasks each of those took). The
decomposition this spec adopts is: **3a** delivers the contract + the
retrieval layer + one usable adapter (PlainAdapter, which doubles as
the local-runtime provider); **3b**, **3c**, **3d** add the three
cloud adapters one at a time. Each is independently reviewable, each
ships working software, and each can take user feedback before the
next begins.

This sub-project (3a) is the prerequisite for all three follow-ups.
Without the trait, there's nothing to plug into. Without the
`Retriever`, every adapter would duplicate the search-and-fetch glue
that belongs in one place. Without the CLI verb, Principle IV's
"every operation reachable from CLI" guarantee has a gap.

## Goals & non-goals

### Goals

1. New crate `singularmem-retrieve` with `Retriever`, `RetrieveOptions`,
   `RetrievedContext`, `MemoryBlock`, `Adapter` trait, `PlainAdapter`,
   `Error`, `Result`.
2. `MockAdapter` in a `testing` module, gated behind `feature = "testing"`
   so cross-crate tests (in sub-projects 3b/3c/3d) can use it without
   pulling in production adapter code.
3. CLI: `singularmem retrieve [OPTIONS] <QUERY>…` with the eleven flags
   listed in Interfaces, defaulting to `--adapter plain`.
4. Adapter registry: a `known_adapters()` function in the CLI that
   sub-projects 3b/3c/3d each extend with one line. Passing an
   unregistered adapter name exits 1 with a message listing what's
   known.
5. `PlainAdapter::format` produces deterministic, Markdown-shaped output
   suitable for local LLMs (Ollama, llama.cpp) and as a fallback for
   any cloud provider.

### Non-goals

- **No HTTP clients, no auth, no streaming.** Adapters are pure functions
  of `RetrievedContext` per the trait contract. LLM calls belong to the
  user's code (or to a future "adapter helper" sub-project that does not
  exist yet).
- **No memory ingest from adapters.** The trait is read-only. Writes
  continue via `Store::ingest` (programmatic), `singularmem ingest`
  (manual), and — in sub-project 4 — MCP tool calls.
- **No token-budget management.** `max_blocks` caps how many memories
  flow into the formatter, but the formatter does not measure tokens or
  truncate to fit a context window. Token budgets need per-provider
  tokenisers (tiktoken, Claude's tokenizer, Gemini's tokenizer) and
  are deferred until at least one cloud adapter has landed.
- **No query rewriting.** The retrieval API takes a single query
  string. Multi-turn conversation context handling, query
  reformulation, hypothetical-document expansion (HyDE) — all out of
  scope.
- **No `capture(response)` or memory extraction from LLM outputs.**
  Trait surface stays minimal; this is a substantive design problem
  deferred indefinitely.
- **No cloud provider adapters in this sub-project.** Claude, OpenAI,
  Gemini are sub-projects 3b, 3c, 3d respectively.
- **No on-disk changes.** Retrieval is read-only over the existing
  store + search sidecars.

## Recommended approach

A new crate `crates/singularmem-retrieve/` depends on `singularmem-core`
and `singularmem-search`. `Retriever<'a>` borrows `&'a Store` and
`&'a HybridSearcher<'a>` (same borrow pattern HybridSearcher uses for
its underlying indexes) and exposes a single `retrieve(&str,
&RetrieveOptions) -> Result<RetrievedContext>` method. The retriever
runs `HybridSearcher::search`, filters by `min_score`, truncates to
`max_blocks`, and fetches the full content for each hit via
`Store::get`. The result is a `RetrievedContext` with a `blocks: Vec<MemoryBlock>`
field carrying full content + score + provenance metadata for each
matched memory.

`Adapter` is a small trait: `name(&self) -> &str` + `format(&self,
&RetrievedContext) -> String`. Adapters are pure formatters; the trait
forbids I/O by convention (documented, not enforced by the type system).
`PlainAdapter` produces Markdown-shaped output with one block per
memory, score and ID in the heading, full content in the body, and
`---` separators between blocks.

The CLI gains a `retrieve` verb that reuses the same directory probe
and mode-resolution logic `cmd_search` uses (extracted as a shared
helper). `--adapter` validates against a registry function; only
`plain` is registered in 3a.

### Approaches discarded

- **Approach B — All four adapters in one sub-project.** Rejected
  because Principle II's "removing any adapter doesn't break non-provider
  features" is easier to validate incrementally — each adapter sub-project
  ends with a CI check that demonstrates removing that adapter still
  builds and tests pass. Bundling all four also lands ~25-35 tasks
  in one PR, fighting the discipline that made 2a/2b/2c reviewable.

- **Approach C — Adapter is a full LLM call wrapper.** Rejected
  because it pulls per-provider HTTP, auth, retries, streaming, and
  tool-calling into the open-core surface. Each provider becomes
  ~20-30 tasks of SDK plumbing. Pure-formatting adapter is the minimum
  Principle II demands; HTTP wrappers can be a later sub-project once
  the formatting contract is validated.

- **Approach D — No CLI verb in 3a; defer until first cloud adapter
  lands.** Rejected because PlainAdapter is a real adapter useful for
  local LLM workflows. Shipping the CLI verb with PlainAdapter as the
  default makes Principle IV's CLI surface complete on day one and
  gives users something they can pipe through `ollama run` immediately.

- **Approach E — Adapter trait lives in `singularmem-search`.**
  Rejected because future per-provider adapter crates would each pull
  in fastembed, USearch, Tantivy, and ORT transitively. Putting the
  trait in its own crate (singularmem-retrieve) keeps adapter crates'
  dependency surface small.

## Architecture

Components:

- **`crates/singularmem-retrieve/`** — new crate, version `0.5.0`
  (workspace-locked). Depends on `singularmem-core`, `singularmem-search`,
  `serde`, `serde_json`, `thiserror`, `tracing`, `jiff`.
- **`singularmem_retrieve::Retriever<'a>`** — borrows `&'a Store` and
  `&'a HybridSearcher<'a>`. Single `retrieve` method.
- **`singularmem_retrieve::RetrieveOptions`** — `max_blocks` (default 10),
  `min_score` (default 0.0), `search: HybridSearchOptions` (pass-through).
- **`singularmem_retrieve::RetrievedContext`** — `blocks: Vec<MemoryBlock>`,
  `query: String`, `elapsed: Duration`, `total_considered: usize`. Serde-
  serialisable for `--json` output.
- **`singularmem_retrieve::MemoryBlock`** — `id`, `content` (full from
  Store), `score`, `score_kind` (from singularmem-search), `source`,
  `tags`, `created_at`. Serde-serialisable. No rank fields, no snippet.
- **`singularmem_retrieve::Adapter`** — trait with `name(&self) -> &str`
  + `format(&self, &RetrievedContext) -> String`. `Send + Sync`. Pure
  function by contract.
- **`singularmem_retrieve::PlainAdapter`** — unit struct implementing
  `Adapter`. Markdown output, deterministic, handles zero-block case.
- **`singularmem_retrieve::testing::MockAdapter`** — gated by
  `feature = "testing"`. Deterministic `MOCK[...]` output for downstream
  test assertions.
- **`singularmem_retrieve::Error`** — variants `Search(#[from] singularmem_search::Error)`,
  `Core(#[from] singularmem_core::Error)`, `EmptyQuery`.
- **Root binary `src/main.rs`** — gains `Command::Retrieve` variant,
  `RetrieveArgs` struct, `cmd_retrieve` function. Reuses (and extracts
  if necessary) the directory-probe + mode-resolution helper from
  `cmd_search`. Adapter registry `fn known_adapters() -> Vec<Box<dyn Adapter>>`.

Composition stays layered: `singularmem-core` knows nothing of search
or retrieval; `singularmem-search` knows nothing of retrieval or
adapters; `singularmem-retrieve` knows nothing of any specific provider;
future `singularmem-adapter-{claude,openai,gemini}` crates know nothing
of one another.

## Data model

**No changes.** Retrieval is read-only. `format_version` in
`docs/formats/store-v1.md` stays `"1"`. No new sidecar directories,
no new SQLite tables.

`MemoryBlock` is a transient runtime type; it never touches disk.

## Interfaces

### CLI

```
singularmem retrieve [OPTIONS] <QUERY>...

Options:
  -a, --adapter <NAME>        Which adapter to use for formatting  [default: plain]
  -l, --limit <N>             Max blocks to include  [default: 10]
      --min-score <F>         Minimum score for a hit to be included  [default: 0.0]
  -m, --mode <MODE>           Underlying search mode (auto|lexical|semantic|hybrid)  [default: auto]
      --fetch-multiplier <N>  Per-ranker overfetch factor (hybrid only)  [default: 3]
      --rrf-k <K>             RRF damping constant (hybrid only)  [default: 60]
      --json                  Emit RetrievedContext as JSON instead of adapter output
      --show-elapsed          Print "Retrieved N blocks in Xms" to stderr after the formatted output
```

**`--mode` / `--fetch-multiplier` / `--rrf-k`** are pass-through to
`HybridSearcher` via `RetrieveOptions.search`. Same defaults as the
`search` verb (auto / 3 / 60).

**`--adapter`** uses a runtime registry. Only `plain` is registered in
3a. Passing an unknown name produces:

```
error: unknown adapter 'claude'; known adapters: plain
```

Exit code 1 (usage error).

**Output streams:**
- Default: adapter-formatted output → stdout (pipe-friendly).
- `--json`: `serde_json::to_writer` of `RetrievedContext` → stdout.
- `--show-elapsed`: timing line → stderr (does not pollute stdout).

**Exit codes** (additive to existing search verb):
- `0` — success (including zero-blocks; zero blocks is not a usage
  error for retrieval, only an empty *query* is).
- `1` — usage error: empty query, unknown adapter, malformed flag.
- `2` — `Error::Search(NoIndexes | HybridMissingIndex | IndexMissing)`
  bubbled through; same as `cmd_search`.

**Auto-wiring** mirrors `cmd_search` exactly: `--mode auto` probes
`<store>.tantivy/` and `<store>.vectors/`, picks the strongest
supported mode, errors `NoIndexes` if neither exists. Explicit modes
fail loudly via the same `HybridMissingIndex` / `IndexMissing` errors.

### Library

`crates/singularmem-retrieve/src/lib.rs` re-exports:

```rust
pub use crate::adapter::{Adapter, PlainAdapter};
pub use crate::error::{Error, Result};
pub use crate::retriever::{MemoryBlock, RetrieveOptions, RetrievedContext, Retriever};

#[cfg(any(test, feature = "testing"))]
pub use crate::testing::MockAdapter;
```

(Order matches rustfmt's lex sort — `RetrievedContext` < `Retriever` because `d` < `r` at the divergent position.)

Public type signatures:

```rust
pub struct Retriever<'a> {
    pub store: &'a Store,
    pub searcher: &'a HybridSearcher<'a>,
}

impl<'a> Retriever<'a> {
    pub const fn new(store: &'a Store, searcher: &'a HybridSearcher<'a>) -> Self;
    pub fn retrieve(&self, query: &str, opts: &RetrieveOptions) -> Result<RetrievedContext>;
}

#[derive(Debug, Clone)]
pub struct RetrieveOptions {
    pub max_blocks: usize,            // default 10
    pub min_score: f32,               // default 0.0
    pub search: HybridSearchOptions,
}
impl Default for RetrieveOptions { /* the defaults above */ }

#[derive(Debug, Clone, serde::Serialize)]
pub struct RetrievedContext {
    pub blocks: Vec<MemoryBlock>,
    pub query: String,
    pub elapsed: std::time::Duration,
    pub total_considered: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MemoryBlock {
    pub id: ItemId,
    pub content: String,
    pub score: f32,
    pub score_kind: ScoreKind,
    pub source: Option<String>,
    pub tags: Vec<String>,
    pub created_at: jiff::Timestamp,
}

pub trait Adapter: Send + Sync {
    /// Stable identifier used in CLI flags + logs.
    /// Lowercase, hyphen-separated. Examples: "plain", "claude", "openai".
    fn name(&self) -> &str;

    /// Render a `RetrievedContext` as a single prompt-ready string.
    /// MUST be a pure function: no I/O, deterministic for identical input.
    fn format(&self, ctx: &RetrievedContext) -> String;
}

pub struct PlainAdapter;
impl Adapter for PlainAdapter { /* see Architecture */ }
```

`Error` in `crates/singularmem-retrieve/src/error.rs`:

```rust
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Underlying search-layer failure.
    #[error("{0}")]
    Search(#[from] singularmem_search::Error),

    /// Underlying core-layer failure (e.g., Store::get on a deleted item).
    #[error("{0}")]
    Core(#[from] singularmem_core::Error),

    /// Query was empty or whitespace-only.
    #[error("query is empty; retrieval requires a non-empty query string")]
    EmptyQuery,
}

pub type Result<T> = std::result::Result<T, Error>;
```

### Wire (MCP / HTTP / IPC)

N/A. Sub-project 4 (MCP server) will expose retrieval over the MCP
protocol; this sub-project ships only the library + CLI.

## Error handling

Per Principle VII, every error names the operation and what was
preserved:

| Error | When | State preserved |
|---|---|---|
| `Error::EmptyQuery` | `retrieve("")` or `retrieve("   ")` | Store untouched. CLI prints message + exits 1. |
| `Error::Search(NoIndexes)` | `--mode auto`, neither sidecar exists | Same as sub-project 2c. Exits 2. |
| `Error::Search(HybridMissingIndex)` | `--mode hybrid`, one sidecar missing | Same as 2c. Exits 2. |
| `Error::Search(IndexMissing)` | `--mode lexical` or `--mode semantic` against missing sidecar | Same as 2c. Exits 2. |
| `Error::Core(NotFound)` | Item referenced by a hit was deleted between search and `Store::get` | Race condition; surface to caller. Exits 1. Caller can retry. |
| Unknown `--adapter` value | CLI argument validation | Exits 1 with message listing known adapter names. |

**No silent fallbacks.** Zero blocks for a valid query is a successful
result (`exit 0`, formatted output says "no memories matched query
'…'"). Only an empty *query* is treated as a usage error.

**No state mutation possible.** Retrieval is read-only. Nothing to
roll back.

## Testing strategy

All tests offline. `SINGULARMEM_TEST_EMBEDDER=mock` selects
`MockEmbedder` from sub-project 2b for any test that needs the
vector sidecar.

### Unit tests (`crates/singularmem-retrieve/src/`)

| Test file | Test | What it pins down |
|---|---|---|
| `retriever.rs` | `retrieve_returns_full_content_not_snippet` | `block.content` equals full ingested string, not Tantivy-trimmed snippet |
| `retriever.rs` | `retrieve_respects_max_blocks` | `max_blocks=3` against 10-hit corpus → exactly 3 blocks |
| `retriever.rs` | `retrieve_filters_below_min_score` | `min_score=0.5` excludes hits at 0.3; survivors all have `score >= 0.5` |
| `retriever.rs` | `retrieve_propagates_search_errors` | Missing index → `Err(Error::Search(_))` |
| `retriever.rs` | `retrieve_propagates_store_get_errors` | Item deleted between search and get → `Err(Error::Core(_))` |
| `retriever.rs` | `empty_query_errors` | `retrieve("")` AND `retrieve("   ")` → `Err(Error::EmptyQuery)` |
| `retriever.rs` | `total_considered_reflects_fusion_input` | `total_considered == HybridSearchResults.total_fused` |
| `adapter.rs` | `plain_adapter_includes_id_score_content` | Format output contains `id:`, `score=`, full content, `---` for each block |
| `adapter.rs` | `plain_adapter_handles_zero_blocks` | Empty context → `[no memories matched query: …]`, no separator |
| `adapter.rs` | `plain_adapter_omits_optional_fields_when_absent` | Block with no `source`/`tags` → no empty `source:`/`tags:` lines |
| `adapter.rs` | `plain_adapter_is_deterministic` | Two calls with same input → byte-identical output |
| `testing.rs` | `mock_adapter_format_includes_ids` | `MOCK[...ids=[id1,id2]...]` for downstream assertion |

Twelve tests.

### CLI integration tests (`tests/cli.rs`)

| Test | Verifies |
|---|---|
| `retrieve_with_default_adapter_emits_plain_format` | Ingest + reindex + `retrieve "fox"` → stdout has `## memory 1` heading |
| `retrieve_json_flag_emits_valid_json` | `--json` output parses; `blocks` array; each block has `id`/`content`/`score`/`score_kind`/`source`/`tags`/`created_at` |
| `retrieve_unknown_adapter_errors` | `--adapter claude` (before 3b lands) → exit 1, stderr lists known adapters |
| `retrieve_empty_query_errors` | `retrieve ""` → exit 1, stderr mentions empty query |
| `retrieve_no_indexes_errors_like_search` | Auto mode, neither sidecar → exit 2, stderr matches search's `no search index exists` |
| `retrieve_mode_hybrid_errors_like_search` | `--mode hybrid` against missing vector → exit 2, `hybrid search requires both indexes` |
| `retrieve_show_elapsed_writes_to_stderr` | `--show-elapsed` puts timing on stderr; stdout stays clean |
| `retrieve_limit_caps_block_count` | `--limit 2` against 10 hits → exactly 2 `## memory` headings |

Eight tests.

### Perf budget

**No new perf budget.** Retrieval cost is bounded by:
- `HybridSearcher::search` — already budgeted at `< 150 ms` (sub-project 2c).
- `N × Store::get` — each ~6 µs (per v0.1.0 bench in project memory).
  Max N is `max_blocks` (default 10) → ~60 µs.
- Adapter `format` — string allocation; microseconds.

A `retrieve_latency` budget would track `hybrid_search_latency + ε`
and be noise-dominated. Adding it later (when token-budget management
or query rewriting introduces real cost) is the right time.

### Offline guarantee

Per Principle VI: every test uses `MockEmbedder` (existing) + `MockAdapter`
(new) — no network. `Adapter::format` is contractually pure. The
`tests-offline` advisory job (still advisory per sub-project 2b's
deferred CI infra work) will pick up the new tests automatically.

## Open questions

None at spec time. Three notes for the implementation plan:

1. **CLI helper extraction.** The directory-probe + mode-resolution
   block in `cmd_search` (sub-project 2c) is duplicated almost verbatim
   in `cmd_retrieve`. The plan should either extract a shared helper
   (e.g., `resolve_search_mode(store_path, args.mode) -> Result<(SearchMode, PathBuf, PathBuf)>`)
   or accept the duplication. Recommendation: extract — DRY is cheap
   here and the new helper has one clear purpose.

2. **`Store::get` returning `Error::NotFound`.** The retriever's
   "deleted between search and get" test (Section 5,
   `retrieve_propagates_store_get_errors`) requires bypassing the
   public Store API to delete an item directly via SQL. The plan
   should call this out as deliberate test scaffolding, not a code
   smell.

3. **CLI registry comment.** The `known_adapters()` function in
   `src/main.rs` must carry a comment pointing sub-projects 3b/3c/3d at
   the two places they need to touch (Cargo.toml dep + this registry
   one-liner). Easy to forget; cheap to document.

## Acceptance criteria

1. New crate `crates/singularmem-retrieve/` exists with `Cargo.toml`
   (version `0.5.0`, workspace-locked), `src/lib.rs`, `src/retriever.rs`,
   `src/adapter.rs`, `src/testing.rs`, `src/error.rs`.
2. Public exports match the Library section above. `MockAdapter` is
   behind `feature = "testing"`.
3. `Retriever::retrieve` implements the algorithm in the Recommended
   Approach section: search → filter by `min_score` → truncate to
   `max_blocks` → fetch full content per hit → return
   `RetrievedContext`. Empty/whitespace query → `Err(Error::EmptyQuery)`.
4. `PlainAdapter::format` produces the Markdown-shaped output
   specified in the Architecture section. Deterministic. Handles
   zero-block input.
5. `MockAdapter::format` produces the `MOCK[query=… blocks=N ids=[…]]`
   shape.
6. CLI gains `singularmem retrieve [OPTIONS] <QUERY>…` with the eight
   flags from the CLI interface section. `--adapter` defaults to
   `plain`; unknown adapter → exit 1 with helpful message.
7. `cmd_retrieve` reuses the same directory-probe + mode-resolution
   logic as `cmd_search` (shared helper recommended). Pre-flight
   failures produce the same exit codes (2 for missing-index variants).
8. `--json` emits `serde_json::to_writer(RetrievedContext)`. Top-level
   keys: `blocks`, `query`, `elapsed`, `total_considered`. Each block
   has `id`, `content`, `score`, `score_kind` (lowercase
   `rrf`/`bm25`/`cos`), `source`, `tags`, `created_at`.
9. All twelve unit tests + eight CLI integration tests from the
   Testing section pass on `ubuntu-latest` and `macos-latest`.
10. No new perf budget; existing `hybrid_search_latency < 150 ms`
    covers retrieval's dominant cost.
11. `docs/formats/store-v1.md` unchanged. `format_version` stays `"1"`.
12. Tagged on merge as `v0.5.0` (additive MINOR bump per Principle V).

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | No new network calls. Adapter trait requires pure functions; PlainAdapter and MockAdapter are pure formatting. |
| **II — Provider-Agnostic by Contract** | Establishes the typed adapter contract. PlainAdapter satisfies the "one fully local runtime" requirement. Sub-projects 3b/3c/3d add the three remaining required providers. Removing PlainAdapter from the build is well-defined to test via the registry pattern. |
| **III — Open Core with a Stable Boundary** | III.a: pure additive surface (new crate, new trait, new CLI verb). Nothing removed. III.b: no on-disk changes; `format_version` unchanged. |
| **IV — CLI-First** | `singularmem retrieve` verb covers the operation. JSON output (`--json`) is scriptable. `--show-elapsed` keeps stdout clean for piping. |
| **V — Composable Library Architecture** | `Retriever<'a>` borrows references to existing components (`Store`, `HybridSearcher`) — no ownership transfer. Adapter trait is the explicit extension point. New crate keeps layers clean (core ← search ← retrieve ← adapters). |
| **VI — Deterministic and Offline-Testable** | All tests use `MockEmbedder` (existing) + `MockAdapter` (new). No network. `Adapter::format` is contractually deterministic and pure. |
| **VII — Honest Failure Modes** | `EmptyQuery` is a typed error, not silent zero-blocks. `Search` and `Core` errors propagate through retrieval with full context. CLI exit codes mirror search's (1 for usage, 2 for missing-index). No silent degradation. |
| **VIII — Privacy Telemetry** | No telemetry added. `tracing::info!` in cmd_retrieve only logs elapsed time + block count; no query text or item content. |
| **IX — Accessible by Default** | CLI uses ASCII; `---` separator avoids the em-dash issue from sub-project 2c. `--json` provides programmatic access for screen readers + tooling. |
| **X — Performance Budgets, Enforced in CI** | No new budget. Retrieval bounded by existing `hybrid_search_latency` (150 ms) + ~60 µs of point-reads; well within headroom. |
