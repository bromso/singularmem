# TypeScript SDK — Search + Retrieve Bindings (Sub-project 5b) — Design Spec

**Date:** 2026-05-19
**Status:** Approved (pending user review of written spec)
**Sub-project:** 5b (second of three TS SDK sub-projects)
**Builds on:** 5a (foundation + reads), spec at `docs/superpowers/specs/2026-05-18-ts-sdk-5a-design.md`

## Summary

Extends the `singularmem-node` napi binding (introduced in 5a) with the search and retrieve halves of the API surface. JS consumers get `store.search(query, options?)` (hybrid + lexical + semantic ranking with full Item content per hit), `store.retrieve(query, options?)` (search + fetch + structured blocks), and a frozen `adapters.{plain,claude,openai,gemini}` namespace whose entries each expose a `format(ctx) → string` method matching the four constitutional Principle II providers. Custom adapters are deferred; writes are 5c.

The spec mirrors the Rust crate split (`singularmem-search` → search, `singularmem-retrieve` → retrieve+adapter trait, four adapter crates → concrete formatters). The five method choices stay consistent with the Rust CLI's mental model, so docs and examples can largely be ported across the language boundary.

## Motivation

5a shipped enough to read items by ID, list them by tag, and walk supersession chains. That covers maintenance tasks. What it does not cover: the *primary* use case for an LLM memory layer — given a natural-language query, find the most relevant memories and format them for a model's context window. That requires search (hybrid retrieval) and retrieve (search + fetch + adapter-formatted output).

The Rust CLI and MCP server both ship this surface already (sub-projects 2c and 4a/b respectively). 5b brings it to JavaScript and TypeScript consumers in the same shape, completing the read-side parity between the three top-level surfaces (CLI, MCP, TS SDK).

## Section 1 — Dependency changes + JS API surface overview

New dependencies in `crates/singularmem-node/Cargo.toml`:

```toml
[dependencies]
singularmem-core = { path = "../singularmem-core" }
singularmem-search = { path = "../singularmem-search" }      # NEW
singularmem-retrieve = { path = "../singularmem-retrieve" }  # NEW
singularmem-adapter-claude = { path = "../singularmem-adapter-claude" }  # NEW
singularmem-adapter-openai = { path = "../singularmem-adapter-openai" }  # NEW
singularmem-adapter-gemini = { path = "../singularmem-adapter-gemini" }  # NEW
# napi + napi-derive + tokio + serde_json stay
```

Same dependency footprint as the root `singularmem` CLI binary.

New JS API surface (5a methods unchanged):

```typescript
// Two new Store methods
store.search(query: string, options?: SearchOptions): Promise<SearchResults>;
store.retrieve(query: string, options?: RetrieveOptions): Promise<RetrievedContext>;

// New top-level export
export const adapters: {
  readonly plain:  Adapter;
  readonly claude: Adapter;
  readonly openai: Adapter;
  readonly gemini: Adapter;
};

// New types
export type SearchMode = 'auto' | 'lexical' | 'semantic' | 'hybrid';
export type ScoreKind = 'rrf' | 'bm25' | 'cosine';
export interface SearchOptions  { mode?, limit?, fetchMultiplier?, rrfK? }
export interface SearchHit      { item, score, kind, bm25?, cosine? }
export interface SearchResults  { query, hits }
export interface RetrieveOptions { mode?, limit?, fetchMultiplier?, rrfK?, minScore? }
export interface MemoryBlock    { item, score, kind }
export interface RetrievedContext { query, blocks }
export interface Adapter        { readonly name: 'plain'|'claude'|'openai'|'gemini'; format(ctx: RetrievedContext): string }
```

**Decisions locked in this section:**

