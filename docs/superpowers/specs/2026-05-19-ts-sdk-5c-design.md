# TypeScript SDK — Write Support (Sub-project 5c) — Design Spec

**Date:** 2026-05-19
**Status:** Approved (pending user review of written spec)
**Sub-project:** 5c (third and final of three TS SDK sub-projects)
**Builds on:** 5a (foundation + reads, `v0.11.0`), 5b (search + retrieve + adapters, `v0.12.0`)

## Summary

Adds `store.ingest(item)` to the napi binding so JS consumers can write items to a Singularmem store in-process. Introduces a `NewItem` interface mirroring the Rust `singularmem_core::NewItem` shape (one required field, four optional). Extends `Store.open` to auto-wire Tantivy + USearch indexes as hooks on the inner CoreStore when sidecars exist, so ingested items become searchable through the existing `store.search` / `store.retrieve` paths without callers having to run `singularmem reindex` themselves.

After 5c merges, the JS SDK reaches feature parity with the CLI's verb set for v0. Multi-platform prebuilt binaries and npm publish workflow follow in sub-project 6.

## Motivation

5a and 5b shipped reads + search + retrieve. JS consumers can currently observe a store but can't write to it without spawning the CLI or running the MCP server. Closing this gap is the last step before TS-driven applications can fully manage their own memory layer in-process — which is the original purpose of the SDK.

The Rust `Store::ingest` API is small (one method, one input type) and the napi pattern is well-established from the previous two sub-projects. The complexity in 5c is concentrated in the hook auto-wiring on `Store.open`, which is the first time the JS SDK opens a store with hooks attached.

## Section 1 — API surface + TS `NewItem` shape

One new method, one new input type:

```typescript
export interface NewItem {
  /** Required: UTF-8 text content. Must be non-empty, ≤ 1 MiB. */
  content: string;
  /** Optional: ULID of the item this supersedes (revision chain). */
  supersedes?: string;
  /** Optional: tags to attach. Default: `[]`. Duplicates are silently deduped. */
  tags?: string[];
  /** Optional: free-form provenance label, ≤ 256 bytes. */
  source?: string;
  /** Optional: arbitrary JSON object. Default: `{}`. */
  metadata?: Record<string, unknown>;
}

export class Store {
  // ...5a + 5b methods unchanged...
  ingest(item: NewItem): Promise<Item>;
}
```

**Decisions locked in this section:**

- `NewItem` is an `#[napi(object)]` POJO (not a class). Constructed with JS object literals.
- Only `content` is required. Missing `tags` becomes `Vec::new()`; missing `metadata` becomes `serde_json::json!({})`.
- `store.ingest()` returns the newly-persisted `Item` with the store-assigned `id` + `createdAt`. Routed through the existing JS wrapper class so `createdAt instanceof Date`.
- No `ingestMany` batch method. YAGNI for v0.
- No JS hook registration. Auto-wiring of internal Tantivy + USearch hooks is transparent (Section 2).

## Section 2 — Auto-wiring strategy

`Store.ingest()` only writes to Tantivy + USearch if the inner CoreStore has hooks attached. 5a/5b's `Store.open` opens without hooks; 5c extends `OpenStoreTask::compute` to probe sidecars and attach hooks via `Store::set_hook`.

### Open-time auto-wiring flow

In `OpenStoreTask::compute`, AFTER `CoreStore::open_with_options(&path, CoreStoreOptions { read_only })`:

```rust
let mut store = CoreStore::open_with_options(&self.path, options)?;

// Auto-wire hooks ONLY when the store is read-write. Read-only stores
// reject writes at the SQLite layer; hooks would be unused.
if !read_only {
    // Reuse the existing `open_sidecars` helper from 5b — it already does
    // the OsString-append path derivation, sidecar existence probing, and
    // env-var-aware embedder construction (mock vs fastembed).
    let (tantivy, vectors) = match open_sidecars(&self.path) {
        Ok(pair) => pair,
        Err(e) => {
            // Per Principle VII: degraded open is better than failed open.
            // Log + continue with no hooks. Ingest will write SQLite only.
            tracing::warn!(error = ?e, "sidecar probe failed during Store.open; ingest will skip hooks");
            (None, None)
        }
    };

    let mut hooks: Vec<Box<dyn IndexHook>> = Vec::new();
    if let Some(idx) = tantivy {
        hooks.push(Box::new(idx));
    }
    if let Some(idx) = vectors {
        hooks.push(Box::new(idx));
    }
    if !hooks.is_empty() {
        store.set_hook(Some(Box::new(singularmem_core::hook::MultiHook::new(hooks))));
    }
}

Ok(Arc::new(store))
```

