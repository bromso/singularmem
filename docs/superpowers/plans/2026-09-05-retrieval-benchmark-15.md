# Retrieval-Quality Benchmark (sub-project 15) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `singularmem-bench` binary that evaluates the real retrieval pipeline against LongMemEval and reports Recall@k and MRR per search mode, with published results.

**Architecture:** New `publish = false` crate `crates/singularmem-bench` with four modules — `dataset` (LongMemEval JSON → `Question`), `runner` (one question → per-mode hit lists in a fresh temp store, through `Store` + `Index` + `EmbedderIndex` + `Retriever`), `metrics` (hit lists → Recall@k / MRR / per-type aggregates), `report` (Markdown + JSON) — and a clap `main`. Every question is evaluated in isolation; the embedder is built once and shared through an `Arc` wrapper.

**Tech Stack:** Rust 2021 (rust-version 1.80), clap 4.5 derive, serde/serde_json, jiff, tempfile, sha2; `singularmem-core`, `-search` (with `testing` feature for `MockEmbedder`), `-retrieve`, `-ingest` (`chunk_text`).

**Spec:** `docs/superpowers/specs/2026-09-05-retrieval-benchmark-15-design.md` — the contract. Where this plan and the spec differ, the spec governs; record deviations in the spec's "Deviations" section.

## Global Constraints

- Crate is `publish = false`; `[lints] workspace = true`; clippy pedantic + nursery `-D warnings` and `cargo fmt --all` clean at every commit.
- No network in any test; no test loads a real embedding model. Tests use `singularmem_search::testing::MockEmbedder` (feature `testing`).
- The binary honours `SINGULARMEM_TEST_EMBEDDER=mock` exactly as the `singularmem` CLI does (mock embedder instead of fastembed), so CLI tests can exercise semantic/hybrid offline.
- Model names on the command line are exactly `all-mini-lm-l6-v2`, `bge-small-en`, `nomic-embed` (same as the CLI); default `all-mini-lm-l6-v2`.
- Mode names are exactly `lexical`, `semantic`, `hybrid`; default all three, in that order.
- Item text per turn is `"{role}: {content}"`, chunked with `singularmem_ingest::chunk_text(text, DEFAULT_CHUNK_BYTES)`; `source = Some("longmemeval")`; `scope = Some("longmemeval")`; tag `s:{session_index}`; `external_id = "longmemeval:{question_id}:{session_index}:{turn}"` plus `#{chunk}` when a turn produced more than one chunk; metadata `{"session_id","session_index","turn","role","date"}`.
- Hit list per mode = ordered distinct session ids of returned blocks (via the `s:{index}` tag), truncated to `max(ks)`; retrieval asks for `max_blocks = max(ks) * 4` and `search.limit = max(ks) * 4`.
- Recall@k = 1.0 if any of the first k hits ∈ evidence; MRR = 1 / rank of first evidence hit, 0 if none. Abstention questions (id ends with `_abs`) and errored questions are excluded from averages and counted.
- Numbers in the Markdown report are printed with three decimals.
- Exit codes: 0 clean; 1 for file/parse/model errors, when any question errored, or when no questions remain after filtering; 2 for clap errors (unknown mode/model, `--k 0`, empty `--k`).
- Every commit signed off (`git commit -s`); never stage `.superpowers/`, `.agents/`, `.claude/`, `skills-lock.json`, `*.proptest-regressions`, `*.node`.

## File Structure

```
crates/singularmem-bench/
  Cargo.toml
  README.md                     (Task 5)
  src/main.rs                   clap + run loop + exit codes (Task 4)
  src/lib.rs                    pub mod dataset, metrics, runner, report (Task 1; grows)
  src/dataset.rs                (Task 1)
  src/metrics.rs                (Task 2)
  src/runner.rs                 (Task 3)
  src/report.rs                 (Task 4)
  tests/fixtures/longmemeval-mini.json   (Task 1)
  tests/fixtures/longmemeval-bad-lengths.json (Task 1)
  tests/dataset.rs              (Task 1)
  tests/metrics.rs              (Task 2)
  tests/end_to_end.rs           (Task 3)
  tests/cli.rs                  (Task 4)
docs/benchmarks/longmemeval.md  (Task 5)
README.md                       status line (Task 5)
```

The crate is a lib + bin so integration tests can call `runner::run_question` directly.

---

### Task 1: Crate scaffold, dataset parser, fixture

**Files:**
- Create: `crates/singularmem-bench/Cargo.toml`
- Create: `crates/singularmem-bench/src/lib.rs`
- Create: `crates/singularmem-bench/src/main.rs` (stub)
- Create: `crates/singularmem-bench/src/dataset.rs`
- Create: `crates/singularmem-bench/tests/fixtures/longmemeval-mini.json`
- Create: `crates/singularmem-bench/tests/fixtures/longmemeval-bad-lengths.json`
- Test: `crates/singularmem-bench/tests/dataset.rs`

**Interfaces:**
- Produces: `dataset::{load, Question, Session, Turn, QuestionType, Error}` as defined below. `QuestionType` is `Copy`-free but `Clone + Eq + Ord + Hash + Serialize + Deserialize + Display`.

- [ ] **Step 1: Cargo.toml and stubs**

`crates/singularmem-bench/Cargo.toml`:

```toml
[package]
name = "singularmem-bench"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
description = "Retrieval-quality benchmark harness for Singularmem (LongMemEval)."
publish = false

[lints]
workspace = true

[[bin]]
name = "singularmem-bench"
path = "src/main.rs"

[dependencies]
singularmem-core = { path = "../singularmem-core", version = "0.16.0" }
singularmem-search = { path = "../singularmem-search", version = "0.16.0", features = ["testing"] }
singularmem-retrieve = { path = "../singularmem-retrieve", version = "0.16.0" }
singularmem-ingest = { path = "../singularmem-ingest", version = "0.16.0" }
clap = { version = "4.5", features = ["derive", "wrap_help"] }
serde = { workspace = true }
serde_json = { workspace = true }
jiff = { workspace = true }
tempfile = { workspace = true }
thiserror = { workspace = true }
sha2 = "0.10"
rand = "0.8"

[dev-dependencies]
assert_cmd = { workspace = true }
predicates = { workspace = true }
```

`crates/singularmem-bench/src/lib.rs`:

```rust
//! Retrieval-quality benchmark harness. See
//! `docs/superpowers/specs/2026-09-05-retrieval-benchmark-15-design.md`.

pub mod dataset;
```

`crates/singularmem-bench/src/main.rs` (stub, replaced in Task 4):

```rust
fn main() {
    eprintln!("singularmem-bench: not yet wired");
    std::process::exit(2);
}
```

Run: `cargo build -p singularmem-bench` — Expected: builds (warning-free).

- [ ] **Step 2: Fixture**

`crates/singularmem-bench/tests/fixtures/longmemeval-mini.json`. Six questions, three sessions each. Evidence sessions contain the question's distinctive words; distractor sessions share no non-stopword with the question. `q5` (temporal-reasoning) is a deliberate lexical miss: its evidence session paraphrases without any query term. `q6_abs` is an abstention (empty `answer_session_ids`).