- Mirror the Rust crate split. `store.search()` returns raw scored hits (with full Item). `store.retrieve()` returns structured `RetrievedContext`. `adapters.X.format(ctx)` turns the context into a formatted string. Composable; matches CLI + MCP mental model.
- Search hits always carry the full Item content (one round-trip; rarely useful to have just IDs in JS).
- Adapters exposed as a frozen `adapters` namespace object, four entries. No JS-side custom adapters in 5b (callers can write their own JS format function over a `RetrievedContext` if they want — no Rust plumbing needed).
- `HybridSearcher` is *not* exposed as a JS class — it's an implementation detail behind `store.search()`. Same for `Retriever`.

What's NOT in 5b: writes (5c), custom adapters, direct `HybridSearcher` exposure, multi-platform prebuilts (6), Flutter GUI (7+).

## Section 2 — TypeScript types

The full TS surface added in 5b.

### Search

```typescript
/**
 * Search mode. 'auto' probes for available sidecar indexes and degrades:
 * - Both Tantivy + USearch present → hybrid (RRF fusion)
 * - Only Tantivy → lexical
 * - Only USearch → semantic
 * - Neither → error with code: 'NoIndexes'
 *
 * Explicit modes fail loudly if the required sidecar is missing.
 */
export type SearchMode = 'auto' | 'lexical' | 'semantic' | 'hybrid';

export type ScoreKind = 'rrf' | 'bm25' | 'cosine';

export interface SearchOptions {
  /** Default: 'auto' */
  mode?: SearchMode;
  /** Default: 10 */
  limit?: number;
  /** Default: 3. Per-ranker fetch depth = limit × fetchMultiplier before RRF fusion. */
  fetchMultiplier?: number;
  /** Default: 60. RRF constant (Cormack et al. 2009). */
  rrfK?: number;
}

export interface SearchHit {
  /** The matched item (always populated; not lazy-loaded). */
  item: Item;
  /** Final score after fusion (RRF) or single-ranker (BM25 / cosine). */
  score: number;
  /** Which ranker produced the score. */
  kind: ScoreKind;
  /** Component BM25 score, present only when kind === 'rrf'. */
  bm25?: number;
  /** Component cosine score, present only when kind === 'rrf'. */
  cosine?: number;
}

export interface SearchResults {
  query: string;
  hits: SearchHit[];
}
```

### Retrieve

```typescript
export interface RetrieveOptions {
  /** Default: 'auto' */
  mode?: SearchMode;
  /** Default: 10 */
  limit?: number;
  /** Default: 3 */
  fetchMultiplier?: number;
  /** Default: 60 */
  rrfK?: number;
  /** Default: 0. Drop blocks whose score is below this threshold. */
  minScore?: number;
}

export interface MemoryBlock {
  item: Item;
  score: number;
  kind: ScoreKind;
}

export interface RetrievedContext {
  query: string;
  blocks: MemoryBlock[];
}
```

### Adapters

```typescript
export interface Adapter {
  readonly name: 'plain' | 'claude' | 'openai' | 'gemini';
  format(ctx: RetrievedContext): string;
}

export const adapters: {
  readonly plain:  Adapter;
  readonly claude: Adapter;
  readonly openai: Adapter;
  readonly gemini: Adapter;
};
```

### Store (additions only)

```typescript
export class Store {
  // ... 5a methods unchanged ...
  search(query: string, options?: SearchOptions): Promise<SearchResults>;
  retrieve(query: string, options?: RetrieveOptions): Promise<RetrievedContext>;
}
```

**Decisions locked in this section:**

- `SearchMode` and `ScoreKind` are TS string-literal unions (not enums). Matches the rest of the surface, including `error.code`.
- `bm25` and `cosine` on `SearchHit` are optional, present only when `kind === 'rrf'`. Single-ranker hits omit them entirely.
- `MemoryBlock` and `SearchHit` are intentionally separate types even with significant overlap. The semantic difference (search result vs retrieved-context block) is real; mixing them would obscure intent.
- `Adapter` is a structural interface, not a class. Consumers never construct adapters — they use the four pre-built ones in the `adapters` namespace.
- `adapters` is a module-level constant, not a static on `Store`. Pure-function helpers live as module exports.
- All four `adapters` entries are frozen at module init; `name` is readonly.

