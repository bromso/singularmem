#!/usr/bin/env bash
# Run a command N times, print the median wall-clock time in milliseconds.
# Usage: median.sh <N> -- <command...>

set -euo pipefail

N=${1:-5}
shift
if [[ "${1:-}" != "--" ]]; then
    echo "usage: $0 <N> -- <command...>" >&2
    exit 64
fi
shift

declare -a times_ms=()
for ((i = 0; i < N; i++)); do
    start_ns=$(date +%s%N)
    "$@" > /dev/null 2>&1 || true  # we measure cold-start regardless of exit
    end_ns=$(date +%s%N)
    elapsed_ms=$(( (end_ns - start_ns) / 1000000 ))
    times_ms+=("$elapsed_ms")
done

# Sort and pick the middle.
IFS=$'\n' sorted=($(sort -n <<<"${times_ms[*]}"))
unset IFS
median_idx=$((N / 2))
echo "${sorted[$median_idx]}"