```json
[
  {
    "question_id": "q1",
    "question_type": "single-session-user",
    "question": "What breed is my dog Biscuit?",
    "question_date": "2024/03/01 (Fri) 10:00",
    "haystack_session_ids": ["s1a", "s1b", "s1c"],
    "haystack_dates": ["2024/02/01 (Thu) 09:00", "2024/02/02 (Fri) 09:00", "2024/02/03 (Sat) 09:00"],
    "haystack_sessions": [
      [
        {"role": "user", "content": "We adopted a corgi last week and named him Biscuit."},
        {"role": "assistant", "content": "Congratulations on adopting Biscuit the corgi!"}
      ],
      [
        {"role": "user", "content": "Can you suggest a recipe for lentil soup?"},
        {"role": "assistant", "content": "Simmer red lentils with onion, cumin and stock for twenty minutes."}
      ],
      [
        {"role": "user", "content": "How do I renew a passport online?"},
        {"role": "assistant", "content": "Use the government portal and upload a recent photo."}
      ]
    ],
    "answer_session_ids": ["s1a"],
    "answer": "corgi"
  },
  {
    "question_id": "q2",
    "question_type": "single-session-assistant",
    "question": "Which sourdough hydration percentage did you recommend?",
    "question_date": "2024/03/01 (Fri) 10:00",
    "haystack_session_ids": ["s2a", "s2b", "s2c"],
    "haystack_dates": ["2024/02/01 (Thu) 09:00", "2024/02/02 (Fri) 09:00", "2024/02/03 (Sat) 09:00"],
    "haystack_sessions": [
      [
        {"role": "user", "content": "Any tips for planting tulip bulbs?"},
        {"role": "assistant", "content": "Plant tulip bulbs in autumn, pointed end up, three times their depth."}
      ],
      [
        {"role": "user", "content": "My bread is dense. What should I change?"},
        {"role": "assistant", "content": "Try a sourdough hydration of seventy-eight percent and a longer bulk ferment."}
      ],
      [
        {"role": "user", "content": "Explain the offside rule in football."},
        {"role": "assistant", "content": "A player is offside when nearer the goal line than the ball and the second-last defender."}
      ]
    ],
    "answer_session_ids": ["s2b"],
    "answer": "seventy-eight percent"
  },
  {
    "question_id": "q3",
    "question_type": "single-session-preference",
    "question": "Recommend a holiday given my preference for quiet mountain villages.",
    "question_date": "2024/03/01 (Fri) 10:00",
    "haystack_session_ids": ["s3a", "s3b", "s3c"],
    "haystack_dates": ["2024/02/01 (Thu) 09:00", "2024/02/02 (Fri) 09:00", "2024/02/03 (Sat) 09:00"],
    "haystack_sessions": [
      [
        {"role": "user", "content": "How many litres are in a gallon?"},
        {"role": "assistant", "content": "A US gallon is about 3.785 litres."}
      ],
      [
        {"role": "user", "content": "Write a limerick about a cat."},
        {"role": "assistant", "content": "There once was a cat from Peru..."}
      ],
      [
        {"role": "user", "content": "I always prefer quiet mountain villages over busy beaches for a holiday."},
        {"role": "assistant", "content": "Noted: quiet mountain villages it is."}
      ]
    ],
    "answer_session_ids": ["s3c"],
    "answer": "quiet mountain villages"
  },
  {
    "question_id": "q4",
    "question_type": "multi-session",
    "question": "How many marathon training runs did I log in total?",
    "question_date": "2024/03/01 (Fri) 10:00",
    "haystack_session_ids": ["s4a", "s4b", "s4c"],
    "haystack_dates": ["2024/02/01 (Thu) 09:00", "2024/02/02 (Fri) 09:00", "2024/02/03 (Sat) 09:00"],
    "haystack_sessions": [
      [
        {"role": "user", "content": "Logging a marathon training run: 12 km today."},
        {"role": "assistant", "content": "Logged 12 km for marathon training."}
      ],
      [
        {"role": "user", "content": "What is the capital of Mongolia?"},
        {"role": "assistant", "content": "Ulaanbaatar."}
      ],
      [
        {"role": "user", "content": "Another marathon training run, 18 km this time."},
        {"role": "assistant", "content": "Logged 18 km for marathon training."}
      ]
    ],
    "answer_session_ids": ["s4a", "s4c"],
    "answer": "two runs, 30 km"
  },
  {
    "question_id": "q5",
    "question_type": "temporal-reasoning",
    "question": "How long after the kitchen renovation did the leak appear?",
    "question_date": "2024/03/01 (Fri) 10:00",
    "haystack_session_ids": ["s5a", "s5b", "s5c"],
    "haystack_dates": ["2024/02/01 (Thu) 09:00", "2024/02/02 (Fri) 09:00", "2024/02/03 (Sat) 09:00"],
    "haystack_sessions": [
      [
        {"role": "user", "content": "The builders finished remodelling our cooking area in January."},
        {"role": "assistant", "content": "Glad the remodel is complete."}
      ],
      [
        {"role": "user", "content": "Which planet has the most moons?"},
        {"role": "assistant", "content": "Saturn, with over one hundred confirmed moons."}
      ],
      [
        {"role": "user", "content": "Translate 'good morning' into Swedish."},
        {"role": "assistant", "content": "God morgon."}
      ]
    ],
    "answer_session_ids": ["s5a"],
    "answer": "unknown"
  },
  {
    "question_id": "q6_abs",
    "question_type": "knowledge-update",
    "question": "What is the new name of my company?",
    "question_date": "2024/03/01 (Fri) 10:00",
    "haystack_session_ids": ["s6a", "s6b", "s6c"],
    "haystack_dates": ["2024/02/01 (Thu) 09:00", "2024/02/02 (Fri) 09:00", "2024/02/03 (Sat) 09:00"],
    "haystack_sessions": [
      [
        {"role": "user", "content": "Draft a polite out-of-office reply."},
        {"role": "assistant", "content": "Thank you for your message; I am away until Monday."}
      ],
      [
        {"role": "user", "content": "How do I center a div?"},
        {"role": "assistant", "content": "Use flexbox with justify-content and align-items set to center."}
      ],
      [
        {"role": "user", "content": "Suggest three names for a goldfish."},
        {"role": "assistant", "content": "Bubbles, Captain, and Nemo."}
      ]
    ],
    "answer_session_ids": [],
    "answer": "N/A"
  }
]
```

`crates/singularmem-bench/tests/fixtures/longmemeval-bad-lengths.json` (two ids, one session):

```json
[
  {
    "question_id": "bad1",
    "question_type": "single-session-user",
    "question": "x",
    "haystack_session_ids": ["a", "b"],
    "haystack_dates": ["2024/02/01 (Thu) 09:00"],
    "haystack_sessions": [[{"role": "user", "content": "hello"}]],
    "answer_session_ids": ["a"]
  }
]
```

- [ ] **Step 3: Failing parser tests**

`crates/singularmem-bench/tests/dataset.rs`:

```rust
use std::collections::HashSet;
use std::path::PathBuf;

use singularmem_bench::dataset::{load, Error, QuestionType};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn loads_the_mini_fixture() {
    let qs = load(&fixture("longmemeval-mini.json")).unwrap();
    assert_eq!(qs.len(), 6);

    let q1 = &qs[0];
    assert_eq!(q1.id, "q1");
    assert_eq!(q1.kind, QuestionType::SingleSessionUser);
    assert!(!q1.abstention);
    assert_eq!(q1.text, "What breed is my dog Biscuit?");
    assert_eq!(q1.date.as_deref(), Some("2024/03/01 (Fri) 10:00"));
    assert_eq!(q1.haystack.len(), 3);
    assert_eq!(q1.haystack[0].id, "s1a");
    assert_eq!(q1.haystack[0].date.as_deref(), Some("2024/02/01 (Thu) 09:00"));
    assert_eq!(q1.haystack[0].turns.len(), 2);
    assert_eq!(q1.haystack[0].turns[0].role, "user");
    assert_eq!(q1.evidence, HashSet::from(["s1a".to_string()]));

    let q4 = &qs[3];
    assert_eq!(q4.kind, QuestionType::MultiSession);
    assert_eq!(
        q4.evidence,
        HashSet::from(["s4a".to_string(), "s4c".to_string()])
    );

    let q6 = &qs[5];
    assert_eq!(q6.kind, QuestionType::KnowledgeUpdate);
    assert!(q6.abstention, "id ending in _abs is an abstention");
    assert!(q6.evidence.is_empty());

    let kinds: Vec<_> = qs.iter().map(|q| q.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            QuestionType::SingleSessionUser,
            QuestionType::SingleSessionAssistant,
            QuestionType::SingleSessionPreference,
            QuestionType::MultiSession,
            QuestionType::TemporalReasoning,
            QuestionType::KnowledgeUpdate,
        ]
    );
}

#[test]
fn unknown_question_type_is_preserved_not_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.json");
    std::fs::write(
        &p,
        r#"[{"question_id":"z","question_type":"brand-new-type","question":"?",
             "haystack_session_ids":["a"],"haystack_dates":["d"],
             "haystack_sessions":[[{"role":"user","content":"c","has_answer":true,"extra":1}]],
             "answer_session_ids":["a"],"unexpected_top_level":true}]"#,
    )
    .unwrap();
    let qs = load(&p).unwrap();
    assert_eq!(qs[0].kind, QuestionType::Other("brand-new-type".into()));
    assert_eq!(qs[0].kind.to_string(), "brand-new-type");
    assert_eq!(qs[0].haystack[0].turns[0].content, "c");
}

#[test]
fn parallel_array_length_mismatch_names_the_question() {
    let err = load(&fixture("longmemeval-bad-lengths.json")).unwrap_err();
    match err {
        Error::Shape { index, field, .. } => {
            assert_eq!(index, 0);
            assert_eq!(field, "haystack_dates");
        }
        other => panic!("expected Shape, got {other:?}"),
    }
    assert!(err.to_string().contains("question 0"), "{err}");
}

#[test]
fn missing_file_is_an_io_error_with_the_path() {
    let err = load(&fixture("does-not-exist.json")).unwrap_err();
    assert!(matches!(err, Error::Io { .. }));
    assert!(err.to_string().contains("does-not-exist.json"));
}

#[test]
fn question_type_display_and_parse_round_trip() {
    for (name, kind) in [
        ("single-session-user", QuestionType::SingleSessionUser),
        ("single-session-assistant", QuestionType::SingleSessionAssistant),
        ("single-session-preference", QuestionType::SingleSessionPreference),
        ("multi-session", QuestionType::MultiSession),
        ("temporal-reasoning", QuestionType::TemporalReasoning),
        ("knowledge-update", QuestionType::KnowledgeUpdate),
    ] {
        assert_eq!(QuestionType::from(name), kind);
        assert_eq!(kind.to_string(), name);
    }
}
```

Run: `cargo test -p singularmem-bench --test dataset` — Expected: compile error (`dataset` module empty).

- [ ] **Step 4: Implement `dataset.rs`**