## Section 3 — Rust ↔ TypeScript type mappings

### Search types

| Rust (`singularmem-search`) | TypeScript | Conversion |
|---|---|---|
| `SearchMode::{Auto, Lexical, Semantic, Hybrid}` | `'auto' \| 'lexical' \| 'semantic' \| 'hybrid'` | Case-sensitive match in napi handler; unknown → `code: "Validation"` |
| `ScoreKind::{Rrf, Bm25, Cosine}` | `'rrf' \| 'bm25' \| 'cosine'` | Static lowercase map in the `From<HybridHit>` conversion |
| `HybridSearchOptions { mode, limit, fetch_multiplier, rrf_k }` | `SearchOptions` | Defaults applied in handler when fields are undefined |
| `HybridHit { item_id, score, kind, bm25_score, cosine_score }` | `SearchHit { item, score, kind, bm25?, cosine? }` | Binding does `store.get(item_id)?` to enrich; `Option<f32>` → `Option<f64>` |
| `HybridSearchResults { query, hits }` | `SearchResults { query, hits }` | Direct |

### Retrieve types

| Rust (`singularmem-retrieve`) | TypeScript | Conversion |
|---|---|---|
| `RetrieveOptions { limit, min_score, mode, fetch_multiplier, rrf_k }` | `RetrieveOptions` | Defaults applied in handler |
| `MemoryBlock { item, score, kind }` | `MemoryBlock { item, score, kind }` | Direct |
| `RetrievedContext { query, blocks }` | `RetrievedContext { query, blocks }` | Direct |

### Adapters

| Rust | TypeScript | Conversion |
|---|---|---|
| `PlainAdapter` (unit struct, `Adapter` trait impl) | `adapters.plain` (frozen object: `name` + `format`) | Each adapter wrapped as a `#[napi]` class with a sync `format(ctx)` method; JS-side wrapper exposes them as a frozen `adapters` namespace |
| `ClaudeAdapter`, `OpenAiAdapter`, `GeminiAdapter` | `adapters.claude`, `adapters.openai`, `adapters.gemini` | Same pattern |

### Score precision

Rust scores are `f32`. JS `number` is `f64`. Conversion is lossless (every `f32` value fits exactly in `f64`). All score fields in the TS interfaces are `number`.

### Error mapping (new in 5b)

The full `singularmem_search::Error` enum (verified against `crates/singularmem-search/src/error.rs`):

