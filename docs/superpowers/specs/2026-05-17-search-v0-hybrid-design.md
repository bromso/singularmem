---
title: Search v0 — Hybrid retrieval
date: 2026-05-17
status: draft
sub-project: 2c-search-v0-hybrid
supersedes: none
---

# Search v0 — Hybrid retrieval

This sub-project completes the **Search v0** piece named in the
constitution's Open / Closed Split by fusing the lexical (Tantivy BM25,
shipped in v0.2.0) and semantic (USearch cosine, shipped in v0.3.0)
indexes into a single ranked result list. Fusion uses Reciprocal Rank
Fusion (RRF) with `k = 60`. The existing `search` CLI verb grows a
`--mode {auto|lexical|semantic|hybrid}` flag and defaults to `auto`,
which picks the strongest mode the store's sidecars support.

## Problem & motivation

After sub-projects 2a and 2b, Singularmem can answer two different
kinds of question — keyword (`search`) and meaning (`semantic-search`)
— but a user has to pick one verb at a time. In practice neither
ranker is uniformly best: lexical excels at exact terms, IDs, and rare
tokens; semantic excels at paraphrase and concept queries. Hybrid
retrieval composes both rankers so the result list contains hits each
ranker would have surfaced on its own, ordered by a fused score that
rewards agreement between rankers.

This is the last open piece of the constitution's Search v0 line item;
once this lands, provider adapters (sub-project 3) and the MCP server
(sub-project 4) can build on a single `search` entrypoint instead of
having to choose a ranker themselves.

## Goals & non-goals

### Goals

1. Library type `HybridSearcher` in `singularmem-search` that borrows
   one or both of `Index` (lexical) and `EmbedderIndex` (semantic) and
   returns a single ranked `HybridSearchResults`.
2. Reciprocal Rank Fusion with `k = 60` (Cormack et al. 2009).
3. CLI: `singularmem search --mode {auto|lexical|semantic|hybrid}
   <query>`, defaulting to `auto`.
4. Graceful degradation under `auto`: hybrid when both sidecars exist,
   lexical-only or semantic-only when only one exists, error when
   neither exists. Explicit modes fail loudly when their required index
   is missing.
5. `--show-ranks` and `--json` output flags on the `search` verb.
6. Deprecated alias: `semantic-search <query>` continues to work,
   prints a one-shot deprecation note, forwards to `search --mode
   semantic`.
7. New criterion bench `hybrid_search_latency` enforced in CI at
   `< 150 ms` on `ubuntu-latest` (blocking, sixth Principle X budget).

### Non-goals

- **Parallel sub-searches.** Sub-searches run sequentially (lexical
  first, then semantic). Parallelisation is deferred to v0.5+.
- **Learned-to-rank or weighted blends.** Only RRF in v0.4.
  Configurable weighting is a v0.5+ topic.
- **Cross-encoder re-ranking.** Out of scope; would require a second
  model and another perf budget.
- **Cross-process index sharing.** Same writer-lock rules as 2a/2b.
- **On-disk format changes.** Hybrid is read-time only. `format_version`
  stays `"1"`. Principle III.b preserved.
- **End-to-end tests against `FastembedEmbedder`.** Model downloads
  violate the offline rule. All tests use `MockEmbedder`.

## Recommended approach

A new `HybridSearcher<'a>` struct in `crates/singularmem-search/src/
hybrid_query.rs` borrows references to the underlying indexes (instead
of owning them), so callers retain control of index lifecycle. Three
constructors: `new(lex, sem)`, `lexical_only(lex)`, `semantic_only(sem)`.
A single `search(&self, query, opts)` method dispatches based on which
references are present.

Fusion is RRF: each document's score is the sum, over the rankers it
appears in, of `1 / (k + rank_in_ranker)`. `k = 60` per Cormack et al.
2009. Per-ranker fetch size is `limit * fetch_multiplier`
(default multiplier `3`), so 20 final hits draws on 60 candidates per
ranker. Ties (rare but possible with disjoint rankers and tiny
corpora) break by `ItemId` ascending — this requires deriving `Ord`
on `ItemId` in `singularmem-core`, an additive change.

The CLI's `cmd_search` does a directory-existence probe
(`<store>.tantivy/`, `<store>.vectors/`) when `--mode auto`, and
picks the appropriate constructor. Explicit modes skip the probe and
either succeed or return `Error::IndexMissing` /
`Error::HybridMissingIndex`.

### Approaches discarded