```rust
//! LongMemEval dataset loader. One JSON array; each element is a question
//! with its own haystack of sessions. Unknown fields are ignored.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// LongMemEval question categories. Unknown strings are preserved in
/// [`QuestionType::Other`] so a new dataset revision still loads.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum QuestionType {
    SingleSessionUser,
    SingleSessionAssistant,
    SingleSessionPreference,
    MultiSession,
    TemporalReasoning,
    KnowledgeUpdate,
    Other(String),
}

impl QuestionType {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::SingleSessionUser => "single-session-user",
            Self::SingleSessionAssistant => "single-session-assistant",
            Self::SingleSessionPreference => "single-session-preference",
            Self::MultiSession => "multi-session",
            Self::TemporalReasoning => "temporal-reasoning",
            Self::KnowledgeUpdate => "knowledge-update",
            Self::Other(s) => s,
        }
    }
}

impl From<&str> for QuestionType {
    fn from(s: &str) -> Self {
        match s {
            "single-session-user" => Self::SingleSessionUser,
            "single-session-assistant" => Self::SingleSessionAssistant,
            "single-session-preference" => Self::SingleSessionPreference,
            "multi-session" => Self::MultiSession,
            "temporal-reasoning" => Self::TemporalReasoning,
            "knowledge-update" => Self::KnowledgeUpdate,
            other => Self::Other(other.to_string()),
        }
    }
}

impl From<String> for QuestionType {
    fn from(s: String) -> Self {
        Self::from(s.as_str())
    }
}

impl From<QuestionType> for String {
    fn from(k: QuestionType) -> Self {
        k.as_str().to_string()
    }
}

impl fmt::Display for QuestionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub date: Option<String>,
    pub turns: Vec<Turn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub kind: QuestionType,
    /// `question_id` ends with `_abs`: no evidence session exists.
    pub abstention: bool,
    pub text: String,
    pub date: Option<String>,
    pub haystack: Vec<Session>,
    pub evidence: HashSet<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not a LongMemEval JSON array: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("question {index} ({id}): {field} has {actual} entries, expected {expected}")]
    Shape {
        index: usize,
        id: String,
        field: &'static str,
        expected: usize,
        actual: usize,
    },
}

#[derive(Deserialize)]
struct RawTurn {
    #[serde(default)]
    role: String,
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct RawQuestion {
    question_id: String,
    #[serde(default)]
    question_type: String,
    #[serde(default)]
    question: String,
    #[serde(default)]
    question_date: Option<String>,
    #[serde(default)]
    haystack_session_ids: Vec<String>,
    #[serde(default)]
    haystack_dates: Vec<String>,
    #[serde(default)]
    haystack_sessions: Vec<Vec<RawTurn>>,
    #[serde(default)]
    answer_session_ids: Vec<String>,
}

/// Load a LongMemEval file.
///
/// # Errors
/// [`Error::Io`] when the file cannot be read, [`Error::Json`] when it is
/// not the expected array, [`Error::Shape`] when a question's parallel
/// haystack arrays disagree in length.
pub fn load(path: &Path) -> Result<Vec<Question>, Error> {
    let shown = path.display().to_string();
    let file = std::fs::File::open(path).map_err(|source| Error::Io {
        path: shown.clone(),
        source,
    })?;
    let reader = std::io::BufReader::new(file);
    let raw: Vec<RawQuestion> = serde_json::from_reader(reader).map_err(|source| Error::Json {
        path: shown,
        source,
    })?;
    raw.into_iter().enumerate().map(|(i, q)| convert(i, q)).collect()
}

fn convert(index: usize, raw: RawQuestion) -> Result<Question, Error> {
    let n = raw.haystack_sessions.len();
    let check = |field: &'static str, actual: usize| {
        if actual == n {
            Ok(())
        } else {
            Err(Error::Shape {
                index,
                id: raw.question_id.clone(),
                field,
                expected: n,
                actual,
            })
        }
    };
    check("haystack_session_ids", raw.haystack_session_ids.len())?;
    check("haystack_dates", raw.haystack_dates.len())?;

    let haystack = raw
        .haystack_session_ids
        .into_iter()
        .zip(raw.haystack_dates)
        .zip(raw.haystack_sessions)
        .map(|((id, date), turns)| Session {
            id,
            date: Some(date),
            turns: turns
                .into_iter()
                .map(|t| Turn {
                    role: t.role,
                    content: t.content,
                })
                .collect(),
        })
        .collect();

    Ok(Question {
        abstention: raw.question_id.ends_with("_abs"),
        id: raw.question_id,
        kind: QuestionType::from(raw.question_type),
        text: raw.question,
        date: raw.question_date,
        haystack,
        evidence: raw.answer_session_ids.into_iter().collect(),
    })
}
```

Note the `Shape` Display contains `question {index}`; the test asserts `"question 0"`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p singularmem-bench --test dataset` — Expected: 5 passed.
Run: `cargo clippy -p singularmem-bench --all-targets --all-features -- -D warnings && cargo fmt --all -- --check` — Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/singularmem-bench/Cargo.toml crates/singularmem-bench/src crates/singularmem-bench/tests Cargo.lock
git commit -s -m "feat(bench): singularmem-bench crate with LongMemEval loader"
```

---

### Task 2: Metrics

**Files:**
- Create: `crates/singularmem-bench/src/metrics.rs`
- Modify: `crates/singularmem-bench/src/lib.rs` (add `pub mod metrics;`)
- Test: `crates/singularmem-bench/tests/metrics.rs`

**Interfaces:**
- Consumes: `dataset::QuestionType`.
- Produces: `metrics::{SearchMode, QuestionResult, ModeMetrics, Summary, summarise}`. `SearchMode` lives here (not in `runner`) so metrics has no dependency on the runner; `runner` re-exports it.

- [ ] **Step 1: Failing tests**

`crates/singularmem-bench/tests/metrics.rs`:

```rust
use std::collections::BTreeMap;

use singularmem_bench::dataset::QuestionType;
use singularmem_bench::metrics::{summarise, QuestionResult, SearchMode};

fn result(
    id: &str,
    kind: QuestionType,
    abstention: bool,
    evidence: &[&str],
    hits: &[&str],
    error: Option<&str>,
) -> QuestionResult {
    let mut h = BTreeMap::new();
    h.insert(
        SearchMode::Lexical,
        hits.iter().map(ToString::to_string).collect::<Vec<_>>(),
    );
    let mut q = BTreeMap::new();
    q.insert(SearchMode::Lexical, 100_u64);
    QuestionResult {
        id: id.into(),
        kind,
        abstention,
        evidence: evidence.iter().map(ToString::to_string).collect(),
        hits: h,
        ingest_ms: 10,
        query_ms: q,
        error: error.map(ToString::to_string),
    }
}

#[test]
fn recall_and_mrr_by_hand() {
    // q1: evidence at rank 1 -> R@1=1, R@5=1, RR=1
    // q2: evidence at rank 3 -> R@1=0, R@5=1, RR=1/3
    // q3: miss              -> 0, 0, 0
    let rs = vec![
        result("q1", QuestionType::MultiSession, false, &["a"], &["a", "x", "y"], None),
        result("q2", QuestionType::MultiSession, false, &["b"], &["x", "y", "b"], None),
        result("q3", QuestionType::KnowledgeUpdate, false, &["c"], &["x", "y", "z"], None),
    ];
    let s = summarise(&rs, &[1, 5]);
    let m = &s.overall[&SearchMode::Lexical];
    assert_eq!(m.n, 3);
    assert!((m.recall[&1] - 1.0 / 3.0).abs() < 1e-9);
    assert!((m.recall[&5] - 2.0 / 3.0).abs() < 1e-9);
    assert!((m.mrr - (1.0 + 1.0 / 3.0) / 3.0).abs() < 1e-9);
    // 3 queries in 300 ms -> 10 q/s
    assert!((m.queries_per_s - 10.0).abs() < 1e-9);

    let multi = &s.by_type[&QuestionType::MultiSession][&SearchMode::Lexical];
    assert_eq!(multi.n, 2);
    assert!((multi.recall[&5] - 1.0).abs() < 1e-9);
    assert_eq!(s.abstentions, 0);
    assert_eq!(s.errors, 0);
}

#[test]
fn multi_evidence_any_hit_counts() {
    let rs = vec![result(
        "q",
        QuestionType::MultiSession,
        false,
        &["a", "b"],
        &["b", "z"],
        None,
    )];
    let s = summarise(&rs, &[1]);
    assert!((s.overall[&SearchMode::Lexical].recall[&1] - 1.0).abs() < 1e-9);
}

#[test]
fn abstentions_and_errors_are_excluded_and_counted() {
    let rs = vec![
        result("q1", QuestionType::SingleSessionUser, false, &["a"], &["a"], None),
        result("q2_abs", QuestionType::SingleSessionUser, true, &[], &["a"], None),
        result("q3", QuestionType::SingleSessionUser, false, &["a"], &[], Some("boom")),
    ];
    let s = summarise(&rs, &[1]);
    let m = &s.overall[&SearchMode::Lexical];
    assert_eq!(m.n, 1, "only q1 is scored");
    assert!((m.recall[&1] - 1.0).abs() < 1e-9);
    assert_eq!(s.abstentions, 1);
    assert_eq!(s.errors, 1);
}

#[test]
fn empty_input_yields_zero_metrics_not_nan() {
    let s = summarise(&[], &[1, 5]);
    assert!(s.overall.is_empty());
    assert_eq!(s.abstentions, 0);
}

#[test]
fn search_mode_names_round_trip() {
    for (name, mode) in [
        ("lexical", SearchMode::Lexical),
        ("semantic", SearchMode::Semantic),
        ("hybrid", SearchMode::Hybrid),
    ] {
        assert_eq!(mode.as_str(), name);
        assert_eq!(name.parse::<SearchMode>().unwrap(), mode);
    }
    assert!("fuzzy".parse::<SearchMode>().is_err());
}
```

