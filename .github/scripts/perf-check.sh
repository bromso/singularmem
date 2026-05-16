#!/usr/bin/env bash
# Enforce the four perf budgets from Constitution Principle X.
# Exits 0 on success, 11–14 to identify which budget broke.

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

# 3. Ingest throughput: >= 50 items/s
# Criterion's bencher output looks like:
#   test ingest_throughput/ingest_one ... bench:  XXXXX ns/iter (+/- YYYY)
# Convert ns/iter to items per second.
INGEST_NS=$(cargo bench -p singularmem-core --bench store_perf -- ingest_throughput --output-format=bencher 2>/dev/null \
    | awk '/ingest_one/ && /ns\/iter/ { gsub(",", "", $5); print $5; exit }')
if [[ -z "$INGEST_NS" ]]; then
    echo "FAIL: could not parse ingest throughput" >&2
    exit 13
fi
THROUGHPUT=$(awk -v ns="$INGEST_NS" 'BEGIN { printf "%.2f", 1e9 / ns }')
if awk -v v="$THROUGHPUT" 'BEGIN { exit !(v < 50) }'; then
    echo "FAIL: ingest throughput $THROUGHPUT items/s below 50 items/s" >&2
    exit 13
fi

# 4. Point-read query latency p95: < 100 ms
# Criterion bencher output is mean ns/iter; for v0 we approximate p95 as
# mean * 1.5 (a generous-but-defensible heuristic given criterion's lack of
# a built-in p95 in bencher mode). The store_perf bench includes a comment
# documenting this. If a future budget revision requires real p95s, switch
# to criterion's HTML report parsing or use criterion-perf-events.
QUERY_MEAN_NS=$(cargo bench -p singularmem-core --bench store_perf -- get_p95 --output-format=bencher 2>/dev/null \
    | awk '/point_read/ && /ns\/iter/ { gsub(",", "", $5); print $5; exit }')
if [[ -z "$QUERY_MEAN_NS" ]]; then
    echo "FAIL: could not parse query latency" >&2
    exit 14
fi
QUERY_P95_MS=$(awk -v ns="$QUERY_MEAN_NS" 'BEGIN { printf "%.2f", (ns * 1.5) / 1e6 }')
if awk -v v="$QUERY_P95_MS" 'BEGIN { exit !(v >= 100) }'; then
    echo "FAIL: query p95 ${QUERY_P95_MS} ms exceeds 100 ms" >&2
    exit 14
fi

echo "All perf budgets satisfied:"
echo "  binary size:       ${SIZE_BYTES} bytes (limit ${SIZE_LIMIT})"
echo "  cold start (p50):  ${COLD_START_P50} ms (limit 200)"
echo "  ingest throughput: ${THROUGHPUT} items/s (limit 50)"
echo "  query p95 (est.):  ${QUERY_P95_MS} ms (limit 100)"
