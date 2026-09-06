# Ingest throughput (sub-project 17)

Numbers for the ingest-throughput work in
[`2026-09-06-ingest-throughput-17-design.md`](../superpowers/specs/2026-09-06-ingest-throughput-17-design.md):
the mock-embedder figures the CI perf gates enforce, and real-model
before/after figures from this machine. CPU: **Apple M2 Max** (12 cores),
macOS (Darwin 25.6.0).

- **Before:** `v0.20.0`, commit `85494c7` (last release before this
  sub-project — no `on_ingest_batch`, no vector journal; every commit
  rewrote the whole `index.usearch`).
- **After:** `ingest-throughput-17`, commit `8a12ace` (batched hook
  indexing, `EmbedderIndex::on_ingest_batch` in chunks of 64, journal-backed
  `VectorIndex` with threshold/batch-end compaction).
- Model: `sentence-transformers/all-MiniLM-L6-v2@v1` (fastembed,
  `FastembedEmbedder::new()`), dim 384.
- Constitution Principle X floor: ingest throughput **≥ 50 items/s**
  (with every index attached).

## CI-gated figures (mock embedder)

`.github/scripts/perf-check.sh` reads these from
`target/criterion/ingest_throughput/*/new/estimates.json`
(`cargo bench -p singularmem-search --bench search_perf -- ingest_
--warm-up-time 1 --measurement-time 5`, run manually on macOS — the script's
`stat -c` is GNU-only, see "Verification" in the Task 5 report).

| Bench | Median | Rate / cost | Gate |
|---|---|---|---|
| `ingest_throughput/ingest_with_indexes` — bulk `ingest_many`, 100 realistic (~1,500 char) items, Tantivy `Index` + `EmbedderIndex` over `MockEmbedder` | 240.81 ms / 100 items | **415.28 items/s** | ≥ 50 items/s (target ≥ 200) |
| `ingest_throughput/ingest_single_with_vector_index` — single `Store::ingest`, **`EmbedderIndex` only**, 20,000 pre-seeded vectors | **6.75 ms/item** | — | **≤ 20 ms/item (gated)** |
| `ingest_throughput/ingest_single_with_both_hooks` — single `Store::ingest`, Tantivy `Index` + `EmbedderIndex` in one `MultiHook`, same 20,000 pre-seeded vectors | 86.74 ms/item | — | ungated, informational |

The bulk gate passes with wide margin (415 ≫ 200 ≫ 50). The single-item gate
is **vector-index-only** and passes comfortably (6.75 ms ≪ 20 ms budget); the
combined-hooks figure is reported alongside it for visibility but is not
gated — see "Why the gate is vector-only" below.

## Why the gate is vector-only

Sub-project 17's acceptance criterion is that the vector index's own
per-item commit cost stops scaling with index size — the whole point of the
journal (see `docs/formats/vectors-v2.md`). Measuring that claim through a
`MultiHook` of a Tantivy `Index` **and** an `EmbedderIndex` (as the design's
text originally specified) conflates it with a second, unrelated cost:
Tantivy commits a whole segment on every single-item `commit()` call — a
pre-existing cost, present before any of Tasks 1–4 and out of this
sub-project's scope (the design's own non-goals list "Changing the Tantivy
sidecar") — and that cost dominates the combined figure:

| Configuration | Median | What it measures |
|---|---|---|
| `ingest_single_with_vector_index` — `EmbedderIndex` alone, 20,000 mock-preseeded vectors, this branch | **6.75 ms** | The vector index's own per-item commit cost, journal-backed. **This is what the CI gate checks, at ≤ 20 ms.** |
| `Index` (Tantivy) alone, fresh/empty directory, this branch | ~88.0 ms | Tantivy's per-item segment commit — unrelated to sub-project 17 |
| `ingest_single_with_both_hooks` — both hooks together | 86.74 ms | Sum of the two, Tantivy-dominated. Printed by `perf-check.sh`'s summary line, not gated. |

Gating the combined figure would fail CI on a cost this sub-project neither
introduced nor can fix, while telling a reader nothing about whether the
vector-index work it is meant to verify actually succeeded. Isolating the
vector-index-only bench as the gate, and keeping the combined-hooks bench
as an ungated, reported sibling, measures the right thing while still
keeping the realistic (Tantivy-dominated) cost visible.