Run: `cargo test -p singularmem-bench --test metrics` — Expected: compile error.

- [ ] **Step 2: Implement `metrics.rs`**

```rust
//! Recall@k and MRR over per-question hit lists.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::dataset::QuestionType;

/// Which indexes a retrieval used. The search crate has no mode flag —
/// the mode is the set of indexes attached to the `HybridSearcher` — so
/// this enum is bench-side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    Lexical,
    Semantic,
    Hybrid,
}

impl SearchMode {
    pub const ALL: [Self; 3] = [Self::Lexical, Self::Semantic, Self::Hybrid];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lexical => "lexical",
            Self::Semantic => "semantic",
            Self::Hybrid => "hybrid",
        }
    }

    /// True when this mode needs an embedder.
    #[must_use]
    pub const fn needs_embedder(self) -> bool {
        !matches!(self, Self::Lexical)
    }
}

impl fmt::Display for SearchMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SearchMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "lexical" => Ok(Self::Lexical),
            "semantic" => Ok(Self::Semantic),
            "hybrid" => Ok(Self::Hybrid),
            other => Err(format!(
                "unknown mode {other:?}; expected lexical, semantic or hybrid"
            )),
        }
    }
}

/// One evaluated question. `hits[mode]` is the ordered list of distinct
/// session ids returned for that mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionResult {
    pub id: String,
    pub kind: QuestionType,
    pub abstention: bool,
    pub evidence: Vec<String>,
    pub hits: BTreeMap<SearchMode, Vec<String>>,
    pub ingest_ms: u64,
    pub query_ms: BTreeMap<SearchMode, u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeMetrics {
    /// Recall@k keyed by k.
    pub recall: BTreeMap<usize, f64>,
    pub mrr: f64,
    /// Scored questions.
    pub n: usize,
    pub queries_per_s: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub overall: BTreeMap<SearchMode, ModeMetrics>,
    pub by_type: BTreeMap<QuestionType, BTreeMap<SearchMode, ModeMetrics>>,
    pub abstentions: usize,
    pub errors: usize,
}

/// Rank (1-based) of the first evidence session in `hits`, if any.
fn first_hit_rank(hits: &[String], evidence: &[String]) -> Option<usize> {
    hits.iter()
        .position(|h| evidence.iter().any(|e| e == h))
        .map(|p| p + 1)
}

struct Acc {
    hit_at: BTreeMap<usize, usize>,
    rr_sum: f64,
    n: usize,
    query_ms: u64,
}

impl Acc {
    fn new(ks: &[usize]) -> Self {
        Self {
            hit_at: ks.iter().map(|&k| (k, 0)).collect(),
            rr_sum: 0.0,
            n: 0,
            query_ms: 0,
        }
    }

    fn add(&mut self, rank: Option<usize>, query_ms: u64) {
        self.n += 1;
        self.query_ms += query_ms;
        if let Some(r) = rank {
            self.rr_sum += 1.0 / r as f64;
            for (k, count) in &mut self.hit_at {
                if r <= *k {
                    *count += 1;
                }
            }
        }
    }

    fn finish(self) -> ModeMetrics {
        let n = self.n as f64;
        ModeMetrics {
            recall: self
                .hit_at
                .into_iter()
                .map(|(k, c)| (k, c as f64 / n))
                .collect(),
            mrr: self.rr_sum / n,
            n: self.n,
            queries_per_s: if self.query_ms == 0 {
                0.0
            } else {
                n / (self.query_ms as f64 / 1000.0)
            },
        }
    }
}

/// Aggregate results into overall and per-type metrics. Abstention and
/// errored questions are excluded from averages and counted.
#[must_use]
pub fn summarise(results: &[QuestionResult], ks: &[usize]) -> Summary {
    let mut overall: BTreeMap<SearchMode, Acc> = BTreeMap::new();
    let mut by_type: BTreeMap<QuestionType, BTreeMap<SearchMode, Acc>> = BTreeMap::new();
    let mut abstentions = 0;
    let mut errors = 0;

    for r in results {
        if r.error.is_some() {
            errors += 1;
            continue;
        }
        if r.abstention {
            abstentions += 1;
            continue;
        }
        for (mode, hits) in &r.hits {
            let rank = first_hit_rank(hits, &r.evidence);
            let ms = r.query_ms.get(mode).copied().unwrap_or(0);
            overall
                .entry(*mode)
                .or_insert_with(|| Acc::new(ks))
                .add(rank, ms);
            by_type
                .entry(r.kind.clone())
                .or_default()
                .entry(*mode)
                .or_insert_with(|| Acc::new(ks))
                .add(rank, ms);
        }
    }

    Summary {
        overall: overall.into_iter().map(|(m, a)| (m, a.finish())).collect(),
        by_type: by_type
            .into_iter()
            .map(|(t, modes)| (t, modes.into_iter().map(|(m, a)| (m, a.finish())).collect()))
            .collect(),
        abstentions,
        errors,
    }
}
```

Clippy pedantic flags `as f64` casts (`cast_precision_loss`); add `#[allow(clippy::cast_precision_loss)]` on `Acc::add` and `Acc::finish` with a one-line reason comment (counts are far below 2^53).

Add `pub mod metrics;` to `lib.rs`.

- [ ] **Step 3: Run tests, lint, commit**

Run: `cargo test -p singularmem-bench --test metrics` — Expected: 5 passed.
Run: `cargo clippy -p singularmem-bench --all-targets --all-features -- -D warnings && cargo fmt --all -- --check`.

```bash
git add crates/singularmem-bench/src/lib.rs crates/singularmem-bench/src/metrics.rs crates/singularmem-bench/tests/metrics.rs
git commit -s -m "feat(bench): recall@k and MRR aggregation"
```

---

### Task 3: Runner (per-question evaluation)

**Files:**
- Create: `crates/singularmem-bench/src/runner.rs`
- Modify: `crates/singularmem-bench/src/lib.rs` (add `pub mod runner;`)
- Test: `crates/singularmem-bench/tests/end_to_end.rs`

**Interfaces:**
- Consumes: `dataset::{Question, Session}`, `metrics::{SearchMode, QuestionResult}`; library APIs `singularmem_core::{Store, NewItem, hook::MultiHook}`, `singularmem_search::{Index, EmbedderIndex, Embedder, HybridSearcher, HybridSearchOptions}`, `singularmem_retrieve::{Retriever, RetrieveOptions}`, `singularmem_ingest::{chunk_text, DEFAULT_CHUNK_BYTES}`.
- Produces: `runner::{RunConfig, SharedEmbedder, run_question}`.

Design notes for the implementer:
- `EmbedderIndex::open(dir, Box<dyn Embedder>)` takes ownership, and each question opens the vector index twice (once as an ingest hook, once for searching). `SharedEmbedder(Arc<dyn Embedder>)` implements `Embedder` by delegation so one model instance serves every open.
- Ingest-then-reopen: hooks are boxed into the store; after ingesting, drop the store (which commits the hooks), then reopen `Store`, `Index`, and `EmbedderIndex` read paths for searching. This is the pattern `singularmem-retrieve`'s own tests use.
- Session identity comes back through the item tag `s:{session_index}`; `MemoryBlock` exposes `tags` but not metadata.

- [ ] **Step 1: Failing end-to-end tests**

`crates/singularmem-bench/tests/end_to_end.rs`:

