# Retrieval-quality benchmark (sub-project 15) — design

**Date:** 2026-09-05
**Status:** approved design, awaiting plan
**Programme:** mempalace parity, sub-project 15 (after 11 transcript
ingestion, 12 scoping, 13 hooks + wake-up, 14 knowledge graph)

## Goal

Give Singularmem a reproducible retrieval-quality number. The tool
evaluates the real retrieval pipeline (`Retriever` over `HybridSearcher`)
against LongMemEval and reports Recall@k and MRR per search mode, so a
change to chunking, fusion, or the embedding model can be judged by a
measurement rather than by feel. mempalace publishes LongMemEval R@5;
this tool produces the comparable figure.

It is a by-hand tool, not a CI gate. Results are published in
`docs/benchmarks/longmemeval.md` with commit, model, and machine noted.

## Non-goals

- Downloading the dataset. The user fetches the file; the tool reads a
  local path. No HTTP client, no network path (Principle VI).
- End-to-end answer accuracy (the LLM-judged LongMemEval metric). This
  tool measures retrieval only.
- Other datasets (LoCoMo, MSC). The parser is LongMemEval-specific; a
  second dataset would be a new subcommand.
- Running in CI. A synthetic fixture keeps the test suite offline and
  fast; the full run is minutes to an hour on a laptop.

## Dataset

LongMemEval (`LongMemEval_S`, `LongMemEval_M`, `LongMemEval_oracle`) is a
JSON array. Each element is one question with its own haystack:

| Field | Type | Use |
|---|---|---|
| `question_id` | string | reported per question |
| `question_type` | string | one of `single-session-user`, `single-session-assistant`, `single-session-preference`, `multi-session`, `temporal-reasoning`, `knowledge-update`; abstention questions carry an id ending in `_abs` |
| `question` | string | the query |
| `question_date` | string | recorded in JSON output only |
| `haystack_session_ids` | string[] | session ids, parallel to `haystack_sessions` |
| `haystack_dates` | string[] | session dates, parallel to `haystack_sessions` |
| `haystack_sessions` | turn[][] | each turn is `{ "role": "user" \| "assistant", "content": string, "has_answer"?: bool }` |
| `answer_session_ids` | string[] | evidence sessions; empty for abstention questions |

Fields not listed (e.g. `answer`) are ignored, so dataset revisions that
add fields still load. `haystack_session_ids`, `haystack_dates`, and
`haystack_sessions` must have equal length; a mismatch is a parse error
naming the question index.

The three files share one shape and differ only in haystack size; the
tool does not distinguish them.

## Architecture

New workspace crate `crates/singularmem-bench`, `publish = false`
(same as `singularmem-node`), binary `singularmem-bench`. Dependencies:
`singularmem-core`, `singularmem-search`, `singularmem-retrieve`,
`singularmem-ingest` (for `chunk_text` and `DEFAULT_CHUNK_BYTES`),
`clap`, `serde`, `serde_json`, `tempfile`, `jiff`.

Principle V: the crate composes library APIs; it contains no retrieval
logic of its own. If a measurement needs a knob the libraries do not
expose, the knob is added to the library, not reimplemented here.

```
crates/singularmem-bench/
  Cargo.toml
  src/main.rs        clap: `longmemeval <FILE> [flags]`
  src/dataset.rs     LongMemEval JSON -> Vec<Question>
  src/runner.rs      one Question -> per-mode hit lists (temp store)
  src/metrics.rs     hit lists -> Recall@k, MRR, per-type aggregates
  src/report.rs      Markdown table + JSON document
  tests/fixtures/longmemeval-mini.json
  tests/dataset.rs   parser
  tests/metrics.rs   hand-computed values
  tests/end_to_end.rs  fixture run through the library API
  tests/cli.rs       binary: report shape, exit codes
```

### `dataset.rs`