- **Approach B — `HybridSearcher` owns both indexes.** Rejected
  because it duplicates index lifecycle management already owned by
  `Store` and the CLI's existing pattern, and makes
  partial-availability awkward (would need `Option<Index>` /
  `Option<EmbedderIndex>` fields, plus constructors that take owned
  values the caller may not have).
- **Approach C — Linear-blend fusion (`α · bm25 + (1−α) · cos`).**
  Rejected because BM25 and cosine scores have incomparable scales,
  requiring per-corpus normalisation or a tuned α. RRF is
  scale-invariant (operates on ranks only) and has no free parameter
  beyond `k`. Linear blend remains a v0.5+ option if users ask.
- **Approach D — One sub-project per concern (RRF library, CLI flag,
  perf budget).** Rejected because the three pieces only make sense
  together; splitting would ship a library with no public driver.

## Architecture

Components and their responsibilities:

- **`singularmem-search::hybrid_query::HybridSearcher<'a>`** — new
  module. Public type that borrows `&'a Index` and/or `&'a
  EmbedderIndex`. Single method `search(&self, &str,
  &HybridSearchOptions) -> Result<HybridSearchResults>`. Pure read-time
  composition, no on-disk state.
- **`singularmem-search::hybrid_query::{HybridSearchOptions,
  HybridSearchResults, HybridHit}`** — request/response types,
  serde-serialisable for `--json` output.
- **`singularmem-search::error::Error`** — gains two variants:
  `NoIndexes` and `HybridMissingIndex { missing: &'static str, path:
  PathBuf }`.
- **`singularmem-core::item::ItemId`** — gains `Ord` + `PartialOrd`
  derives. Additive; ULID byte order = lexicographic time order =
  deterministic tie-break.
- **Root binary `src/main.rs`** — `Command::Search` variant grows
  `--mode`, `--fetch-multiplier`, `--rrf-k`, `--show-ranks`, `--json`
  flags. `cmd_search` gains the auto-mode probe + dispatch.
  `Command::SemanticSearch` is retained as a deprecated alias; emits
  one-shot deprecation note via `std::sync::OnceLock`.

The hybrid layer reuses, untouched, every existing piece:
`Index::search`, `EmbedderIndex::semantic_search`, `Store::open_*`,
the `MultiHook` auto-wiring for `Ingest`. No changes to
`singularmem-core` beyond the `ItemId` derive.

## Data model

**No changes.** Hybrid search is read-time only. `format_version`
in `docs/formats/store-v1.md` stays `"1"`. No new sidecar directories,
no new SQLite tables, no new files of any kind. Principle III.b is
preserved by construction.

## Interfaces

### CLI

```
singularmem search [OPTIONS] <QUERY>...

Options:
  -m, --mode <MODE>           auto | lexical | semantic | hybrid  [default: auto]
  -l, --limit <N>             max hits to return  [default: 20]
      --fetch-multiplier <N>  per-ranker overfetch factor (hybrid only)  [default: 3]
      --rrf-k <K>             RRF damping constant (hybrid only)  [default: 60]
      --no-snippets           omit snippet highlights
      --show-ranks            include per-ranker rank columns in human output
      --json                  emit JSON results
```

**Human output (default)** — score-column tag tells the user which
ranker path executed:

```
01ARZ3NDEKTSV4RRFFQ69G5FAV  rrf=0.0328  "the quick brown fox jumps over…"
01ARZ3NEHTNV1V4SY2K0E2H6PJ  rrf=0.0312  "lazy dogs are the worst kind of…"
```

Lexical-only fallback uses `bm25=<float>`; semantic-only uses
`cos=<float>`.

**`--show-ranks`** adds per-ranker columns; `—` for a doc absent from
that ranker:

```
01ARZ3NDEKTSV4RRFFQ69G5FAV  rrf=0.0328  lex=1  sem=4  "the quick brown fox…"
01ARZ3NEHTNV1V4SY2K0E2H6PJ  rrf=0.0312  lex=2  sem=—  "lazy dogs are the worst…"
```

**`--json`** emits the `HybridSearchResults` struct serialised via
serde; `lexical_rank` / `semantic_rank` are `null` for docs absent
from that ranker.

**`--fetch-multiplier` and `--rrf-k`** are ignored (debug-level warn,
not an error) when the resolved mode is lexical-only or semantic-only.

**`singularmem semantic-search <QUERY>`** — retained as deprecated
alias for `search --mode semantic`. Emits, once per process,
`note: 'semantic-search' is deprecated; use 'search --mode semantic'`
to stderr. Forwards all other arguments.

