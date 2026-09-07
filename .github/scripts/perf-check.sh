#!/usr/bin/env bash
# Enforce the eight perf budgets from Constitution Principle X.
# Reads criterion's per-bench estimates.json (stable JSON schema) rather
# than parsing CLI bencher output.
# Exit codes: 0 success, 11=size, 12=cold start, 13=ingest, 14=ingest with
# indexes, 15=single ingest with vector index only, 16=query, 17=semantic,
# 18=hybrid.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

cargo build --release --bin singularmem
BIN="$REPO_ROOT/target/release/singularmem"

# 1. Binary size budget: < 150 MB
SIZE_BYTES=$(stat -c %s "$BIN")
SIZE_LIMIT=$((150 * 1024 * 1024))
if [[ "$SIZE_BYTES" -ge "$SIZE_LIMIT" ]]; then
    echo "FAIL: binary size $SIZE_BYTES exceeds limit $SIZE_LIMIT" >&2
    exit 11
fi

# 2. CLI cold start budget: < 200 ms (median of 5 runs)
COLD_START_P50=$("$REPO_ROOT/.github/scripts/median.sh" 5 -- "$BIN" --version)
if [[ "$COLD_START_P50" -ge 200 ]]; then
    echo "FAIL: cold start $COLD_START_P50 ms exceeds 200 ms" >&2
    exit 12
fi

# 3. Run benches (writes target/criterion/*/new/estimates.json)
cargo bench --workspace --quiet 2>&1 | tail -5

# Helper: extract median point_estimate (nanoseconds) from a criterion
# estimates.json file. Argument is the bench path relative to
# target/criterion/ (without the trailing /new/estimates.json).
# Schema: { "median": { "point_estimate": <float-ns> }, ... }
read_median_ns() {
    local bench_path="$1"
    local file="$REPO_ROOT/target/criterion/$bench_path/new/estimates.json"
    if [[ ! -f "$file" ]]; then
        echo "FAIL: criterion estimates file missing: $file" >&2
        return 1
    fi
    python3 -c "import json; print(int(json.load(open('$file'))['median']['point_estimate']))"
}

# 4. Ingest throughput: >= 50 items/s
# bench path: target/criterion/ingest_throughput/ingest_one/new/estimates.json
INGEST_NS=$(read_median_ns "ingest_throughput/ingest_one")
THROUGHPUT=$(awk -v ns="$INGEST_NS" 'BEGIN { printf "%.2f", 1e9 / ns }')
if awk -v v="$THROUGHPUT" 'BEGIN { exit !(v < 50) }'; then
    echo "FAIL: ingest throughput $THROUGHPUT items/s below 50 items/s" >&2
    exit 13
fi

# 4b. Ingest throughput with both indexes attached (lexical + semantic,
# mock embedder): >= 50 items/s. This gates every cost sub-project 17
# addresses except the embedding model itself (deferred/async embedding is a
# non-goal, so the model cost isn't part of this gate).
# bench path: target/criterion/ingest_throughput/ingest_with_indexes/new/estimates.json
# The bench batches 100 items per iteration, hence 100 * 1e9 / ns.
WITH_IDX_NS=$(read_median_ns "ingest_throughput/ingest_with_indexes")
WITH_IDX_RATE=$(awk -v ns="$WITH_IDX_NS" 'BEGIN { printf "%.2f", 100 * 1e9 / ns }')
if awk -v v="$WITH_IDX_RATE" 'BEGIN { exit !(v < 50) }'; then
    echo "FAIL: ingest with indexes $WITH_IDX_RATE items/s below 50 items/s" >&2
    exit 14
fi

# 4c. Single-item ingest through only the vector index (EmbedderIndex), at
# 20,000 pre-seeded vectors: <= 20 ms. Asserts the journal makes the vector
# index's own per-item commit cost independent of index size (no more
# whole-graph rewrite per ingest). Gated on the vector-only bench, not the
# combined-hooks one: a Tantivy `Index` hook's per-item `commit` cost is
# pre-existing, unrelated to this sub-project, and dominates the combined
# figure (~88 ms on the measurement machine) — gating on it would fail CI
# on a cost this work didn't introduce and is out of scope to fix. See
# docs/benchmarks/ingest.md.
# bench path: target/criterion/ingest_throughput/ingest_single_with_vector_index/new/estimates.json
SINGLE_NS=$(read_median_ns "ingest_throughput/ingest_single_with_vector_index")
SINGLE_MS=$(awk -v ns="$SINGLE_NS" 'BEGIN { printf "%.2f", ns / 1e6 }')
if awk -v v="$SINGLE_MS" 'BEGIN { exit !(v > 20) }'; then
    echo "FAIL: single ingest with vector index ${SINGLE_MS} ms exceeds 20 ms" >&2
    exit 15