| Rust variant | JS `.code` | Source |
|---|---|---|
| `Tantivy { ... }` | `"Tantivy"` | Tantivy-specific errors |
| `QueryParse(String)` | `"QueryParse"` | Tantivy query syntax error |
| `IndexMissing { ... }` | `"IndexMissing"` | Explicit lexical/semantic mode with required sidecar absent |
| `IndexCorrupted { ... }` | `"IndexCorrupted"` | Sidecar exists but is unreadable |
| `Io(std::io::Error)` | `"Io"` | Filesystem error (matches 5a's `Io` code) |
| `Embedding { ... }` | `"Embedding"` | Embedder runtime error |
| `ModelDownload { ... }` | `"ModelDownload"` | fastembed model download failure |
| `InvalidModelFiles { ... }` | `"InvalidModelFiles"` | Embedder model files malformed |
| `DimMismatch { ... }` | `"DimMismatch"` | Vector dimension mismatch on USearch insert |
| `ModelMismatch { ... }` | `"ModelMismatch"` | Sidecar built with different embedder model |
| `Usearch { ... }` | `"Usearch"` | USearch-specific errors |
| `NoIndexes` | `"NoIndexes"` | Auto mode with no sidecars present |
| `HybridMissingIndex { ... }` | `"HybridMissingIndex"` | Explicit `mode: 'hybrid'` with one sidecar absent |

The `singularmem_retrieve::Error` enum:

| Rust variant | JS `.code` | Source |
|---|---|---|
| `EmptyQuery` | `"EmptyQuery"` | Empty/whitespace-only query string |
| `Search(inner)` | (inner code) | Unwrap to the wrapped search error's `.code` |
| `Core(inner)` | (inner code) | Unwrap to the wrapped core error's `.code` |

The unwrap behavior means the final JS error carries the *innermost* meaningful code. `retrieve::Error::Search(NoIndexes)` surfaces as `code: "NoIndexes"`, not `code: "Search"`.

### napi-rs annotation patterns

- `Store::search` and `Store::retrieve` use the Task pattern (same as 5a's 5 methods). One Task struct each (`SearchTask`, `RetrieveTask`), three fields (`failed: Option<NodeError>` + inputs).
- The four adapter classes are `#[napi]` with a synchronous `format(&self, ctx: RetrievedContext) -> String` method. Sync because formatting is pure CPU work, microsecond-scale; no need for the Task pattern's overhead.
- `RetrievedContext`, `MemoryBlock`, `SearchHit`, `SearchResults` are `#[napi(object)]` (POJO-style), matching 5a's `Item` handling.
- JS wrapper class in `scripts/patch-index.js` gains `search` and `retrieve` methods that call `_native.search/retrieve` and lift nested `Item` instances via `liftItem`.

## Section 4 — Index lifecycle, options resolution, error flow

### Index discovery — per-call probing, no caching

`Store` wraps `Arc<CoreStore>`. Tantivy + USearch sidecars are *separate* from the SQLite store — they live as `<store-path>.tantivy/` and `<store-path>.vectors/` sibling directories.

`Store::search` and `Store::retrieve` each:

1. Read the store path from the napi `Store` wrapper (path is stored alongside the inner CoreStore — see below)
2. Compute the sidecar paths
3. Probe for existence in the Task's `compute` (libuv thread, not JS thread)
4. Resolve the search mode (auto → degrade; explicit → error if missing)
5. Open the indexes via `singularmem_search::Index::open(...)` and/or `EmbedderIndex::open(...)`
6. Construct the `HybridSearcher` and run the query
7. For each `HybridHit`, do `store.get(item_id)` to enrich into `SearchHit`
8. Return the structured result

**No caching in 5b.** Each call opens indexes fresh. Rationale:
- Matches the CLI's `cmd_search` behavior exactly — proven, perf-budget verified under 150ms hybrid
- Avoids the cache-invalidation question (mid-process `reindex`)
- Tantivy/USearch `Index::open` is mmap-based — cheap after first OS page-cache warmup
- Cache layer can be added later as a transparent optimization if benchmarks show it's needed

### Store-path storage

`singularmem_core::Store` doesn't have a public `path()` accessor. The 5b binding adds the path to the napi `Store` wrapper:

```rust
pub struct Store {
    pub(crate) inner: Arc<CoreStore>,
    pub(crate) path: PathBuf,  // NEW in 5b
}
```

`Store::open` already takes a `String` path; save it before opening. **Zero core changes.** This keeps the change scoped to the binding crate.

### Options resolution + defaults

Defaults applied in the napi handlers, matching the Rust `Default` impls:

| Field | Default |
|---|---|
| `mode` | `'auto'` |
| `limit` | 10 |
| `fetchMultiplier` | 3 |
| `rrfK` | 60 |
| `minScore` (retrieve only) | 0.0 |

Invalid mode string → `code: "Validation"`. Negative or zero `limit` → `code: "Validation"`. Negative `fetchMultiplier` or `rrfK` → `code: "Validation"`.

### Empty query handling

- `Store::search('')` → returns `{ query: '', hits: [] }` with no error. Matches `HybridSearcher` behavior (no empty-query check in search).
- `Store::retrieve('')` → rejects with `code: "EmptyQuery"`. Matches `singularmem_retrieve::Retriever::retrieve` behavior (has the check upstream).

### Adapter format — synchronous

Adapter formatting is pure CPU, no I/O. The four adapter `#[napi]` classes expose `format(&self, ctx: RetrievedContext) -> String` as a synchronous method (no Task), avoiding the Task pattern's per-call overhead.

```typescript
const ctx = await store.retrieve('cat care', { limit: 5 });
const formatted = adapters.claude.format(ctx);  // synchronous, returns string
```

### JS wrapper class updates (`scripts/patch-index.js`)

Two new methods join the existing `open`/`get`/`list`/`revisions`/`formatVersion`/`export`:

```javascript
search(query, options) {
  return this._native.search(query, options).then((res) => ({
    query: res.query,
    hits: res.hits.map((h) => ({ ...h, item: liftItem(h.item) })),
  }))
}

retrieve(query, options) {
  return this._native.retrieve(query, options).then((ctx) => ({
    query: ctx.query,
    blocks: ctx.blocks.map((b) => ({ ...b, item: liftItem(b.item) })),
  }))
}
```

The `adapters` object passes through from the native binding directly — no `liftItem` needed (adapter output is `string`).

### Test seeding for indexed stores

Search and retrieve need Tantivy + USearch sidecars to exercise. The 5a `helpers.mjs::seedStore` uses `singularmem ingest` which auto-wires sidecars *only if they exist*.

New helper `seedStoreWithIndexes(path, items)`:

```javascript
export function seedStoreWithIndexes(path, items) {
  const reindex = spawnSync(
    'cargo',
    ['run', '-q', '-p', 'singularmem', '--', 'reindex', '--with-embeddings', '--store', path],
    { stdio: 'pipe', encoding: 'utf8' },
  );
  if (reindex.status !== 0) {
    throw new Error(`reindex failed: ${reindex.stderr}`);
  }
  seedStore(path, items);  // sidecars now exist, ingest will auto-wire them
}
```

Mirrors the pattern from `crates/singularmem-mcp/tests/full_write_read_cycle.rs`.

## Section 5 — Testing strategy

Three layers (consistent with 5a).

### Layer 1: Rust unit tests (`src/*.rs`)

Standard `#[cfg(test)] mod tests` blocks. Run via `cargo test -p singularmem-node`. No Node required.

**New tests in `src/types.rs`:**
- `score_kind_to_string_lowercase`
- `search_mode_from_string_valid`, `search_mode_from_string_invalid`
- `hybrid_hit_to_search_hit_passes_component_scores`
- `hybrid_hit_to_search_hit_omits_components_for_single_ranker`
- `memory_block_round_trips`
- `retrieved_context_round_trips`

**New tests in `src/error.rs`:**
- `no_indexes_maps_to_code`
- `hybrid_missing_index_maps_to_code`
- `index_missing_maps_to_code`
- `tantivy_error_maps_to_code`
- `usearch_error_maps_to_code`
- `empty_query_maps_to_code`
- `wrapped_search_error_unwraps_to_inner_code`
- `wrapped_core_error_unwraps_to_inner_code`

**New tests for the four adapter glue classes:**
- `plain_adapter_format_empty` + `plain_adapter_format_populated`
- `claude_adapter_emits_documents_wrapper`
- `openai_adapter_emits_bracket_citations`
- `gemini_adapter_emits_em_dash_headers`

Total new Rust unit tests: ~18.

### Layer 2: Node integration tests (`test/*.test.mjs`)

End-to-end via `node --test` + `node:assert/strict`.

**New helper:** `seedStoreWithIndexes(path, items)` in `test/helpers.mjs` (per Section 4).

**`test/store_search.test.mjs`** (~6 tests):
- returns hits with full Item content
- explicit `mode: 'lexical'` works (Tantivy-only store)
- explicit `mode: 'semantic'` works
- explicit `mode: 'hybrid'` returns RRF-fused results with `bm25` + `cosine` populated
- `mode: 'auto'` on store with no indexes → `code: 'NoIndexes'`
- `mode: 'hybrid'` on store missing one sidecar → `code: 'HybridMissingIndex'`

**`test/store_retrieve.test.mjs`** (~5 tests):
- returns `RetrievedContext` with `query` + `blocks`
- blocks have `item.createdAt instanceof Date` (lift correctness)
- `minScore` filter drops low-scoring blocks
- empty query → `code: 'EmptyQuery'`
- explicit `mode: 'lexical'` works (mode passthrough)

**`test/adapters.test.mjs`** (~6 tests):
- `adapters.plain.format(ctx)` returns non-empty Markdown
- `adapters.claude.format(ctx)` contains `<documents>` and `<document index=`
- `adapters.openai.format(ctx)` contains `[1]` and the leading instruction line
- `adapters.gemini.format(ctx)` contains `Source 1` and the em-dash separator
- `adapters.X.name` returns the expected string for all four
- Empty `RetrievedContext` → each adapter returns its empty-state output

Total new Node tests: ~17.

### Layer 3: TypeScript type-level smoke test (`test/types.test.mts`)

Extends 5a's smoke test. Asserts all new types compile and method signatures match.

```typescript
import {
  Store,
  adapters,
  type Item,
  type SearchOptions,
  type SearchHit,
  type SearchResults,
  type SearchMode,
  type ScoreKind,
  type RetrieveOptions,
  type MemoryBlock,
  type RetrievedContext,
  type Adapter,
} from '../index.js';

const searchOpts: SearchOptions = { mode: 'hybrid', limit: 5, fetchMultiplier: 3, rrfK: 60 };
const retrieveOpts: RetrieveOptions = { minScore: 0.5, mode: 'auto' };

declare const hit: SearchHit;
const _hitItem: Item = hit.item;
const _hitScore: number = hit.score;
const _hitKind: ScoreKind = hit.kind;
const _hitBm25: number | undefined = hit.bm25;
const _hitCosine: number | undefined = hit.cosine;

declare const ctx: RetrievedContext;
const _ctxQuery: string = ctx.query;
const _ctxBlocks: MemoryBlock[] = ctx.blocks;

const _adapterName: 'plain' | 'claude' | 'openai' | 'gemini' = adapters.claude.name;
const _formatted: string = adapters.claude.format(ctx);

async function _check(s: Store): Promise<void> {
  const _sr: SearchResults = await s.search('q');
  const _sr2: SearchResults = await s.search('q', { mode: 'hybrid' });
  const _rc: RetrievedContext = await s.retrieve('q');
  const _rc2: RetrievedContext = await s.retrieve('q', { minScore: 0.5 });
}

function _exhaustive(m: SearchMode): string {
  switch (m) {
    case 'auto': return 'a';
    case 'lexical': return 'l';
    case 'semantic': return 's';
    case 'hybrid': return 'h';
  }
}
```

### CI

The existing `node-bindings` job (added in 5a) needs no changes — the new test files match the existing `test/*.test.mjs` glob. The job will pick them up automatically.

Implicit dependency: CI must run `singularmem reindex --with-embeddings` which downloads the fastembed model. Already exercised by `crates/singularmem-mcp/tests/full_write_read_cycle.rs`, so the model is presumably cached between runs. If cold-CI model download is slow, the 5b tests share that cost — acceptable.

### Test counts after 5b lands

- Rust unit tests: 15 (5a) + ~18 (5b) = ~33
- Node integration tests: 17 (5a) + ~17 (5b) = ~34
- TS smoke test: 1 file (extended)

## Section 6 — Acceptance criteria + Constitution Check

### Acceptance criteria

A reviewer can sign off on 5b iff all of these are true.

**Dependencies:**
- `crates/singularmem-node/Cargo.toml` depends on `singularmem-search`, `singularmem-retrieve`, and the three cloud-adapter crates
- No `singularmem-core` API changes (the `Store.path` field is binding-internal)

**JS API surface (matches Section 2 exactly):**
- `store.search(query, options?): Promise<SearchResults>`
- `store.retrieve(query, options?): Promise<RetrievedContext>`
- `adapters` top-level export with `plain`, `claude`, `openai`, `gemini`, each having `name` (readonly literal) + `format(ctx) → string`
- All new interfaces match Section 2 verbatim

**Type mappings (per Section 3):**
- `SearchMode` and `ScoreKind` are string-literal unions, validated case-sensitively; invalid → `code: "Validation"`
- `f32` → `f64` lossless
- `bm25`/`cosine` populate only when `kind === 'rrf'`
- Wrapped errors unwrap to innermost `.code`

**Index lifecycle (per Section 4):**
- Sidecar paths computed from `Store.path` (binding-internal field)
- Each call opens Tantivy + USearch fresh in the Task's `compute` (no caching)
- Auto-mode probes for sidecars and degrades; explicit modes fail with the right `.code`

**Behavior:**
- `store.search('')` returns `{ query: '', hits: [] }` with no error
- `store.retrieve('')` rejects with `code: "EmptyQuery"`
- All `Item` instances in results have `createdAt instanceof Date` (lift wrapper)
- Each adapter routes through to the right Rust `Adapter::format` (5b doesn't re-test adapter formatting; just confirms the JS surface routes correctly)

**Tests:**
- ~33 Rust unit tests pass
- ~34 Node integration tests pass
- TS smoke test compiles
- All three layers blocking in the existing `node-bindings` CI job
- Version-drift check passes (no version change in 5b; bump post-merge)

### Out of scope

- Writes (5c)
- Custom JS adapters
- Direct `HybridSearcher` exposure as a JS class
- Caching of opened indexes
- Multi-platform prebuilt binaries (6)
- GUI work (7+)

### Constitution Check (v0.2.0)

- **I. Local-first, file-based** ✅ — same SQLite + Tantivy + USearch sidecars; no network. Identical to CLI behavior.
- **II. Provider-agnostic** ✅ — all four adapters ship and are equally reachable. Removing one is a colocated three-edit change (Cargo.toml dep + glue class + namespace entry).
- **III. Append-only, revisable** ✅ — read-only; no ingest changes.
- **IV. Open core** ✅ — Apache-2.0; lives in `crates/`. Four adapter crates remain independently removable.
- **V. Stable on-disk format** ✅ — no format changes; `formatVersion` stays `"1"`.
- **VI. Single binary, zero deps** — partial / acceptable (same trade-off as 5a; Node 20.12+ for consumers).
- **VII. Composable crates** ✅ — depends on all five upstream crates the CLI uses, in the same direction. No bypass of upstream APIs.
- **VIII. Tested at every layer** ✅ — three test layers, all blocking in CI.
- **IX. Documented behavior** ✅ — JSDoc on every new `#[napi]` item; README extended with search + retrieve + adapters sections.
- **X. Performance budgets** — N/A for the binding itself. `hybrid_search_latency < 150ms` enforced upstream. Napi overhead per call is sub-millisecond.

### Open items (deferred, not blocking)

- **Empty-query search behavior** — currently returns empty results, not an error. Matches `HybridSearcher`. Could tighten in a future minor.
- **Format dispatcher** — no `formatRetrieved(adapterName, ctx)` convenience. `adapters.X.format()` suffices; add later if needed.
- **Batched search/retrieve** — single query in, single result out. YAGNI for v0.

## Next steps after this spec is approved

1. Run writing-plans skill to produce `docs/superpowers/plans/2026-05-19-ts-sdk-5b.md`
2. Execute via subagent-driven-development
3. PR, merge, version bump to 0.12.0, tag
4. Brainstorm sub-project 5c (writes)