**Exit codes** (additive to existing):

- `0` — success (including zero hits)
- `2` — `Error::NoIndexes` (auto mode, neither sidecar exists)
- `2` — `Error::HybridMissingIndex` (explicit `--mode hybrid` with
  missing sidecar)
- Existing `Error::IndexMissing`, `Error::IndexCorrupted`,
  `Error::DimMismatch`, `Error::ModelMismatch` behaviours unchanged

### Library

`crates/singularmem-search/src/hybrid_query.rs` (new module, re-exported
from `lib.rs`):

```rust
pub struct HybridSearcher<'a> {
    pub lexical: Option<&'a Index>,
    pub semantic: Option<&'a EmbedderIndex>,
}

impl<'a> HybridSearcher<'a> {
    /// Both rankers present; `search` will fuse via RRF.
    pub fn new(lexical: &'a Index, semantic: &'a EmbedderIndex) -> Self;

    /// Lexical only; `search` returns `HybridHit`s with
    /// `semantic_rank == None`, score field is the BM25 score.
    pub fn lexical_only(lexical: &'a Index) -> Self;

    /// Semantic only; symmetric.
    pub fn semantic_only(semantic: &'a EmbedderIndex) -> Self;

    pub fn search(
        &self,
        query: &str,
        opts: &HybridSearchOptions,
    ) -> Result<HybridSearchResults>;
}

#[derive(Debug, Clone)]
pub struct HybridSearchOptions {
    pub limit: usize,            // default 20
    pub fetch_multiplier: usize, // default 3
    pub rrf_k: usize,            // default 60
    pub include_snippets: bool,  // default true
}

impl Default for HybridSearchOptions { /* the defaults above */ }

#[derive(Debug, Clone, serde::Serialize)]
pub struct HybridSearchResults {
    pub hits: Vec<HybridHit>,
    pub elapsed: std::time::Duration,
    pub total_fused: usize,
    pub lexical_hits: Option<u64>,
    pub semantic_hits: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HybridHit {
    pub id: ItemId,
    /// Fused RRF score when both rankers ran; BM25 score when
    /// lexical-only; cosine similarity when semantic-only.
    pub score: f32,
    /// Discriminator so JSON consumers can interpret `score` correctly.
    pub score_kind: ScoreKind,
    pub lexical_rank: Option<usize>,
    pub semantic_rank: Option<usize>,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScoreKind { Rrf, Bm25, Cosine }
```

`Error` gains two variants in `crates/singularmem-search/src/error.rs`:

```rust
/// Neither lexical nor vector index exists for this store.
#[error(
    "no search index exists for this store; \
     run `singularmem reindex` (and optionally `--with-embeddings`) first"
)]
NoIndexes,

/// User requested --mode hybrid but only one of the two indexes exists.
#[error(
    "hybrid search requires both indexes; {missing} index missing at {path}; \
     run `singularmem reindex --with-embeddings` to build both"
)]
HybridMissingIndex {
    /// Which side was missing — "lexical" or "semantic".
    missing: &'static str,
    /// Path that was probed.
    path: std::path::PathBuf,
},
```

`ItemId` in `crates/singularmem-core/src/item.rs` gains `Ord` +
`PartialOrd` derives.

### Wire (MCP / HTTP / IPC)

N/A. Sub-project 4 (MCP server) will surface hybrid search; this
sub-project ships the underlying library and CLI only.

## Error handling

Per Principle VII, every error names operation + state preserved:

| Error | When | State preserved |
|---|---|---|
| `Error::NoIndexes` | `--mode auto`, neither sidecar exists | Store untouched; suggests `singularmem reindex`. |
| `Error::HybridMissingIndex { missing, path }` | `--mode hybrid`, one sidecar missing | Names which side and its path; suggests `reindex --with-embeddings`. |
| `Error::IndexMissing { path }` | `--mode lexical` with no `.tantivy/`, or `--mode semantic` with no `.vectors/` | Existing variant, existing message. |
| `Error::IndexCorrupted` / `DimMismatch` / `ModelMismatch` | Bubbled up from underlying ranker | Unchanged; existing messages already tell user what to do. |

**Auto-mode degradation logs but does not error:**

- Only lexical exists → `tracing::info!("no vector index at {path}; using lexical-only search")`, returns `HybridSearcher::lexical_only` results with `score_kind: Bm25`.
- Only semantic exists → symmetric, `score_kind: Cosine`.

