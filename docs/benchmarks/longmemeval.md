# LongMemEval retrieval benchmark

This measures retrieval quality of Singularmem's real pipeline
(`Retriever` over `HybridSearcher`) against
[LongMemEval](https://arxiv.org/abs/2410.10813). For each question,
Recall@k is the fraction of questions whose evidence session appears
among the top-k *distinct* sessions retrieved (any one of a
multi-session question's evidence sessions counts as a hit); MRR is the
mean reciprocal rank of the first evidence-session hit. Both are
reported per search mode (`lexical`, `semantic`, `hybrid`). Abstention
questions (no evidence session — the model is meant to say "I don't
know") and questions that errored during retrieval are excluded from
the scored average but counted separately in the header. A question
that is both an abstention and errored during retrieval is counted
under `errors`, not `abstention` — the reported abstention count is
therefore a floor, not necessarily the dataset's true abstention count.
Retrieval uses `min_score: 0.0`, which drops any candidate scoring
below zero; BM25 and RRF scores are always non-negative, so this floor
only ever has an effect in semantic mode, where cosine similarity can
be negative. This is **retrieval only** — it does not measure
LLM-judged answer accuracy, which is the metric LongMemEval's own
leaderboard reports.

## Getting the dataset

The dataset lives at
[huggingface.co/datasets/xiaowu0162/longmemeval](https://huggingface.co/datasets/xiaowu0162/longmemeval).
Its file listing names the `_S` split file `longmemeval_s` (no `.json`
extension — the content is JSON regardless):

```bash
curl -L -o longmemeval_s \
  https://huggingface.co/datasets/xiaowu0162/longmemeval/resolve/main/longmemeval_s
```

The file downloaded for this run was 278,025,796 bytes
(sha256 `08d8dad4be43ee2049a22ff5674eb86725d0ce5ff434cde2627e5e8e7e117894`),
matching the LFS object hash Hugging Face reports for that path. It
contains 500 questions. `singularmem-bench` reads the path directly —
rename it to `.json` for your own convenience if you like, or don't.

## Running

```bash
singularmem-bench longmemeval /path/to/longmemeval_s \
  --json longmemeval-s-<commit>.json
```

Default flags run all three modes (`lexical`, `semantic`, `hybrid`) with
`--k 1,5,10` and the `all-mini-lm-l6-v2` embedder. On the machine below
this took **73 minutes 29 seconds** for the full 500-question `_S`
split — semantic and hybrid re-embed every haystack turn per question
(each question gets an isolated store, per the design's isolation
guarantee), which dominates the wall time.

For fast iteration:

```bash
# lexical only, no embedder — a couple of minutes for the full split
singularmem-bench longmemeval /path/to/longmemeval_s --modes lexical

# a reproducible 100-question sample across all modes
singularmem-bench longmemeval /path/to/longmemeval_s --limit 100 --seed 1
```

## Results — LongMemEval_S, 2026-09-05

commit `6e79632` (fix(search): fall back to lenient query parsing for
natural-language input), model `sentence-transformers/all-MiniLM-L6-v2@v1`,
CPU Apple M2 Max (12 cores), macOS (Darwin 25.6.0).

**Lexical-only full run** — wall 02:09:

```
# LongMemEval retrieval — longmemeval_s.json

commit 6e79632  model none  dataset sha256 08d8dad4be43  questions 500 (scored 470, abstention 30, errors 0)
ingest 2320.6 items/s  wall 02:09

| mode | R@1 | R@5 | R@10 | MRR |
|---|---|---|---|---|
| lexical | 0.849 | 0.949 | 0.972 | 0.891 |

## R@5 by question type

| type | n | lexical |
|---|---|---|
| single-session-user | 64 | 1.000 |
| single-session-assistant | 56 | 1.000 |
| single-session-preference | 30 | 0.667 |
| multi-session | 121 | 0.967 |
| temporal-reasoning | 127 | 0.921 |
| knowledge-update | 72 | 1.000 |
```

**All-modes full run** — wall 73:29, exit 0, `errors 0`:

```
# LongMemEval retrieval — longmemeval_s.json

commit 6e79632  model sentence-transformers/all-MiniLM-L6-v2@v1  dataset sha256 08d8dad4be43  questions 500 (scored 470, abstention 30, errors 0)
ingest 56.4 items/s  wall 73:29

| mode | R@1 | R@5 | R@10 | MRR |
|---|---|---|---|---|
| lexical | 0.849 | 0.949 | 0.972 | 0.891 |
| semantic | 0.864 | 0.972 | 0.987 | 0.910 |
| hybrid | 0.891 | 0.981 | 0.996 | 0.929 |

## R@5 by question type

| type | n | lexical | semantic | hybrid |
|---|---|---|---|---|
| single-session-user | 64 | 1.000 | 0.969 | 0.984 |
| single-session-assistant | 56 | 1.000 | 1.000 | 1.000 |
| single-session-preference | 30 | 0.667 | 0.967 | 0.967 |
| multi-session | 121 | 0.967 | 0.975 | 0.983 |
| temporal-reasoning | 127 | 0.921 | 0.945 | 0.961 |
| knowledge-update | 72 | 1.000 | 1.000 | 1.000 |
```

Retrieval calls take a few milliseconds per question (the runs above
recorded them at millisecond granularity: roughly 1 ms lexical, 4–5 ms
semantic and hybrid on average; later runs record `query_us` in
microseconds). Wall time is dominated by ingestion, not retrieval —
2320.6 items/s and a 02:09 wall for the lexical-only run vs. 56.4
items/s and a 73:29 wall once semantic/hybrid re-embed every haystack
turn per question (both figures from each run's header above); 470
retrievals at ~5 ms is about 2 s of a 73-minute run.

Reproducibility note: after these runs, `Query::parse` in
`singularmem-search` was tightened (commit after `6e79632`) so that an
identifier-like `field:value` prefix naming an unknown field is an error
instead of being dropped. No question in `longmemeval_s` contains such a
prefix (checked over all 500 questions), so a rerun at the current
commit produces the same `errors 0`.

The two runs both scored 470/500 questions (30 abstention questions
excluded, 0 errors). Full per-question hit lists were written to
`longmemeval-s-lexical-6e79632.json` (176 KB) and
`longmemeval-s-allmodes-6e79632.json` (349 KB) on the machine that ran
this; neither file is committed to this repository (see
`crates/singularmem-bench/README.md` — `--json` output is a local
artefact, not a build product).

An earlier attempt at this same run, against commit `8f18be5`, hit 6
per-question errors — `singularmem-search`'s Tantivy query parser
rejected natural-language questions containing colons or quoted
apostrophes as syntax errors (e.g. *"I was going through our previous
conversation about The Library of Babel, and I wanted to confirm..."*).
That was fixed in `6e79632` (lenient query-parsing fallback), and the
runs above, taken against `6e79632`, show `errors 0` in both headers.

## Comparing with mempalace

mempalace's README states, under
[**Benchmarks**](https://github.com/milla-jovovich/mempalace#benchmarks)
("LongMemEval — retrieval recall (R@5, 500 questions)"):

> Raw (semantic search, no heuristics, no LLM) | **96.6%** | None

and separately:

> Hybrid v4, held-out 450q (tuned on 50 dev, not seen during training) | **98.4%** | None

Singularmem's hybrid R@5 on the same 500-question `_S` split, no LLM
rerank, is **0.981 (98.1%)** — close to mempalace's held-out hybrid
figure of 98.4% and above their no-heuristics semantic baseline of
96.6%. Singularmem's own semantic-only R@5 is 0.972 (97.2%), also above
mempalace's raw 96.6%. These numbers are **not directly comparable**:

- mempalace's raw/hybrid rows use their own chunking and retrieval
  heuristics (keyword boosting, temporal-proximity boosting,
  preference-pattern extraction); Singularmem's hybrid mode is RRF
  fusion of Tantivy (lexical) and USearch (semantic) with no
  LongMemEval-specific heuristics.
- Chunking differs: Singularmem ingests one item per conversation turn
  (`"{role}: {content}"`, chunked with `chunk_text`/`DEFAULT_CHUNK_BYTES`
  from `singularmem-ingest`); mempalace's chunking scheme is not
  specified in their README.
- Embedding models are not matched: Singularmem used
  `sentence-transformers/all-MiniLM-L6-v2`; mempalace's README does not
  name an embedding model for the 96.6%/98.4% rows.
- mempalace's 98.4% figure is a held-out 450-question subset (50
  questions used for tuning); Singularmem's figure is the full,
  untuned 500 questions.
- Both use a session-level hit definition (a hit is any evidence
  session appearing in the top-k retrieved sessions), so the *k*
  semantics should be comparable even though the pipelines are not.
- The dataset file itself differs: mempalace's README reproduces its
  numbers against `longmemeval_s_cleaned.json`; the runs in this
  document were taken against the raw `longmemeval_s` file (see
  "Getting the dataset" above), and the two are not guaranteed to
  contain identical questions or haystacks.

## Reading the numbers

Hybrid beats both lexical and semantic alone at every k and on every
question type except `single-session-assistant` and `knowledge-update`
(where all three modes already hit 1.000, so there is no room to
improve), confirming RRF fusion adds value rather than just averaging.
The biggest lexical weak spot is `single-session-preference`
(R@5 0.667) — preference questions ("what's my favorite ...")
rarely share exact wording with the answer turn, so lexical search
misses them; semantic and hybrid close that gap to 0.967. The next
things worth trying: sweep `rrf_k` to see whether it narrows the
`temporal-reasoning` gap (0.921 lexical vs 0.961 hybrid, the largest
remaining spread), and compare `bge-small-en` against
`all-mini-lm-l6-v2` for the semantic and hybrid rows since embedding
quality is the more likely lever than fusion parameters at this recall
level. (Over-fetching more candidates before truncation — raising the
retrieval `fetch_multiplier`, currently `max(For the lexical and semantic columns this lever is exhausted: every missed question already reached ten distinct sessions within the top-40 blocks fetched, so fetching more candidates cannot change their top-10. Hybrid fuses two truncated lists with RRF, so a deeper fetch can still reorder its top-10 slightly.)