```rust
pub struct Question {
    pub id: String,
    pub kind: QuestionType,        // enum of the six types + Other(String)
    pub abstention: bool,          // id ends with "_abs"
    pub text: String,
    pub date: Option<String>,
    pub haystack: Vec<Session>,
    pub evidence: HashSet<String>, // answer_session_ids
}
pub struct Session { pub id: String, pub date: Option<String>, pub turns: Vec<Turn> }
pub struct Turn { pub role: String, pub content: String }

pub fn load(path: &Path) -> Result<Vec<Question>, Error>;
```

Parsing is streaming-free: `serde_json::from_reader` into serde structs
with `#[serde(default)]` on optional fields. `LongMemEval_S` is ~200 MB;
loading it whole is acceptable for a by-hand tool.

### `runner.rs`

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SearchMode { Lexical, Semantic, Hybrid }   // bench-side

pub struct RunConfig {
    pub modes: Vec<SearchMode>,          // lexical | semantic | hybrid
    pub ks: Vec<usize>,                  // sorted, deduped; max drives max_blocks
    pub model: Option<EmbeddingModel>,   // None -> lexical only allowed
}
pub struct QuestionResult {
    pub id: String,
    pub kind: QuestionType,
    pub abstention: bool,
    pub evidence: Vec<String>,
    pub hits: BTreeMap<SearchMode, Vec<String>>, // ordered distinct session ids
    pub ingest_ms: u64,
    pub query_ms: BTreeMap<SearchMode, u64>,
    pub error: Option<String>,
}
pub fn run_question(q: &Question, cfg: &RunConfig, embedder: Option<&dyn Embedder>) -> QuestionResult;
```

Per question:

1. Create a temp dir; open a fresh `Store` there, a Tantivy `Index`, and
   (when `model` is set) a `VectorIndex` with the given embedder.
   Attach both as ingest hooks the same way the CLI does.
2. For every session and turn, ingest one item per chunk:
   `text = "{role}: {content}"` chunked with
   `chunk_text(text, DEFAULT_CHUNK_BYTES)`; `source = "longmemeval"`;
   `scope = "longmemeval/{question_id}"`; metadata
   `{ "session_id", "session_index", "turn", "role", "date" }`;
   `external_id = "longmemeval:{question_id}:{session_id}:{turn}[#chunk]"`.
3. For every mode, build a `HybridSearcher` that exposes only the
   indexes the mode uses (`lexical: Some, semantic: None` for lexical;
   the reverse for semantic; both for hybrid — the search crate has no
   mode flag; the mode IS the set of attached indexes, so `SearchMode`
   is a bench-side enum), wrap it in a `Retriever`, and call
   `retrieve(question.text, &RetrieveOptions { max_blocks: max(ks) * 4, .. })`
   with `search.limit` set to the same value. The over-fetch (×4) is
   because several blocks can belong to one session; the hit list is the
   ordered list of distinct `session_id` values from the returned
   blocks, truncated to `max(ks)`.
4. Record timings. Any error inside a question is caught, stored in
   `error`, and the question's `hits` are left empty; the run continues.

Isolation per question is deliberate: LongMemEval defines a distinct
haystack per question, and a fresh store makes the measurement exact.
The cost is re-embedding sessions shared across questions; `--limit`,
`--question-type`, and `--modes lexical` exist for fast iterations.

The embedder is built once per run (model load is seconds) and shared
across questions via `&dyn Embedder`. Tests pass `MockEmbedder` from
`singularmem_search::testing`, so semantic and hybrid paths run offline.

### `metrics.rs`

```rust
pub struct ModeMetrics { pub recall: BTreeMap<usize, f64>, pub mrr: f64, pub n: usize, pub queries_per_s: f64 }
pub struct Summary {
    pub overall: BTreeMap<SearchMode, ModeMetrics>,
    pub by_type: BTreeMap<QuestionType, BTreeMap<SearchMode, ModeMetrics>>,
    pub abstentions: usize,
    pub errors: usize,
}
pub fn summarise(results: &[QuestionResult], ks: &[usize]) -> Summary;
```

- Recall@k for one question: 1.0 if any of the first k hits is in
  `evidence`, else 0.0. Averaged over scored questions.
- MRR: 1 / rank of the first evidence hit, 0 if none. Averaged.
- Scored questions exclude abstentions (no evidence) and errored
  questions; both are counted and reported.
- Multi-session questions have several evidence sessions; any one of
  them counts as a hit (this matches mempalace and the LongMemEval
  retrieval protocol).

### `report.rs`

Markdown to stdout:

```
# LongMemEval retrieval — <file basename>

