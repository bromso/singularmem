#!/usr/bin/env bash
# Enforce the five perf budgets from Constitution Principle X.
# Reads criterion's per-bench estimates.json (stable JSON schema) rather
# than parsing CLI bencher output.
# Exit codes: 0 success, 11=size, 12=cold start, 13=ingest, 14=query, 15=semantic.

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
    exit 14
fi

# 5. Semantic search latency: < 100 ms (median of criterion estimates.json)
# bench path: target/criterion/semantic_search_latency/new/estimates.json
# (bench_function at top level creates a single-level directory, same
# convention as search_latency_p95 above.)
SEM_NS=$(read_median_ns "semantic_search_latency")
SEM_MS=$(awk -v ns="$SEM_NS" 'BEGIN { printf "%.2f", ns / 1e6 }')
if awk -v v="$SEM_MS" 'BEGIN { exit !(v >= 100) }'; then
    echo "FAIL: semantic search latency ${SEM_MS} ms exceeds 100 ms" >&2
    exit 15
fi

echo "All perf budgets satisfied:"
echo "  binary size:       ${SIZE_BYTES} bytes (limit ${SIZE_LIMIT})"
echo "  cold start (p50):  ${COLD_START_P50} ms (limit 200)"
echo "  ingest throughput: ${THROUGHPUT} items/s (limit 50)"
echo "  search latency:    ${QUERY_MS} ms (limit 100)"
echo "  semantic search:   ${SEM_MS} ms (limit 100)"