```rust
use std::path::PathBuf;
use std::sync::Arc;

use singularmem_bench::dataset::load;
use singularmem_bench::metrics::{summarise, SearchMode};
use singularmem_bench::runner::{run_question, RunConfig, SharedEmbedder};
use singularmem_search::testing::MockEmbedder;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/longmemeval-mini.json")
}

fn mock() -> SharedEmbedder {
    SharedEmbedder::new(Arc::new(MockEmbedder::default()))
}

#[test]
fn lexical_hits_on_the_fixture_are_exact() {
    let qs = load(&fixture()).unwrap();
    let cfg = RunConfig {
        modes: vec![SearchMode::Lexical],
        ks: vec![1, 5],
    };
    let results: Vec<_> = qs.iter().map(|q| run_question(q, &cfg, None)).collect();
    for r in &results {
        assert!(r.error.is_none(), "{}: {:?}", r.id, r.error);
        assert!(r.ingest_ms < 60_000, "{}: ingest took {} ms", r.id, r.ingest_ms);
    }
    // q1..q4 hit at rank 1; q5 is a lexical miss; q6_abs is an abstention.
    assert_eq!(results[0].hits[&SearchMode::Lexical][0], "s1a");
    assert_eq!(results[1].hits[&SearchMode::Lexical][0], "s2b");
    assert_eq!(results[2].hits[&SearchMode::Lexical][0], "s3c");
    let q4 = &results[3].hits[&SearchMode::Lexical][0];
    assert!(q4 == "s4a" || q4 == "s4c", "{q4}");
    assert!(
        !results[4].hits[&SearchMode::Lexical].contains(&"s5a".to_string()),
        "q5 is written to be a lexical miss: {:?}",
        results[4].hits
    );
    assert!(results[5].abstention);

    let s = summarise(&results, &cfg.ks);
    let m = &s.overall[&SearchMode::Lexical];
    assert_eq!(m.n, 5);
    assert!((m.recall[&1] - 0.8).abs() < 1e-9, "{:?}", m.recall);
    assert!((m.recall[&5] - 0.8).abs() < 1e-9, "{:?}", m.recall);
    assert!((m.mrr - 0.8).abs() < 1e-9);
    assert_eq!(s.abstentions, 1);
    assert_eq!(s.errors, 0);
}

#[test]
fn hit_lists_are_distinct_sessions_bounded_by_max_k() {
    let qs = load(&fixture()).unwrap();
    let cfg = RunConfig {
        modes: vec![SearchMode::Lexical, SearchMode::Semantic, SearchMode::Hybrid],
        ks: vec![1, 2],
    };
    let emb = mock();
    let r = run_question(&qs[0], &cfg, Some(&emb));
    assert!(r.error.is_none(), "{:?}", r.error);
    for mode in SearchMode::ALL {
        let hits = &r.hits[&mode];
        assert!(hits.len() <= 2, "{mode}: {hits:?}");
        let mut dedup = hits.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(dedup.len(), hits.len(), "{mode}: duplicates in {hits:?}");
        for h in hits {
            assert!(qs[0].haystack.iter().any(|s| &s.id == h), "{mode}: unknown session {h}");
        }
        assert!(r.query_ms.contains_key(&mode));
    }
}

#[test]
fn semantic_mode_without_an_embedder_is_a_per_question_error() {
    let qs = load(&fixture()).unwrap();
    let cfg = RunConfig {
        modes: vec![SearchMode::Semantic],
        ks: vec![1],
    };
    let r = run_question(&qs[0], &cfg, None);
    let err = r.error.expect("error recorded");
    assert!(err.contains("embedder"), "{err}");
    assert!(r.hits.is_empty());
}

#[test]
fn a_failing_question_does_not_abort_the_batch() {
    let qs = load(&fixture()).unwrap();
    let cfg = RunConfig {
        modes: vec![SearchMode::Lexical],
        ks: vec![1],
    };
    // Force a failure by making the temp root unwritable: point TMPDIR at a file.
    let dir = tempfile::tempdir().unwrap();
    let not_a_dir = dir.path().join("file");
    std::fs::write(&not_a_dir, b"x").unwrap();
    let results: Vec<_> = qs
        .iter()
        .map(|q| run_question_in(q, &cfg, None, &not_a_dir))
        .collect();
    assert_eq!(results.len(), 6);
    assert!(results.iter().all(|r| r.error.is_some()));
    let s = summarise(&results, &cfg.ks);
    assert_eq!(s.errors, 6);
}

use singularmem_bench::runner::run_question_in;
```

Run: `cargo test -p singularmem-bench --test end_to_end` — Expected: compile error.

- [ ] **Step 2: Implement `runner.rs`**

```rust
//! Evaluate one LongMemEval question in an isolated temporary store.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use singularmem_core::hook::MultiHook;
use singularmem_core::{IndexHook, NewItem, Store};
use singularmem_ingest::{chunk_text, DEFAULT_CHUNK_BYTES};
use singularmem_retrieve::{RetrieveOptions, Retriever};
use singularmem_search::{Embedder, EmbedderIndex, HybridSearchOptions, HybridSearcher, Index};

use crate::dataset::{Question, Session};
use crate::metrics::{QuestionResult, SearchMode};

/// Retrieval over-fetch factor: several blocks can belong to one session.
const OVERFETCH: usize = 4;
const SESSION_TAG_PREFIX: &str = "s:";

#[derive(Debug, Clone)]
pub struct RunConfig {
    pub modes: Vec<SearchMode>,
    /// Sorted, deduplicated, non-empty.
    pub ks: Vec<usize>,
}

impl RunConfig {
    fn max_k(&self) -> usize {
        self.ks.iter().copied().max().unwrap_or(1)
    }
}

/// An [`Embedder`] that can be handed to several `EmbedderIndex::open`
/// calls without loading the model more than once.
#[derive(Clone)]
pub struct SharedEmbedder(Arc<dyn Embedder>);

impl SharedEmbedder {
    #[must_use]
    pub fn new(inner: Arc<dyn Embedder>) -> Self {
        Self(inner)
    }

    fn boxed(&self) -> Box<dyn Embedder> {
        Box::new(self.clone())
    }
}

impl Embedder for SharedEmbedder {
    fn dim(&self) -> usize {
        self.0.dim()
    }
    fn model_id(&self) -> &str {
        self.0.model_id()
    }
    fn embed(&self, content: &str) -> singularmem_search::Result<Vec<f32>> {
        self.0.embed(content)
    }
    fn embed_batch(&self, items: &[&str]) -> singularmem_search::Result<Vec<Vec<f32>>> {
        self.0.embed_batch(items)
    }
}

/// Evaluate `q` with the system temp dir as the scratch root.
#[must_use]
pub fn run_question(q: &Question, cfg: &RunConfig, embedder: Option<&SharedEmbedder>) -> QuestionResult {
    run_question_in(q, cfg, embedder, &std::env::temp_dir())
}

/// Evaluate `q`, creating its temporary store under `scratch_root`.
/// Never panics on I/O or index errors: they are recorded in `error`.
#[must_use]
pub fn run_question_in(
    q: &Question,
    cfg: &RunConfig,
    embedder: Option<&SharedEmbedder>,
    scratch_root: &Path,
) -> QuestionResult {
    let mut result = QuestionResult {
        id: q.id.clone(),
        kind: q.kind.clone(),
        abstention: q.abstention,
        evidence: q.evidence.iter().cloned().collect(),
        hits: BTreeMap::new(),
        ingest_ms: 0,
        query_ms: BTreeMap::new(),
        error: None,
    };
    result.evidence.sort();
    if let Err(e) = evaluate(q, cfg, embedder, scratch_root, &mut result) {
        result.hits.clear();
        result.query_ms.clear();
        result.error = Some(e);
    }
    result
}

fn evaluate(
    q: &Question,
    cfg: &RunConfig,
    embedder: Option<&SharedEmbedder>,
    scratch_root: &Path,
    out: &mut QuestionResult,
) -> Result<(), String> {
    let needs_embedder = cfg.modes.iter().any(|m| m.needs_embedder());
    if needs_embedder && embedder.is_none() {
        return Err("semantic/hybrid mode requested but no embedder was provided".into());
    }

    let dir = tempfile::Builder::new()
        .prefix("singularmem-bench-")
        .tempdir_in(scratch_root)
        .map_err(|e| format!("creating temp dir under {}: {e}", scratch_root.display()))?;
    let store_path = dir.path().join("store.db");
    let lex_path = dir.path().join("lex");
    let sem_path = dir.path().join("sem");

    // --- ingest ---------------------------------------------------------
    let t0 = Instant::now();
    {
        let mut hooks: Vec<Box<dyn IndexHook>> = vec![Box::new(
            Index::open(&lex_path).map_err(|e| format!("opening lexical index: {e}"))?,
        )];
        if let Some(emb) = embedder.filter(|_| needs_embedder) {
            hooks.push(Box::new(
                EmbedderIndex::open(&sem_path, emb.boxed())
                    .map_err(|e| format!("opening vector index: {e}"))?,
            ));
        }
        let store = Store::open_with_hook(&store_path, Box::new(MultiHook::new(hooks)))
            .map_err(|e| format!("opening store: {e}"))?;
        for (session_index, session) in q.haystack.iter().enumerate() {
            for item in items_for(q, session_index, session) {
                store
                    .ingest(item)
                    .map_err(|e| format!("ingesting session {}: {e}", session.id))?;
            }
        }
        // `store` (and the hooks) drop here, committing the indexes.
    }
    out.ingest_ms = t0.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

    // --- query ----------------------------------------------------------
    let store = Store::open(&store_path).map_err(|e| format!("reopening store: {e}"))?;
    let lex = Index::open(&lex_path).map_err(|e| format!("reopening lexical index: {e}"))?;
    let sem = match embedder.filter(|_| needs_embedder) {
        Some(emb) => Some(
            EmbedderIndex::open(&sem_path, emb.boxed())
                .map_err(|e| format!("reopening vector index: {e}"))?,
        ),
        None => None,
    };

    let fetch = cfg.max_k() * OVERFETCH;
    for &mode in &cfg.modes {
        let searcher = match (mode, sem.as_ref()) {
            (SearchMode::Lexical, _) => HybridSearcher::lexical_only(&lex),
            (SearchMode::Semantic, Some(s)) => HybridSearcher::semantic_only(s),
            (SearchMode::Hybrid, Some(s)) => HybridSearcher::new(&lex, s),
            (_, None) => return Err(format!("{mode} mode requires an embedder")),
        };
        let retriever = Retriever::new(&store, &searcher);
        let opts = RetrieveOptions {
            max_blocks: fetch,
            min_score: 0.0,
            search: HybridSearchOptions {
                limit: fetch,
                include_snippets: false,
                ..HybridSearchOptions::default()
            },
            scope: None,
        };
        let t = Instant::now();
        let ctx = retriever
            .retrieve(&q.text, &opts)
            .map_err(|e| format!("{mode} retrieval: {e}"))?;
        let ms = t.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

        let mut hits: Vec<String> = Vec::new();
        for block in &ctx.blocks {
            let Some(idx) = session_index_from_tags(&block.tags) else {
                continue;
            };
            let Some(session) = q.haystack.get(idx) else {
                continue;
            };
            if !hits.iter().any(|h| h == &session.id) {
                hits.push(session.id.clone());
            }
            if hits.len() == cfg.max_k() {
                break;
            }
        }
        out.hits.insert(mode, hits);
        out.query_ms.insert(mode, ms);
    }
    Ok(())
}

fn items_for(q: &Question, session_index: usize, session: &Session) -> Vec<NewItem> {
    let mut items = Vec::new();
    for (turn, t) in session.turns.iter().enumerate() {
        let text = format!("{}: {}", t.role, t.content);
        let chunks = chunk_text(&text, DEFAULT_CHUNK_BYTES);
        let multi = chunks.len() > 1;
        for (chunk, content) in chunks.into_iter().enumerate() {
            let mut external_id = format!("longmemeval:{}:{session_index}:{turn}", q.id);
            if multi {
                external_id.push_str(&format!("#{chunk}"));
            }
            items.push(NewItem {
                content,
                supersedes: None,
                tags: vec![format!("{SESSION_TAG_PREFIX}{session_index}")],
                source: Some("longmemeval".to_string()),
                metadata: serde_json::json!({
                    "session_id": session.id,
                    "session_index": session_index,
                    "turn": turn,
                    "role": t.role,
                    "date": session.date,
                }),
                external_id: Some(external_id),
                scope: Some("longmemeval".to_string()),
            });
        }
    }
    items
}

fn session_index_from_tags(tags: &[String]) -> Option<usize> {
    tags.iter()
        .find_map(|t| t.strip_prefix(SESSION_TAG_PREFIX))
        .and_then(|n| n.parse().ok())
}
```