No silent fallbacks: every degradation produces either a visible score
tag (`bm25=` / `cos=`) in the output, or an info-level log line, or
both. Quiet-by-default at the terminal (`tracing` defaults), but
discoverable at `RUST_LOG=info`.

**No state mutation possible:** hybrid search is read-only. Nothing
to roll back.

## Testing strategy

All tests offline; `SINGULARMEM_TEST_EMBEDDER=mock` selects the
deterministic `MockEmbedder` from sub-project 2b.

### Unit tests (`crates/singularmem-search/src/hybrid_query.rs`)

| Test | What it pins down |
|---|---|
| `rrf_fuses_overlapping_results` | Doc in both rankers → score = `1/(k+r_lex) + 1/(k+r_sem)`; verify exact float |
| `rrf_handles_disjoint_results` | Doc only in lexical → score = `1/(k+r_lex)`; rank fields populated correctly |
| `rrf_ties_break_by_item_id` | Two docs with identical RRF score → sorted by `ItemId` ascending; deterministic across runs |
| `rrf_respects_limit_after_fusion` | `limit=5`, each ranker returns 15 → fusion returns ≤5 hits, uses all 30 candidates for ranking |
| `lexical_only_constructor_skips_vector_search` | `HybridSearcher::lexical_only` → every hit's `semantic_rank` is `None`; no embedder calls (mock counter) |
| `semantic_only_constructor_skips_lexical_search` | Symmetric |
| `empty_query_returns_empty_results` | Both rankers return empty → `hits.is_empty()`, no error |
| `snippet_comes_from_lexical_when_available` | Doc in both rankers → snippet text matches Tantivy's highlighted output |
| `snippet_is_none_when_only_semantic_match` | Doc only in vector ranker → `snippet: None` |

### CLI integration tests (`tests/cli.rs`)

| Test | What it verifies |
|---|---|
| `search_default_mode_uses_hybrid_when_vectors_exist` | After `reindex --with-embeddings`, plain `search foo` prints `rrf=` scores |
| `search_default_mode_falls_back_to_lexical_when_no_vectors` | After `reindex` only, plain `search foo` prints `bm25=` scores + info log present at `RUST_LOG=info` |
| `search_mode_lexical_explicit` | `--mode lexical` works regardless of vector index presence |
| `search_mode_semantic_explicit` | Symmetric |
| `search_mode_hybrid_errors_when_vectors_missing` | `--mode hybrid` without `.vectors/` → exit 2, stderr names `HybridMissingIndex` |
| `search_mode_hybrid_errors_when_lexical_missing` | Symmetric |
| `search_errors_when_both_indexes_missing` | Fresh store, no `reindex` → exit 2, stderr says `no search index exists` |
| `search_show_ranks_flag_includes_columns` | `--show-ranks` output contains `lex=` and `sem=` columns |
| `search_json_flag_emits_valid_json` | Output parses as JSON, has `hits` array with `score`/`score_kind`/`lexical_rank`/`semantic_rank` fields |
| `semantic_search_deprecated_alias_still_works` | `semantic-search foo` produces same results as `search --mode semantic foo`; stderr contains deprecation note |
| `semantic_search_deprecation_note_appears_once` | Two calls into the same process (in-process library test, not two `assert_cmd` invocations) → only first emits the note |

### Benchmark

`crates/singularmem-search/benches/hybrid_search.rs`:

- Single criterion bench `hybrid_search_latency`.
- 1000-doc corpus (same fixture shape as `semantic_search_latency`).
- 4-token query, `MockEmbedder`.
- Writes `target/criterion/hybrid_search_latency/new/estimates.json`.

### Perf budget

Sixth budget in `.github/scripts/perf-check.sh`:

```bash
# Budget 16: hybrid_search_latency < 150 ms
# Rationale: lexical (~121µs) + semantic (~181µs) + RRF fusion (HashMap, ~1000 entries)
# All blocking on ubuntu-latest.
check_budget "hybrid_search_latency" 150 16
```

Exit code 16. Same `perf-budgets` job, just one more `check_budget`
call. All six budgets blocking.

### Offline guarantee

Per Principle VI: every test in this sub-project uses `MockEmbedder`
and never touches the network. No `fastembed::TextEmbedding::try_new`
calls. The `tests-offline` advisory job already verifies this for
sub-project 2b's tests; it will pick up the new ones with no
configuration changes.

## Open questions

None at spec time. Two notes for the implementation plan:

1. **`ItemId` `Ord` derive** must land in sub-project 2c (not deferred)
   because the tie-break code depends on it. The change is one
   `#[derive]` line, no behaviour change (ULID byte order = lexicographic
   time order), but it does need an entry in the plan.
