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
    // Compact, not pretty: `--json /dev/stdout` concatenates this after the
    // Markdown report, and callers (including the CLI tests) locate the
    // JSON by searching for the literal `{"tool"` prefix.
    serde_json::to_string(&doc)
}

fn fmt3(x: f64) -> String {
    format!("{x:.3}")
}

/// The Markdown report printed to stdout.
// `wall_secs` is always a small non-negative elapsed-time measurement, so
// truncating to minutes/seconds via `as u64` cannot lose meaningful
// precision or wrap around.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[must_use]
pub fn render_markdown(r: &Report<'_>) -> String {
    let m = r.meta;
    let s = r.summary;
    let mut out = String::new();
    let basename = std::path::Path::new(&m.dataset_path)
        .file_name()
        .map_or_else(
            || m.dataset_path.clone(),
            |f| f.to_string_lossy().into_owned(),
        );
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
    header.push_str(" MRR |");
    rule.push_str("---|");
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
        let _ = write!(row, " {} |", fmt3(mm.mrr));
        let _ = writeln!(out, "{row}");
    }

    // By-type table at the middle k. `m.ks` is non-empty by construction
    // (`RunConfig::new` rejects an empty `ks`), but guard anyway so a
    // `Report` built by hand (as in tests) can't panic on the index below.
    if let Some(&mid_k) = m.ks.get(m.ks.len() / 2) {
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
    }
    out
}