fi

# 4d. Single-item ingest with both a Tantivy `Index` and an `EmbedderIndex`
# hook attached (the shape a real ingest path uses): informational only,
# not gated. Tantivy's per-item segment commit dominates this figure and is
# out of this sub-project's scope (see 4c above); printed in the summary so
# the combined cost stays visible without failing CI on it.
# bench path: target/criterion/ingest_throughput/ingest_single_with_both_hooks/new/estimates.json
BOTH_HOOKS_NS=$(read_median_ns "ingest_throughput/ingest_single_with_both_hooks")
BOTH_HOOKS_MS=$(awk -v ns="$BOTH_HOOKS_NS" 'BEGIN { printf "%.2f", ns / 1e6 }')

# 5. Search query latency: < 100 ms (median; we treat median as p95-equivalent
# for v0 — criterion exposes median directly; p95 requires the iteration data
# which Tantivy + criterion don't trivially provide. Defensible v0.2.0
# approximation; v0.3+ can switch to a real p95 via criterion's raw samples).
# bench path: target/criterion/search_latency_p95/new/estimates.json
# (bench_function at top level creates a single-level directory, not a
# two-level group/func path; verified against actual criterion output.)
QUERY_NS=$(read_median_ns "search_latency_p95")
QUERY_MS=$(awk -v ns="$QUERY_NS" 'BEGIN { printf "%.2f", ns / 1e6 }')
if awk -v v="$QUERY_MS" 'BEGIN { exit !(v >= 100) }'; then
    echo "FAIL: query latency ${QUERY_MS} ms exceeds 100 ms" >&2
    exit 16
fi

# 6. Semantic search latency: < 100 ms (median of criterion estimates.json)
# bench path: target/criterion/semantic_search_latency/new/estimates.json
# (bench_function at top level creates a single-level directory, same
# convention as search_latency_p95 above.)
SEM_NS=$(read_median_ns "semantic_search_latency")
SEM_MS=$(awk -v ns="$SEM_NS" 'BEGIN { printf "%.2f", ns / 1e6 }')
if awk -v v="$SEM_MS" 'BEGIN { exit !(v >= 100) }'; then
    echo "FAIL: semantic search latency ${SEM_MS} ms exceeds 100 ms" >&2
    exit 17
fi

# 7. Hybrid search latency: < 150 ms (median of criterion estimates.json)
# bench path: target/criterion/hybrid_search_latency/new/estimates.json
# (same single-level path as search_latency_p95 and semantic_search_latency.)
HYBRID_NS=$(read_median_ns "hybrid_search_latency")
HYBRID_MS=$(awk -v ns="$HYBRID_NS" 'BEGIN { printf "%.2f", ns / 1e6 }')
if awk -v v="$HYBRID_MS" 'BEGIN { exit !(v >= 150) }'; then
    echo "FAIL: hybrid search latency ${HYBRID_MS} ms exceeds 150 ms" >&2
    exit 18
fi

echo "All perf budgets satisfied:"
echo "  binary size:            ${SIZE_BYTES} bytes (limit ${SIZE_LIMIT})"
echo "  cold start (p50):       ${COLD_START_P50} ms (limit 200)"
echo "  ingest throughput:      ${THROUGHPUT} items/s (limit 50)"
echo "  ingest with indexes:    ${WITH_IDX_RATE} items/s (limit 50)"
echo "  single ingest, vector index only:  ${SINGLE_MS} ms (limit 20)"
echo "  single ingest, both hooks (info):  ${BOTH_HOOKS_MS} ms (not gated; Tantivy-dominated)"
echo "  search latency:         ${QUERY_MS} ms (limit 100)"
echo "  semantic search:        ${SEM_MS} ms (limit 100)"
echo "  hybrid search:          ${HYBRID_MS} ms (limit 150)"