Caveat: `open_sidecars` currently returns `Err` if any individual `Index::open` / `EmbedderIndex::open` fails. For 5c's auto-wiring we want degraded-open semantics (one sidecar failing should not prevent the store from opening — the working sidecar still attaches as a hook). Two options:

1. **Refactor `open_sidecars`** to return `(Option<Index>, Option<EmbedderIndex>)` always-Ok, logging individual failures inline. This changes the semantics for search/retrieve too, but in a beneficial way (they currently return error on partial failures; degrading to `None` makes them fall back to the available ranker).
2. **Add a second helper** `open_sidecars_lenient(&path)` that handles open failures non-fatally, used only by the auto-wiring path. `open_sidecars` keeps its current strict semantics for search/retrieve.

Spec recommendation: **option 1**. The strict-vs-lenient distinction is artificial; the consumer always wants degraded behavior. The few lines of error-mapping change in search/retrieve are minor and align with the "no failed opens" principle.

**Decisions locked in this section:**

- **Read-only stores skip auto-wiring entirely.** Hooks would be unused.
- **`set_hook` after `open_with_options`** — reuses the existing core API, no `singularmem-core` refactor needed.
- **Failed sidecar opens are logged + skipped, not fatal.** Better degraded ingest than failed open. Matches CLI behavior.
- **`MultiHook` wrapping** even for a single hook, for uniformity.
- **Hooks live for the Store handle's lifetime.** Indexes are mmap'd; the JS wrapper holding `Arc<CoreStore>` keeps them alive.
- **Reads keep their per-call probing.** No coordination with the new hook handles. Future optimization: share via `Arc`. Out of scope for 5c.
- **`open_sidecars` from 5b is reused** — it already probes both sidecars, builds the embedder per env var, and returns `(Option<Index>, Option<EmbedderIndex>)`. The hook auto-wiring just rewraps the returned indexes as `Box<dyn IndexHook>` and assembles a `MultiHook`. One small refactor: change `open_sidecars` to handle individual `Index::open` failures non-fatally (log + return `None` for the failing sidecar) instead of bubbling. This benefits search/retrieve too (graceful degradation when one sidecar is corrupted).

### Second-Store-open semantics

Multiple JS processes (or a JS process with two `Store.open` calls on the same path) each independently probe sidecars and attach their own hooks. Writer contention is handled by SQLite's busy-timeout retry + Tantivy's writer locks. Same as running the CLI twice in parallel. Not a 5c concern.

## Section 3 — Error handling, validation, supersession

### Errors reuse the 5a mapping

No new error variants in 5c. All ingest-related errors already map through `NodeError → napi::Error<&'static str>`:

| Trigger | `.code` | Source |
|---|---|---|
| Empty `content` | `Validation` | `Error::Validation { field: "content", reason: "empty" }` |
| `content` > 1 MiB | `Validation` | core size check |
| Single tag > 64 bytes | `Validation` | core tag-size check |
| `source` > 256 bytes | `Validation` | core source-size check |
| `metadata` is not a JSON object | `Validation` | core metadata-shape check |
| `supersedes` is not a valid ULID | `InvalidId` | binding-layer ULID parse |
| `supersedes` references missing ID | `SupersedesNotFound` | core supersedes-existence check |
| Read-only store | `ReadOnly` | `Error::ReadOnly { operation: "ingest" }` |
| SQLite write fails | `Sqlite` | underlying rusqlite error |
| Hook write fails (Tantivy/USearch error during ingest) | _(hidden)_ | logged via `tracing::warn!` per Principle VII; ingest still succeeds |

### Implementation pattern

`store.ingest(item)` follows the established Task pattern (`IngestTask`):

1. **`Store::ingest` (synchronous wrapper)**: takes JS `NewItem`, validates the ULID format of `supersedes` (if present and non-empty). On parse failure, set `pre_error: Some(InvalidId)` so the Promise rejects via the established `coded_error_to_napi_raw` path.