**Next performance follow-up:** Tantivy's per-item `commit` cost
(~88 ms on this machine, for a single-item ingest through the Tantivy
`Index` hook) is unaddressed by this sub-project and is the largest
remaining single-item ingest cost when both hooks are attached. Reducing
it is out of scope here (see the design's non-goals) but is the natural
next target for ingest-latency work.

## Vector-index-only isolation: before vs after

To evidence the actual claim — per-item vector-index cost no longer scales
with index size — the same single-item loop with **only** the
`EmbedderIndex` hook (`MockEmbedder`, 384-dim, so identical cost profile to
the real model's vector-index path) was run at 20,000 pre-seeded vectors on
both commits:

| | Before (`85494c7`, whole-graph rewrite every commit) | After (`8a12ace`, journal) |
|---|---|---|
| Median | 20.19 ms | **6.63 ms** |
| p90 | 24.30 ms | 9.75 ms |
| Max | 36.84 ms | 12.17 ms |

**~3× faster**, and the before figure (20.19 ms at 20,000 items) lines up
with the design's own historical measurement of raw `USearch::save` cost —
11 ms at 10k items, 58 ms at 50k — confirming the old per-commit cost really
was the whole-graph rewrite, and that it keeps growing with corpus size
where the after figure does not (it depends on journal size, which resets
to empty at every compaction).

## Real-model bulk ingest (`Store::ingest_many`, both hooks, 256 items)

Ad-hoc example (not committed — the brief's temporary example convention):
`Store::ingest_many` of 256 items through a `MultiHook` of a Tantivy
`Index` and an `EmbedderIndex` over `FastembedEmbedder::new()`, for three
text lengths.

| Text length | Before (`85494c7`) | After (`8a12ace`) | Speedup |
|---|---|---|---|
| Short (~80 chars) | 401.76 items/s (2.489 ms/item) | 666.81 items/s (1.500 ms/item) | **1.66×** |
| Realistic (~1,500 chars) | 43.64 items/s (22.914 ms/item) | 69.69 items/s (14.349 ms/item) | **1.60×** |
| Long (~4,000 chars) | 23.22 items/s (43.066 ms/item) | 37.99 items/s (26.324 ms/item) | **1.64×** |

Realistic-text speedup (1.60×) clears acceptance criterion 3's ≥ 1.4×
floor. The gain here is `EmbedderIndex::on_ingest_batch`'s chunked
`embed_batch` calls (chunks of 64) plus one compacting commit per batch,
replacing the old per-item embed + per-item implicit graph currency;
Tantivy's cost is already one commit per batch on both sides, so it isn't
the source of the improvement here (contrast the single-item case above).

## Real-model single-item ingest (50-item loop, both hooks)

Pre-seeded with 2,000 items via the **real embedder** (not `MockEmbedder` —
opening the same directory with a different model id fails with
`Error::ModelMismatch`, so the seed and the measured loop must share a
model). 2,000 was chosen instead of the design's 20,000 to keep the
real-model run to a few minutes; see "Vector-index-only isolation" above
for the size-independence evidence at 20,000, using `MockEmbedder` to keep
that run fast.

| | Before (`85494c7`) | After (`8a12ace`) |
|---|---|---|
| Both hooks, median | 86.48 ms/item | 88.62 ms/item |
| `EmbedderIndex` alone, median | 24.07 ms/item | 27.85 ms/item |

Both figures are essentially unchanged before/after, as expected: at only
2,000 pre-seeded items the old whole-graph rewrite cost is small (a couple
of ms, per the `USearch::save` figures above) relative to the real
embedding cost itself (~14–24 ms for a single ~1,500-char text, per the
design's own "one at a time" figures), so this scale doesn't exercise the
size-dependent regression the journal fixes. The mock-embedder measurement
at 20,000 items above isolates that effect instead, without paying for
20,000 real embedding calls.

## Reproducing

```bash
# CI-gated mock-embedder benches
cargo bench -p singularmem-search --bench search_perf -- ingest_ \
  --warm-up-time 1 --measurement-time 5
python3 -c "import json; print(json.load(open('target/criterion/ingest_throughput/ingest_with_indexes/new/estimates.json'))['median']['point_estimate'])"

# Real-model numbers: build a temporary example under
# crates/singularmem-search/examples/ that ingests through Store::ingest_many
# / Store::ingest with a Tantivy Index hook and an EmbedderIndex over
# FastembedEmbedder::new(), run with `cargo run --release -p
# singularmem-search --example <name>`, then delete it — do not commit an
# example that depends on network/model download at build time.
```
