# singularmem-bench

Retrieval-quality benchmark harness for Singularmem. Evaluates the real
retrieval pipeline (`Retriever` over `HybridSearcher`) against
[LongMemEval](https://huggingface.co/datasets/xiaowu0162/longmemeval) and
reports Recall@k and MRR per search mode. It is a by-hand tool, not a CI
gate — see [`docs/benchmarks/longmemeval.md`](../../docs/benchmarks/longmemeval.md)
for a published run with numbers, commit, model, and machine.

The crate composes `singularmem-core`, `singularmem-search`,
`singularmem-retrieve`, and `singularmem-ingest`; it contains no retrieval
logic of its own (Principle V). It does not download the dataset — fetch
the file yourself and pass a local path (Principle VI, no network path in
the tool).

## Usage

```bash
singularmem-bench longmemeval <FILE> [OPTIONS]
```

`<FILE>` is a local path to `longmemeval_s`, `longmemeval_m`, or
`longmemeval_oracle` (the Hugging Face files have no `.json` extension;
renaming to `.json` is only for your own convenience, the tool reads the
content regardless of extension).

| Flag | Meaning |
|---|---|
| `--modes <MODES>` | Comma-separated search modes to evaluate: `lexical`, `semantic`, `hybrid`. Default: all three. `--modes lexical` never loads an embedder. |
| `--k <KS>` | Comma-separated Recall cut-offs, each ≥ 1. Default: `1,5,10`. |
| `--model <MODEL>` | Embedding model: `all-mini-lm-l6-v2`, `bge-small-en`, `nomic-embed`. Default: `all-mini-lm-l6-v2`. Ignored for `--modes lexical`. |
| `--limit <N>` | Evaluate only `N` questions (see `--seed`). |
| `--question-type <T>` | Only questions of this type; repeatable. One of `single-session-user`, `single-session-assistant`, `single-session-preference`, `multi-session`, `temporal-reasoning`, `knowledge-update` — any other value is a clap usage error. |
| `--seed <S>` | Shuffle seed used with `--limit`; `0` (default) keeps file order. |
| `--json <PATH>` | Write the full JSON report (per-question hit lists included) to this path. |
| `--quiet` | Suppress per-question progress on stderr. |

## Exit codes

| Situation | Exit |
|---|---|
| Clean run | 0 |
| File missing or not JSON | 1 |
| Parse error (names the question index and field) | 1 |
| Model download or load fails | 1 |
| One or more questions errored during retrieval (recorded per-question, run continues) | 1 |
| All questions filtered out by `--limit` (or a known `--question-type` absent from the file) | 1 |
| Unknown mode/model/`--question-type`, `k = 0`, or empty `--k` | 2 (clap usage error) |

Exit 1 whenever any question errored, so a partial report is never
mistaken for a clean one.

## JSON output

The Markdown report and the `--json` document share one `Report`; the
Markdown table omits a couple of fields the JSON keeps. Notable field
names, per question (`questions[]`) and per mode in the summary
(`summary.overall[mode]` / `summary.by_type[type][mode]`):

| Field | Meaning |
|---|---|
| `questions[].query_us` | Retrieval wall time per mode, in **microseconds** (not milliseconds — retrieval calls take single-digit milliseconds, so a millisecond field would quantise them badly). |
| `questions[].items_ingested` | Items (post-chunking) actually ingested for that question — the length of the `Vec<Item>` `Store::ingest_many` returned. |
| `summary...[mode].retrieve_queries_per_s` | Retrieval call throughput for that mode, excluding ingestion. Not printed in the Markdown table (see below) — read it from here. |
| `ingest_items_per_s` (top level) | Ingestion throughput across all scored questions; excludes errored questions from both the item count and the elapsed time. |

The Markdown mode table used to print a `q/s` column computed from
`queries_per_s`; that field no longer exists (renamed and moved to
`retrieve_queries_per_s`, JSON-only) because at millisecond resolution
it was usually a divide-by-near-zero artefact, not a real number.

## Quick iteration

```bash
# lexical only — no embedder, a few minutes for the full _S split
singularmem-bench longmemeval /path/to/longmemeval_s --modes lexical

# a fast, reproducible sample across all modes
singularmem-bench longmemeval /path/to/longmemeval_s --limit 100 --seed 1
```

## Full run and published numbers

See [`docs/benchmarks/longmemeval.md`](../../docs/benchmarks/longmemeval.md)
for the dataset download command, the exact full-run command, expected
wall time, and the latest results table on this machine, plus a
comparison with mempalace's published LongMemEval R@5.