Add `pub mod runner;` to `lib.rs`. If `chunk_text` returns a single chunk for the whole text when it fits, `multi` is false and no `#chunk` suffix is written — that is the intended behaviour.

If `NewItem` has more fields than listed in this plan (check `crates/singularmem-core/src/item.rs`), set them to their neutral defaults and note it in the report.

- [ ] **Step 3: Run tests**

Run: `cargo test -p singularmem-bench --test end_to_end` — Expected: 4 passed. If `lexical_hits_on_the_fixture_are_exact` fails on `q5`, the fixture's `s5a` shares a query term with the question by accident: fix the FIXTURE wording (keep it a paraphrase), not the assertion. If it fails on `q1..q4` rank 1, check that the distractor sessions share no query word beyond stopwords and adjust the fixture the same way. Record any fixture change in the task report.

Run: `cargo clippy -p singularmem-bench --all-targets --all-features -- -D warnings && cargo fmt --all -- --check`.

- [ ] **Step 4: Commit**

```bash
git add crates/singularmem-bench/src/lib.rs crates/singularmem-bench/src/runner.rs crates/singularmem-bench/tests/end_to_end.rs crates/singularmem-bench/tests/fixtures
git commit -s -m "feat(bench): per-question runner over the real retrieval pipeline"
```

---

### Task 4: Report and CLI

**Files:**
- Create: `crates/singularmem-bench/src/report.rs`
- Replace: `crates/singularmem-bench/src/main.rs`
- Modify: `crates/singularmem-bench/src/lib.rs` (add `pub mod report;`)
- Test: `crates/singularmem-bench/tests/cli.rs`

**Interfaces:**
- Consumes: everything above; `singularmem_search::{FastembedEmbedder, EmbeddingModel}`.
- Produces: `report::{RunMeta, Report, render_markdown, to_json}`; the `singularmem-bench longmemeval` command.

- [ ] **Step 1: Failing CLI tests**

`crates/singularmem-bench/tests/cli.rs`:

```rust
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/longmemeval-mini.json")
}

fn bench() -> Command {
    let mut c = Command::cargo_bin("singularmem-bench").expect("binary exists");
    c.env("SINGULARMEM_TEST_EMBEDDER", "mock");
    c
}

#[test]
fn lexical_run_prints_markdown_report_and_json() {
    let dir = tempfile::tempdir().unwrap();
    let json = dir.path().join("out.json");
    let assert = bench()
        .args(["longmemeval"])
        .arg(fixture())
        .args(["--modes", "lexical", "--k", "1,5", "--quiet", "--json"])
        .arg(&json)
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(out.starts_with("# LongMemEval retrieval — longmemeval-mini.json"), "{out}");
    assert!(out.contains("questions 6 (scored 5, abstention 1, errors 0)"), "{out}");
    assert!(out.contains("| mode"), "{out}");
    assert!(out.contains("| lexical | 0.800 | 0.800 | 0.800 |"), "{out}");
    assert!(out.contains("## R@5 by question type"), "{out}");
    assert!(out.contains("| single-session-user"), "{out}");
    assert!(!out.contains("semantic"), "only the requested mode is reported: {out}");

    let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(doc["tool"], "singularmem-bench");
    assert_eq!(doc["dataset"]["questions"], 6);
    assert_eq!(doc["config"]["modes"], serde_json::json!(["lexical"]));
    assert_eq!(doc["config"]["ks"], serde_json::json!([1, 5]));
    assert_eq!(doc["questions"].as_array().unwrap().len(), 6);
    assert_eq!(doc["summary"]["abstentions"], 1);
    assert!(doc["dataset"]["sha256"].as_str().unwrap().len() == 64);
    assert!(doc["commit"].is_string());
}

#[test]
fn all_modes_run_with_the_mock_embedder() {
    let out = bench()
        .arg("longmemeval")
        .arg(fixture())
        .args(["--quiet", "--limit", "2"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("| lexical "), "{out}");
    assert!(out.contains("| semantic "), "{out}");
    assert!(out.contains("| hybrid "), "{out}");
    assert!(out.contains("questions 2 "), "{out}");
}

#[test]
fn question_type_filter_and_seeded_limit() {
    let out = bench()
        .arg("longmemeval")
        .arg(fixture())
        .args(["--modes", "lexical", "--quiet", "--question-type", "multi-session"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("questions 1 (scored 1"), "{out}");

    let a = bench()
        .arg("longmemeval").arg(fixture())
        .args(["--modes", "lexical", "--quiet", "--limit", "3", "--seed", "7", "--json"])
        .arg("/dev/stdout")
        .assert().success().get_output().stdout.clone();
    let b = bench()
        .arg("longmemeval").arg(fixture())
        .args(["--modes", "lexical", "--quiet", "--limit", "3", "--seed", "7", "--json"])
        .arg("/dev/stdout")
        .assert().success().get_output().stdout.clone();
    // Same seed -> same selection (compare the ids in the JSON part).
    let ids = |s: &[u8]| -> Vec<String> {
        let text = String::from_utf8_lossy(s);
        let json_start = text.find("{\"tool\"").expect("json present");
        let doc: serde_json::Value = serde_json::from_str(&text[json_start..]).unwrap();
        doc["questions"].as_array().unwrap().iter().map(|q| q["id"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(ids(&a), ids(&b));
    assert_eq!(ids(&a).len(), 3);
}

#[test]
fn missing_file_exits_1_with_the_path() {
    bench()
        .args(["longmemeval", "/nonexistent/lme.json", "--modes", "lexical"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("/nonexistent/lme.json"));
}

#[test]
fn bad_k_and_bad_mode_are_usage_errors() {
    bench()
        .arg("longmemeval").arg(fixture())
        .args(["--k", "0"])
        .assert()
        .code(2);
    bench()
        .arg("longmemeval").arg(fixture())
        .args(["--modes", "fuzzy"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("fuzzy"));
    bench()
        .arg("longmemeval").arg(fixture())
        .args(["--model", "gpt-embed"])
        .assert()
        .code(2);
}

#[test]
fn filter_that_leaves_nothing_exits_1() {
    bench()
        .arg("longmemeval").arg(fixture())
        .args(["--modes", "lexical", "--question-type", "brand-new-type"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("no questions"));
}
```

The `--json /dev/stdout` trick is Unix-only; the CI matrix runs Linux and macOS. Put `#[cfg(unix)]` on `question_type_filter_and_seeded_limit`.

Run: `cargo test -p singularmem-bench --test cli` — Expected: failures (stub binary exits 2).

- [ ] **Step 2: Implement `report.rs`**