commit <sha>  model <model_id or "none">  dataset sha256 <hex>  questions <n> (scored <m>, abstention <a>, errors <e>)
ingest <items/s>  wall <mm:ss>

| mode     | R@1   | R@5   | R@10  | MRR   | q/s  |
|----------|-------|-------|-------|-------|------|
| lexical  | 0.412 | 0.688 | 0.774 | 0.531 | 41.2 |
| semantic | ...   |
| hybrid   | ...   |

## R@5 by question type

| type                     | n   | lexical | semantic | hybrid |
|--------------------------|-----|---------|----------|--------|
| single-session-user      | ... |
```

Columns follow the requested `--k` values; the by-type table uses the
middle requested k (5 by default). Numbers are printed to three
decimals.

JSON (`--json <path>`):

```json
{
  "tool": "singularmem-bench", "version": "<crate version>", "commit": "<sha>",
  "dataset": { "path": "...", "sha256": "...", "questions": 500 },
  "config": { "modes": [...], "ks": [1,5,10], "model": "...", "limit": null, "question_type": null, "seed": 0 },
  "summary": { ...Summary... },
  "questions": [ ...QuestionResult... ],
  "started_at": "...", "finished_at": "..."
}
```

`questions` carries every hit list so two runs can be diffed per
question.

### `main.rs`

```
singularmem-bench longmemeval <FILE>
    --modes lexical,semantic,hybrid    default: all three
    --k 1,5,10                         default: 1,5,10
    --model all-mini-lm-l6-v2|bge-small-en|nomic-embed   default: all-mini-lm-l6-v2
    --limit N                          evaluate N questions
    --question-type T                  only this type (repeatable)
    --seed S                           shuffle seed used with --limit (default 0: file order)
    --json PATH                        write the JSON document
    --quiet                            no per-question progress on stderr