2. **`IngestTask::compute` (libuv thread)**: constructs a `singularmem_core::NewItem` from the JS-sent fields and calls `inner.ingest(new_item)`. Core does all other validation. On error, store the `NodeError` in `self.failed` for the reject callback.

### `NewItem` conversion helper

```rust
fn js_new_item_to_core(
    item: crate::types::NewItem,
) -> Result<singularmem_core::NewItem, NapiError<&'static str>> {
    use std::str::FromStr;
    let supersedes = match item.supersedes.as_deref() {
        Some(s) if !s.is_empty() => Some(
            singularmem_core::item::ItemId::from_str(s)
                .map_err(|e| NodeError::from(singularmem_core::Error::from(e)).into())?,
        ),
        _ => None,
    };
    Ok(singularmem_core::NewItem {
        content: item.content,
        supersedes,
        tags: item.tags.unwrap_or_default(),
        source: item.source,
        metadata: item.metadata.unwrap_or_else(|| serde_json::json!({})),
    })
}
```

The empty-string normalization (`Some("")` → `None`) is defensive — TS consumers might pass an empty string from a form field rather than `undefined`. Treating them equivalently is friendlier than rejecting.

### Hook write failures hidden from JS

Per Principle VII, hook failures during ingest do NOT roll back the SQLite insert. The CLI logs via `tracing::warn!` and continues. 5c matches that behavior:

- `store.ingest(item)` resolves successfully even if Tantivy or USearch failed to update
- Subsequent `store.search(...)` may not find the new item until `singularmem reindex` runs
- Documented in the JSDoc for `Store.ingest`

## Section 4 — Testing strategy

Three layers consistent with 5a/5b.

### Layer 1: Rust unit tests (`src/types.rs`)

Extend with `NewItem` conversion tests:

- `new_item_minimal_only_content` — `{ content: "hello" }` round-trips with empty `tags` and `{}` `metadata`
- `new_item_full_fields` — all 5 fields populated round-trip correctly
- `new_item_supersedes_valid_ulid` — valid ULID string converts to `Some(ItemId)`
- `new_item_supersedes_invalid_ulid_returns_error` — malformed ULID → `code: "InvalidId"` from the conversion helper
- `new_item_supersedes_empty_string_treated_as_none` — `supersedes: ""` becomes `None`
- `new_item_metadata_default_empty_object` — missing metadata → `serde_json::json!({})`

Count: ~6 new Rust unit tests.

### Layer 2: Node integration tests

**`test/store_ingest.test.mjs`** with ~7 tests covering the public API:

- Minimal NewItem returns persisted Item (ULID format, createdAt Date, empty tags)
- Full NewItem (all 5 fields) round-trips correctly
- Empty content → `code: "Validation"`
- Malformed `supersedes` → `code: "InvalidId"`
- Non-existent `supersedes` → `code: "SupersedesNotFound"`
- Read-only store + ingest → `code: "ReadOnly"`
- End-to-end supersession chain via two ingests + `store.revisions`

**`test/store_ingest_indexes.test.mjs`** with ~3 tests covering hook auto-wiring:

- Ingest into fresh store (no sidecars) — subsequent search throws `NoIndexes` (proves no auto-wiring without sidecars)
- Ingest after pre-priming sidecars — newly ingested item appears in `store.search` results (proves auto-wiring works)
- Read-only store + ingest still rejects with `ReadOnly` even when sidecars exist (proves auto-wiring skipped for RO)

Count: ~10 new Node integration tests.

All tests use `process.env.SINGULARMEM_TEST_EMBEDDER = 'mock'` at the top to keep CI fast.

### Layer 3: TypeScript type smoke test

Extend `test/types.test.mts` with `NewItem` exercise:

```typescript
const minimal: NewItem = { content: 'hello' };
const full: NewItem = {
  content: 'all fields',
  supersedes: '01H...',
  tags: ['x', 'y'],
  source: 'test',
  metadata: { k: 'v' },
};

async function _ingestCheck(s: Store): Promise<void> {
  const _x: Item = await s.ingest({ content: 'hi' });
  const _y: Item = await s.ingest(full);
}
```

### CI

No new CI job. The existing `node-bindings` job picks up new `test/*.test.mjs` files via its glob.