```rust
//! Markdown and JSON rendering of a benchmark run.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::dataset::QuestionType;
use crate::metrics::{QuestionResult, SearchMode, Summary};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMeta {
    pub tool: String,
    pub version: String,
    pub commit: String,
    pub dataset_path: String,
    pub dataset_sha256: String,
    pub dataset_questions: usize,
    pub modes: Vec<SearchMode>,
    pub ks: Vec<usize>,
    pub model: Option<String>,
    pub limit: Option<usize>,
    pub question_type: Vec<QuestionType>,
    pub seed: u64,
    pub started_at: String,
    pub finished_at: String,
    pub ingest_items_per_s: f64,
    pub wall_secs: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report<'a> {
    pub meta: &'a RunMeta,
    pub summary: &'a Summary,
    pub questions: &'a [QuestionResult],
}

/// The JSON document written by `--json`. Shape per spec § `report.rs`.
///
/// # Errors
/// Only if serialisation fails, which it cannot for these types.
pub fn to_json(r: &Report<'_>) -> serde_json::Result<String> {
    let doc = serde_json::json!({
        "tool": r.meta.tool,
        "version": r.meta.version,
        "commit": r.meta.commit,
        "dataset": {
            "path": r.meta.dataset_path,
            "sha256": r.meta.dataset_sha256,
            "questions": r.meta.dataset_questions,
        },
        "config": {
            "modes": r.meta.modes,
            "ks": r.meta.ks,
            "model": r.meta.model,
            "limit": r.meta.limit,
            "question_type": r.meta.question_type,
            "seed": r.meta.seed,
        },
        "summary": r.summary,
        "questions": r.questions,
        "started_at": r.meta.started_at,
        "finished_at": r.meta.finished_at,
        "ingest_items_per_s": r.meta.ingest_items_per_s,
        "wall_secs": r.meta.wall_secs,
    });
    serde_json::to_string_pretty(&doc)
}

fn fmt3(x: f64) -> String {
    format!("{x:.3}")
}

/// The Markdown report printed to stdout.
#[must_use]
pub fn render_markdown(r: &Report<'_>) -> String {
    let m = r.meta;
    let s = r.summary;
    let mut out = String::new();
    let basename = std::path::Path::new(&m.dataset_path)
        .file_name()
        .map_or_else(|| m.dataset_path.clone(), |f| f.to_string_lossy().into_owned());
    let scored = r
        .questions
        .iter()
        .filter(|q| !q.abstention && q.error.is_none())
        .count();
    let _ = writeln!(out, "# LongMemEval retrieval — {basename}\n");
    let _ = writeln!(
        out,
        "commit {}  model {}  dataset sha256 {}  questions {} (scored {}, abstention {}, errors {})",
        m.commit,
        m.model.as_deref().unwrap_or("none"),
        &m.dataset_sha256[..12.min(m.dataset_sha256.len())],
        r.questions.len(),
        scored,
        s.abstentions,
        s.errors
    );
    let _ = writeln!(
        out,
        "ingest {:.1} items/s  wall {:02}:{:02}\n",
        m.ingest_items_per_s,
        (m.wall_secs / 60.0).floor() as u64,
        (m.wall_secs % 60.0).floor() as u64
    );

    // Overall table.
    let mut header = String::from("| mode |");
    let mut rule = String::from("|---|");
    for k in &m.ks {
        let _ = write!(header, " R@{k} |");
        rule.push_str("---|");
    }
    header.push_str(" MRR | q/s |");
    rule.push_str("---|---|");
    let _ = writeln!(out, "{header}\n{rule}");
    for mode in &m.modes {
        let Some(mm) = s.overall.get(mode) else {
            let _ = writeln!(out, "| {mode} | n/a |");
            continue;
        };
        let mut row = format!("| {mode} |");
        for k in &m.ks {
            let _ = write!(row, " {} |", fmt3(mm.recall.get(k).copied().unwrap_or(0.0)));
        }
        let _ = write!(row, " {} | {:.1} |", fmt3(mm.mrr), mm.queries_per_s);
        let _ = writeln!(out, "{row}");
    }

    // By-type table at the middle k.
    let mid_k = m.ks[m.ks.len() / 2];
    let _ = writeln!(out, "\n## R@{mid_k} by question type\n");
    let mut header = String::from("| type | n |");
    let mut rule = String::from("|---|---|");
    for mode in &m.modes {
        let _ = write!(header, " {mode} |");
        rule.push_str("---|");
    }
    let _ = writeln!(out, "{header}\n{rule}");
    for (kind, modes) in &s.by_type {
        let n = modes.values().next().map_or(0, |mm| mm.n);
        let mut row = format!("| {kind} | {n} |");
        for mode in &m.modes {
            let cell = modes
                .get(mode)
                .and_then(|mm| mm.recall.get(&mid_k))
                .map_or_else(|| "n/a".to_string(), |v| fmt3(*v));
            let _ = write!(row, " {cell} |");
        }
        let _ = writeln!(out, "{row}");
    }
    out
}
```

Add `#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]` on `render_markdown` with a reason comment (minute/second arithmetic on small positive floats).

- [ ] **Step 3: Implement `main.rs`**

```rust
//! `singularmem-bench` — retrieval-quality benchmark CLI.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand};
use rand::seq::SliceRandom;
use rand::SeedableRng;
use sha2::{Digest, Sha256};
use singularmem_bench::dataset::{self, Question, QuestionType};
use singularmem_bench::metrics::{summarise, QuestionResult, SearchMode};
use singularmem_bench::report::{render_markdown, to_json, Report, RunMeta};
use singularmem_bench::runner::{run_question, RunConfig, SharedEmbedder};
use singularmem_search::{EmbeddingModel, FastembedEmbedder};

#[derive(Parser)]
#[command(name = "singularmem-bench", version, about = "Retrieval-quality benchmarks for Singularmem")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Evaluate retrieval against a LongMemEval JSON file.
    Longmemeval(LongMemEvalArgs),
}

#[derive(clap::Args)]
struct LongMemEvalArgs {
    /// Path to longmemeval_s.json / longmemeval_m.json / longmemeval_oracle.json
    file: PathBuf,
    /// Search modes to evaluate (comma-separated): lexical, semantic, hybrid
    #[arg(long, value_delimiter = ',', default_values_t = SearchMode::ALL)]
    modes: Vec<SearchMode>,
    /// Recall cut-offs (comma-separated), each >= 1
    #[arg(long = "k", value_delimiter = ',', default_values_t = [1usize, 5, 10], value_parser = clap::value_parser!(u64).range(1..))]
    ks: Vec<u64>,
    /// Embedding model: all-mini-lm-l6-v2, bge-small-en, nomic-embed
    #[arg(long, default_value = "all-mini-lm-l6-v2", value_parser = ["all-mini-lm-l6-v2", "bge-small-en", "nomic-embed"])]
    model: String,
    /// Evaluate only N questions (see --seed)
    #[arg(long)]
    limit: Option<usize>,
    /// Only questions of this type (repeatable)
    #[arg(long = "question-type")]
    question_type: Vec<String>,
    /// Shuffle seed used with --limit; 0 keeps file order
    #[arg(long, default_value_t = 0)]
    seed: u64,
    /// Write the JSON document here
    #[arg(long)]
    json: Option<PathBuf>,
    /// Suppress per-question progress on stderr
    #[arg(long)]
    quiet: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Longmemeval(args) => match run_longmemeval(args) {
            Ok(clean) => {
                if clean {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                }
            }
            Err(msg) => {
                eprintln!("singularmem-bench: {msg}");
                ExitCode::from(1)
            }
        },
    }
}

fn parse_model(name: &str) -> EmbeddingModel {
    match name {
        "bge-small-en" => EmbeddingModel::BgeSmallEnV15,
        "nomic-embed" => EmbeddingModel::NomicEmbedTextV15,
        _ => EmbeddingModel::AllMiniLmL6V2,
    }
}

fn build_embedder(model: EmbeddingModel) -> Result<SharedEmbedder, String> {
    if std::env::var("SINGULARMEM_TEST_EMBEDDER").as_deref() == Ok("mock") {
        return Ok(SharedEmbedder::new(Arc::new(
            singularmem_search::testing::MockEmbedder::default(),
        )));
    }
    FastembedEmbedder::with_model(model)
        .map(|e| SharedEmbedder::new(Arc::new(e)))
        .map_err(|e| format!("loading embedding model failed: {e}; use --modes lexical to skip embeddings"))
}

fn sha256_of(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn git_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(|| "unknown".to_string(), |o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn select(mut qs: Vec<Question>, args: &LongMemEvalArgs) -> Vec<Question> {
    if !args.question_type.is_empty() {
        let wanted: Vec<QuestionType> = args.question_type.iter().map(|s| QuestionType::from(s.as_str())).collect();
        qs.retain(|q| wanted.contains(&q.kind));
    }
    if let Some(n) = args.limit {
        if args.seed != 0 {
            let mut rng = rand::rngs::StdRng::seed_from_u64(args.seed);
            qs.shuffle(&mut rng);
        }
        qs.truncate(n);
    }
    qs
}

/// Returns `Ok(true)` for a clean run, `Ok(false)` when any question errored.
fn run_longmemeval(args: LongMemEvalArgs) -> Result<bool, String> {
    let started = Instant::now();
    let started_at = jiff::Timestamp::now().to_string();

    let all = dataset::load(&args.file).map_err(|e| e.to_string())?;
    let total = all.len();
    let qs = select(all, &args);
    if qs.is_empty() {
        return Err("no questions left after --limit/--question-type filtering".into());
    }

    let mut ks: Vec<usize> = args.ks.iter().map(|&k| usize::try_from(k).unwrap_or(usize::MAX)).collect();
    ks.sort_unstable();
    ks.dedup();
    let cfg = RunConfig { modes: args.modes.clone(), ks: ks.clone() };

    let needs_embedder = cfg.modes.iter().any(|m| m.needs_embedder());
    let embedder = if needs_embedder { Some(build_embedder(parse_model(&args.model))?) } else { None };
    let model_id = embedder.as_ref().map(|e| singularmem_search::Embedder::model_id(e).to_string());

    let mut results: Vec<QuestionResult> = Vec::with_capacity(qs.len());
    let mut items_ingested = 0usize;
    let mut ingest_ms_total = 0u64;
    let n = qs.len();
    for (i, q) in qs.iter().enumerate() {
        let r = run_question(q, &cfg, embedder.as_ref());
        items_ingested += q.haystack.iter().map(|s| s.turns.len()).sum::<usize>();
        ingest_ms_total += r.ingest_ms;
        if !args.quiet {
            let mut line = format!("[{}/{n}] {} {}", i + 1, r.id, r.kind);
            match &r.error {
                Some(e) => line.push_str(&format!("  ERROR {e}")),
                None => {
                    for (mode, hits) in &r.hits {
                        let rank = hits.iter().position(|h| r.evidence.contains(h)).map_or_else(|| "miss".to_string(), |p| format!("hit@{}", p + 1));
                        line.push_str(&format!("  {mode} {rank}"));
                    }
                }
            }
            let secs = started.elapsed().as_secs_f64();
            line.push_str(&format!("  {secs:.1}s"));
            eprintln!("{line}");
            if i + 1 == 10 && n > 10 {
                let est = secs / 10.0 * n as f64;
                eprintln!("estimated total: {:.0} min", est / 60.0);
            }
        }
        results.push(r);
    }

    let summary = summarise(&results, &ks);
    let wall_secs = started.elapsed().as_secs_f64();
    let meta = RunMeta {
        tool: "singularmem-bench".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        commit: git_commit(),
        dataset_path: args.file.display().to_string(),
        dataset_sha256: sha256_of(&args.file)?,
        dataset_questions: total,
        modes: cfg.modes.clone(),
        ks,
        model: model_id,
        limit: args.limit,
        question_type: args.question_type.iter().map(|s| QuestionType::from(s.as_str())).collect(),
        seed: args.seed,
        started_at,
        finished_at: jiff::Timestamp::now().to_string(),
        ingest_items_per_s: if ingest_ms_total == 0 { 0.0 } else { items_ingested as f64 / (ingest_ms_total as f64 / 1000.0) },
        wall_secs,
    };
    let report = Report { meta: &meta, summary: &summary, questions: &results };

    print!("{}", render_markdown(&report));
    if let Some(path) = &args.json {
        let doc = to_json(&report).map_err(|e| format!("serialising report: {e}"))?;
        std::fs::write(path, doc).map_err(|e| format!("writing {}: {e}", path.display()))?;
    }
    Ok(summary.errors == 0)
}
```