```

Model names match the `singularmem` CLI. `--modes lexical` never loads
an embedder. Progress on stderr: one line per question
(`[12/500] q_0012 multi-session  lexical hit@3  hybrid hit@1  1.9s`);
after ten questions an estimated total is printed once.

## Error handling

| Situation | Behaviour | Exit |
|---|---|---|
| File missing or not JSON | message with path, no report | 1 |
| Parse error | message naming question index and field | 1 |
| Unknown mode, model, k=0, or empty `--k` | clap error | 2 |
| Model download or load fails | message; suggest `--modes lexical` | 1 |
| Error inside one question | recorded in JSON, counted, run continues | 1 at the end |
| All questions filtered out by `--limit`/`--question-type` | message, no report | 1 |
| Clean run | report | 0 |

The exit code is 1 whenever any question errored, so a partial report
is never mistaken for a clean one (Principle VII).

## Performance

Lexical: a few minutes for the full `_S` set. Semantic and hybrid
re-embed every haystack turn (~800 per question with MiniLM at
~100–300 texts/s on a laptop CPU), so a full run is on the order of an
hour. Temp dirs are removed per question, so disk use stays at one
haystack. Memory is bounded by the loaded dataset (~1 GB for `_S` once
parsed) plus one question's store.

## Testing

All tests are offline (Principle VI).

- `tests/fixtures/longmemeval-mini.json`: six questions, three sessions
  each (two to four turns), one per question type, plus one abstention.
  Hand-written so the lexical hit ranks are known.
- `tests/dataset.rs`: loads the fixture; asserts ids, types, abstention
  flag, evidence sets, turn counts; a length-mismatch fixture yields the
  named error; unknown extra fields are ignored.
- `tests/metrics.rs`: hand-computed Recall@k and MRR for two questions
  with known hit lists; abstentions and errors excluded and counted;
  multi-evidence any-hit rule.
- `tests/end_to_end.rs`: runs the fixture through `run_question` with
  `MockEmbedder`, all three modes; asserts exact lexical Recall@1/5 on
  the fixture (the fixture is written so lexical hits are unambiguous),
  that semantic and hybrid produce hit lists of distinct session ids
  bounded by `max(k)`, and that an injected failure (an unwritable temp
  dir) becomes `error: Some(..)` without aborting the batch.
- `tests/cli.rs`: `assert_cmd` on the binary with `--modes lexical` on
  the fixture: exit 0, Markdown contains the mode row and the by-type
  table, `--json` file parses and has six questions; missing file exits
  1 with the path; `--k 0` exits 2.

No test loads a real embedding model.

## Documentation

- `docs/benchmarks/longmemeval.md`: where to download the dataset
  (Hugging Face `xiaowu0162/longmemeval`), the exact command, a results
  table from one full `_S` run on this machine with commit, model,
  CPU, and wall time, and a note on how to compare with mempalace's
  published R@5.
- `crates/singularmem-bench/README.md`: usage and flags.
- README status line gains "retrieval benchmark"; the parity memory
  gains a benchmark entry.

## Acceptance criteria

1. `cargo test -p singularmem-bench` passes offline in under a minute.
2. `singularmem-bench longmemeval longmemeval-mini.json --modes lexical`
   prints the Markdown report with exact expected R@1 and R@5.
3. A full `LongMemEval_S` run with default flags completes and its
   table is committed to `docs/benchmarks/longmemeval.md`.
4. `--json` output re-parses and contains one entry per question.
5. Any per-question error yields exit 1 and a non-empty `errors` count.
6. Workspace clippy (pedantic + nursery, `-D warnings`) and fmt clean;
   `cargo publish --dry-run` is unaffected because the crate is
   `publish = false`.

## Deviations

- Ingested items use `scope: Some("longmemeval")`, not
  `longmemeval/{question_id}`: each question gets its own throwaway store
  under a fresh temp dir (`run_question_in`), so there is no cross-question
  collision to guard against with a per-question scope segment.
- `MemoryBlock` (what `Retriever::retrieve` returns) exposes `tags`, not
  `metadata`, so the session a hit belongs to is recovered from the
  `s:{session_index}` tag (`session_index_from_tags`) rather than from
  `metadata.session_index`; `external_id` is likewise built from
  `session_index` (`longmemeval:{question_id}:{session_index}:{turn}[#chunk]`).
- `RunConfig` has no `model` field: the embedder is a runtime value
  (`Option<&SharedEmbedder>`) passed into `run_question`/`run_question_in`,
  not a config field, since which embedder to use is a caller concern
  (mock in tests, a real `FastembedEmbedder` in the CLI), not part of the
  eval configuration being measured.
- Ingestion goes through `Store::ingest_many` (one call per question,
  covering all haystack sessions), not a `Store::ingest` call per item.
  `Store::ingest` fires `on_ingest` + `commit` on every index hook per
  item (a Tantivy commit + reader reload, and a full USearch save, each
  time — roughly 90 ms/item), which made per-question ingest dominate
  runtime; `ingest_many` inserts all items in one SQLite transaction and
  fires the hooks' `on_ingest` + `commit` once at the end.
- The runner verifies index doc counts after ingest: `Store::ingest`/
  `ingest_many` only log and swallow `on_ingest`/`commit` hook failures,
  so a broken index would otherwise silently produce empty hits with
  `error: None`. After reopening the lexical (and, when in use, vector)
  index for querying, the runner checks `doc_count()` on each against the
  number of items ingested and fails the question with an error naming
  the mismatch if they disagree.