**Test counts after 5c:**
- Rust unit tests: 35 → ~41
- Node integration tests: 34 → ~44
- TS smoke test: 1 file, extended

## Section 5 — Acceptance criteria + Constitution Check

### Acceptance criteria

**API surface (matches Section 1):**
- `NewItem` interface exported with one required field + 4 optional
- `store.ingest(item: NewItem): Promise<Item>` async method on `Store`
- Generated `.d.ts` matches the interface and method signature verbatim
- Returned `Item` has `createdAt instanceof Date`

**Auto-wiring (matches Section 2):**
- `Store.open(path)` (read-write) reuses the 5b `open_sidecars` helper to probe both sidecars
- `open_sidecars` refactored to non-fatal: individual `Index::open` / `EmbedderIndex::open` failures log via `tracing::warn!` and return `None` for that sidecar instead of bubbling an error (benefits search/retrieve too)
- Returned `Index` and/or `EmbedderIndex` re-wrapped as `Box<dyn IndexHook>` and assembled into a `MultiHook`
- `MultiHook` registered via `Store::set_hook` on the inner CoreStore
- `Store.open(path, { readOnly: true })` skips auto-wiring entirely
- Hook write failures during ingest logged + ignored per Principle VII

**Behavior (matches Section 3):**
- Empty `content` → `code: "Validation"`
- Malformed `supersedes` ULID → `code: "InvalidId"`
- Empty-string `supersedes: ""` treated as `None`
- Non-existent `supersedes` ID → `code: "SupersedesNotFound"`
- Read-only ingest → `code: "ReadOnly"`
- Missing `tags` → `[]`
- Missing `metadata` → `{}`
- Hook write failures don't surface to JS

**Tests (matches Section 4):**
- ~6 new Rust unit tests pass (~41 total)
- ~10 new Node integration tests pass (~44 total)
- TS smoke test compiles with `NewItem` exercise
- All three layers blocking in `node-bindings` CI

**DCO sign-off on every commit.**

### Out of scope

- `ingestMany` batch method
- JS hook registration / custom hooks
- Shared index handles between write hooks and read paths
- Multi-platform prebuilt binaries (sub-project 6)
- Flutter GUI (sub-project 7+)

### Constitution Check (v0.2.0)

- **I. Local-first, file-based** ✅ — local SQLite writes; no network beyond opt-in fastembed model fetch
- **II. Provider-agnostic** ✅ — ingest is provider-neutral
- **III. Append-only, revisable** ✅ — writes go through existing core API; append-only + supersedes semantics enforced upstream
- **IV. Open core** ✅ — Apache-2.0; lives in `crates/`
- **V. Stable on-disk format** ✅ — `formatVersion` stays `"1"`; uses existing schema
- **VI. Single binary, zero deps** — partial / acceptable (same trade-off as 5a/5b; Node 20.12+ required for consumers)
- **VII. Composable crates** ✅ — uses `Store::set_hook` + `Index::open` + `EmbedderIndex::open` from public APIs
- **VIII. Tested at every layer** ✅ — three test layers, all blocking
- **IX. Documented behavior** ✅ — JSDoc on `NewItem` + `Store.ingest`; README extended with Ingest section; hook-failure semantics documented
- **X. Performance budgets** — N/A for the binding itself. Ingest throughput bounded by the existing `ingest_throughput` perf budget (~19,800 items/s). Napi + hook overhead is sub-millisecond per call.

### Open items (deferred, not blocking)

- **Hook-success visibility** — JS callers have no signal for "ingest succeeded but Tantivy hook silently failed." Could add `{ item: Item, hookWarnings?: string[] }` return shape later; YAGNI for v0.
- **Concurrent ingest from multiple JS processes** — handled at the SQLite/Tantivy/USearch lock layer. Same behavior as the CLI run twice. Not a 5c concern.

## Next steps after this spec is approved

1. Run writing-plans skill to produce `docs/superpowers/plans/2026-05-19-ts-sdk-5c.md`
2. Execute via subagent-driven-development
3. PR, merge, version bump to 0.13.0, tag
4. **TS SDK feature-complete for v0.** Begin sub-project 6 brainstorming (distribution & packaging — multi-platform prebuilt binaries, npm publish workflow)