Notes: `dataset_questions` is the count in the FILE (before filtering); the Markdown header's `questions N` uses `r.questions.len()` (after filtering) — the CLI test asserts `questions 2` under `--limit 2` and `dataset.questions == 6` in JSON. `SearchMode` needs `clap::ValueEnum`-free parsing: `default_values_t` with `FromStr` + `Display` works because `SearchMode: FromStr<Err = String> + Display + Clone`. Add `#[allow(clippy::cast_precision_loss)]` with reason where `as f64` appears. `main.rs` may exceed clippy's `too_many_lines` for `run_longmemeval`; split the progress line into `fn progress_line(i, n, r, secs) -> String` rather than allowing the lint.

- [ ] **Step 4: Run tests, lint, commit**

Run: `cargo test -p singularmem-bench` — Expected: all tests in all four test files pass.
Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo fmt --all -- --check`.

```bash
git add crates/singularmem-bench/src crates/singularmem-bench/tests/cli.rs Cargo.lock
git commit -s -m "feat(bench): longmemeval command with Markdown and JSON reports"
```

---

### Task 5: Full run and documentation

**Files:**
- Create: `crates/singularmem-bench/README.md`
- Create: `docs/benchmarks/longmemeval.md`
- Modify: `README.md` (status line; add a "Benchmarks" pointer sentence under Contributing or a new short section)
- Modify: `docs/superpowers/specs/2026-09-05-retrieval-benchmark-15-design.md` (Deviations section, if any)

This task needs the network once, by hand, to download the dataset and the embedding model. Nothing in the repo's tests depends on it.

- [ ] **Step 1: Download the dataset**

Hugging Face dataset `xiaowu0162/longmemeval`. Check the file listing at `https://huggingface.co/datasets/xiaowu0162/longmemeval/tree/main` and download `longmemeval_s.json` (the `_S` split, ~200 MB) into `~/Downloads/` or a scratch dir outside the repo:

```bash
curl -L -o ~/Downloads/longmemeval_s.json \
  https://huggingface.co/datasets/xiaowu0162/longmemeval/resolve/main/longmemeval_s.json
```

If the file name differs in the listing, use the listed name and record it in `docs/benchmarks/longmemeval.md`.

- [ ] **Step 2: Sanity run**

```bash
cargo run --release -p singularmem-bench -- longmemeval ~/Downloads/longmemeval_s.json --modes lexical --limit 20
```

Expected: a Markdown report; no `ERROR` lines on stderr; exit 0. Then a 20-question run with all modes to confirm the model loads:

```bash
cargo run --release -p singularmem-bench -- longmemeval ~/Downloads/longmemeval_s.json --limit 20
```

- [ ] **Step 3: Full run**

```bash
cargo run --release -p singularmem-bench -- longmemeval ~/Downloads/longmemeval_s.json \
  --json docs/benchmarks/longmemeval-s-$(git rev-parse --short HEAD).json \
  2> /tmp/longmemeval-progress.log | tee /tmp/longmemeval-report.md
```

Expected: exit 0 after roughly an hour; `errors 0` in the header. Do NOT commit the JSON file if it exceeds 1 MB (it will: 500 questions × 3 hit lists); keep it out of git and note its location in the doc instead. Commit only the Markdown table.

- [ ] **Step 4: Write `docs/benchmarks/longmemeval.md`**

Structure (fill every value from the actual run; no placeholders may remain):

```markdown
# LongMemEval retrieval benchmark

What it measures, in one paragraph: Recall@k = fraction of questions whose
evidence session appears among the top-k distinct sessions retrieved;
MRR; per search mode; abstention questions excluded. Retrieval only —
not the LLM-judged answer accuracy.

## Getting the dataset
(the curl command above; file size; sha256 of the file you used)

## Running
(the exact command; expected wall time; `--limit`/`--modes lexical` for quick runs)

## Results — LongMemEval_S, <date>
commit <sha>, model sentence-transformers/all-MiniLM-L6-v2, <CPU model>, <wall time>

(paste the two tables from the report verbatim)

## Comparing with mempalace
mempalace reports LongMemEval R@5 on the same split with a session-level
hit definition. State their published number with a link to the README
line it comes from, and the caveats: different chunking (per turn here),
different embedding model unless matched, different k semantics if any.

## Reading the numbers
Two or three sentences on what lexical vs hybrid tells us and what to
try next (e.g. rrf_k, fetch_multiplier, bge-small-en).
```

- [ ] **Step 5: `crates/singularmem-bench/README.md`**

Usage, every flag with one line, the exit-code table from the spec, and a pointer to `docs/benchmarks/longmemeval.md`.

- [ ] **Step 6: Root README**

In the status blockquote add ", and a LongMemEval retrieval benchmark (`singularmem-bench`)". Add after "Editor integration" a three-line "## Benchmarks" section pointing to `docs/benchmarks/longmemeval.md`.

- [ ] **Step 7: Verify and commit**

Run: `cargo test --workspace --all-targets && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo fmt --all -- --check`.

```bash
git add docs/benchmarks/longmemeval.md crates/singularmem-bench/README.md README.md docs/superpowers/specs/2026-09-05-retrieval-benchmark-15-design.md
git commit -s -m "docs(bench): LongMemEval results and usage"
```

---

## Self-review

- Spec coverage: dataset (T1), runner incl. isolation, tags, over-fetch, per-question error capture (T3), metrics incl. abstention/error exclusion and multi-evidence rule (T2), Markdown/JSON report shapes and exit codes (T4), docs and full run (T5). `--seed`, `--limit`, `--question-type`, `--quiet`, `SINGULARMEM_TEST_EMBEDDER` (T4). Acceptance criteria 1–6 map to T1–T5.
- Types: `QuestionResult`, `SearchMode` defined in `metrics` and used identically in `runner`, `report`, `main`; `RunConfig { modes, ks }` consistent; `SharedEmbedder::new(Arc<dyn Embedder>)` consistent between T3 and T4.
- Deviation from spec text: `RunConfig` has no `model` field (the embedder is passed separately) and `run_question` takes `Option<&SharedEmbedder>`; the tag is `s:{session_index}` (not the session id) because `MemoryBlock` exposes tags but not metadata. Record both in the spec's Deviations section in Task 5.