2. **`HybridHit::score` shape.** The spec uses one `score: f32` field
   plus a `score_kind: ScoreKind` discriminator (rather than separate
   `rrf_score` / `bm25_score` / `cos_score` fields) so that JSON
   consumers can iterate uniformly. The plan should call this out so
   the implementer doesn't accidentally pick the per-ranker-field
   shape from earlier section drafts.

## Acceptance criteria

1. `crates/singularmem-search/src/hybrid_query.rs` exists and exports
   `HybridSearcher`, `HybridSearchOptions`, `HybridSearchResults`,
   `HybridHit`, `ScoreKind` with the signatures in the Library
   interface section above.
2. `Error::NoIndexes` and `Error::HybridMissingIndex { missing, path }`
   exist in `crates/singularmem-search/src/error.rs`.
3. `ItemId` derives `Ord` + `PartialOrd` in
   `crates/singularmem-core/src/item.rs`.
4. `singularmem search <query>` (no `--mode` flag) prints `rrf=<float>`
   scores when both `.tantivy/` and `.vectors/` exist; prints
   `bm25=<float>` scores when only `.tantivy/` exists; prints
   `cos=<float>` scores when only `.vectors/` exists; exits 2 with
   `no search index exists` when neither exists.
5. `singularmem search --mode {lexical|semantic|hybrid} <query>`
   enforces the strict-mode behaviour from the error-handling table
   (hard error when required index missing).
6. `singularmem search --show-ranks <query>` adds `lex=N` and `sem=N`
   columns (with `—` for missing rank) to human output.
7. `singularmem search --json <query>` emits a JSON document with
   `hits` array; each hit has `id`, `score`, `score_kind`,
   `lexical_rank`, `semantic_rank`, `snippet` fields.
8. `singularmem semantic-search <query>` continues to work as an alias
   for `search --mode semantic <query>`; emits one-shot deprecation
   note to stderr.
9. All 9 unit tests + 11 CLI integration tests from the Testing
   Strategy section pass on both `ubuntu-latest` and `macos-latest`.
10. `hybrid_search_latency` criterion bench exists and the
    `perf-budgets` job enforces it `< 150 ms` on `ubuntu-latest`
    (blocking, exit code 16).
11. `docs/formats/store-v1.md` is unchanged (`format_version` stays
    `"1"`).
12. Tagged on merge as `v0.4.0` (additive MINOR bump per Principle V).

## Constitution Check

| Principle | How this design complies |
|---|---|
| **I — Local-First and Sovereign** | No new network calls. Hybrid search reads existing local sidecars only. |
| **II — Provider-Agnostic by Contract** | All new code lives in `singularmem-search` (Apache-2.0) and the root binary. Nothing reserved for the proprietary GUI. |
| **III — Open Core with a Stable Boundary** | III.a: adds public surface (`HybridSearcher`, `--mode` flag, `--json`); removes nothing. `semantic-search` retained as deprecated alias preserves the one-way ratchet. III.b: zero on-disk changes; `format_version` unchanged. |
| **IV — CLI-First** | All hybrid functionality reachable from the existing `search` verb; no library-only paths. JSON output (`--json`) is scriptable. |
| **V — Composable Library Architecture** | `HybridSearcher::{new, lexical_only, semantic_only}` constructors let library users compose; `&'a` borrow design means no ownership transfer. Reuses `Index` and `EmbedderIndex` unchanged. |
| **VI — Deterministic and Offline-Testable** | RRF with fixed `k = 60` + `ItemId` tie-break → byte-identical results for fixed corpus + fixed embedder. All tests use `MockEmbedder`; offline. |
| **VII — Honest Failure Modes** | Auto-mode degradation = log + continue with a visible score-kind tag (`bm25=` / `cos=`). Explicit modes fail loudly with named-state errors (`NoIndexes`, `HybridMissingIndex`). No silent fallbacks. No state mutation possible (read-only). |
| **VIII — Privacy Telemetry** | No telemetry added. `tracing::info!` lines emit only the file path that was probed, no item content or query text. |
| **IX — Accessible by Default** | CLI output uses ASCII (`—` is U+2014; acceptable but plan should verify Windows-cmd behaviour or fall back to `-`). `--json` provides programmatic access for screen-reader and tooling users. |
| **X — Performance Budgets, Enforced in CI** | New `hybrid_search_latency < 150 ms` budget joins existing 5; all 6 blocking on `ubuntu-latest`. |
