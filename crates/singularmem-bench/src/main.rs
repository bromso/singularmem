//! `singularmem-bench` — retrieval-quality benchmark CLI.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand};
use rand::seq::SliceRandom;
use rand::SeedableRng;
use singularmem_bench::dataset::{self, Question, QuestionType};
use singularmem_bench::metrics::{summarise, QuestionResult, SearchMode};
use singularmem_bench::report::{render_markdown, to_json, Report, RunMeta};
use singularmem_bench::runner::{run_question, RunConfig, SharedEmbedder};
use singularmem_search::{EmbeddingModel, FastembedEmbedder};

#[derive(Parser)]
#[command(
    name = "singularmem-bench",
    version,
    about = "Retrieval-quality benchmarks for Singularmem"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Evaluate retrieval against a `LongMemEval` JSON file.
    Longmemeval(LongMemEvalArgs),
}

#[derive(clap::Args)]
struct LongMemEvalArgs {
    /// Path to `longmemeval_s.json` / `longmemeval_m.json` / `longmemeval_oracle.json`
    file: PathBuf,
    /// Search modes to evaluate (comma-separated): lexical, semantic, hybrid
    #[arg(long, value_delimiter = ',', default_values_t = SearchMode::ALL)]
    modes: Vec<SearchMode>,
    /// Recall cut-offs (comma-separated), each >= 1
    #[arg(
        long = "k",
        value_delimiter = ',',
        default_values_t = [1u64, 5, 10],
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    ks: Vec<u64>,
    /// Embedding model: all-mini-lm-l6-v2, bge-small-en, nomic-embed
    #[arg(
        long,
        default_value = "all-mini-lm-l6-v2",
        value_parser = ["all-mini-lm-l6-v2", "bge-small-en", "nomic-embed"]
    )]
    model: String,
    /// Evaluate only N questions (see --seed)
    #[arg(long)]
    limit: Option<usize>,
    /// Only questions of this type (repeatable)
    #[arg(
        long = "question-type",
        value_parser = [
            "single-session-user",
            "single-session-assistant",
            "single-session-preference",
            "multi-session",
            "temporal-reasoning",
            "knowledge-update",
        ]
    )]
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
        Command::Longmemeval(args) => match run_longmemeval(&args) {
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
        .map_err(|e| {
            format!("loading embedding model failed: {e}; use --modes lexical to skip embeddings")
        })
}

fn git_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(
            || "unknown".to_string(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_string(),
        )
}

fn select(mut qs: Vec<Question>, args: &LongMemEvalArgs) -> Vec<Question> {
    if !args.question_type.is_empty() {
        let wanted: Vec<QuestionType> = args
            .question_type
            .iter()
            .map(|s| QuestionType::from(s.as_str()))
            .collect();
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

/// One line of per-question progress, printed to stderr unless `--quiet`.
fn progress_line(i: usize, n: usize, r: &QuestionResult, secs: f64) -> String {
    let mut line = format!("[{}/{n}] {} {}", i + 1, r.id, r.kind);
    match &r.error {
        Some(e) => {
            let _ = write!(line, "  ERROR {e}");
        }
        None => {
            for (mode, hits) in &r.hits {
                let rank = hits
                    .iter()
                    .position(|h| r.evidence.contains(h))
                    .map_or_else(|| "miss".to_string(), |p| format!("hit@{}", p + 1));
                let _ = write!(line, "  {mode} {rank}");
            }
        }
    }
    let _ = write!(line, "  {secs:.1}s");
    line
}

/// Returns `Ok(true)` for a clean run, `Ok(false)` when any question errored.
fn run_longmemeval(args: &LongMemEvalArgs) -> Result<bool, String> {
    let started = Instant::now();
    let started_at = jiff::Timestamp::now().to_string();

    // Hash the bytes that are actually parsed, in one read, so the reported
    // sha256 can never drift from what was loaded.
    let (all, dataset_sha256) = dataset::load_with_digest(&args.file).map_err(|e| e.to_string())?;
    let total = all.len();
    let qs = select(all, args);
    if qs.is_empty() {
        return Err("no questions left after --limit/--question-type filtering".into());
    }

    let ks: Vec<usize> = args
        .ks
        .iter()
        .map(|&k| usize::try_from(k).unwrap_or(usize::MAX))
        .collect();
    let cfg = RunConfig::new(args.modes.clone(), ks)?;

    let needs_embedder = cfg.modes.iter().any(|m| m.needs_embedder());
    let embedder = if needs_embedder {
        Some(build_embedder(parse_model(&args.model))?)
    } else {
        None
    };
    let model_id = embedder
        .as_ref()
        .map(|e| singularmem_search::Embedder::model_id(e).to_string());

    let mut results: Vec<QuestionResult> = Vec::with_capacity(qs.len());
    let mut items_ingested = 0usize;
    let mut ingest_ms_total = 0u64;
    let n = qs.len();
    for (i, q) in qs.iter().enumerate() {
        let r = run_question(q, &cfg, embedder.as_ref());
        // Errored questions may have ingested nothing (or ingested and then
        // failed at query time) — exclude them from both the numerator and
        // the denominator so a handful of failures can't skew the rate.
        if r.error.is_none() {
            items_ingested += r.items_ingested;
            ingest_ms_total += r.ingest_ms;
        }
        if !args.quiet {
            let secs = started.elapsed().as_secs_f64();
            eprintln!("{}", progress_line(i, n, &r, secs));
            if i + 1 == 10 && n > 10 {
                let est = secs / 10.0 * question_count_as_f64(n);
                eprintln!("estimated total: {:.0} min", est / 60.0);
            }
        }
        results.push(r);
    }

    let summary = summarise(&results, &cfg.ks);
    let wall_secs = started.elapsed().as_secs_f64();
    let meta = RunMeta {
        tool: "singularmem-bench".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        commit: git_commit(),
        dataset_path: args.file.display().to_string(),
        dataset_sha256,
        dataset_questions: total,
        modes: cfg.modes,
        ks: cfg.ks,
        model: model_id,
        limit: args.limit,
        question_type: args
            .question_type
            .iter()
            .map(|s| QuestionType::from(s.as_str()))
            .collect(),
        seed: args.seed,
        started_at,
        finished_at: jiff::Timestamp::now().to_string(),
        ingest_items_per_s: ingest_items_per_second(items_ingested, ingest_ms_total),
        wall_secs,
    };
    let report = Report {
        meta: &meta,
        summary: &summary,
        questions: &results,
    };

    print!("{}", render_markdown(&report));
    if let Some(path) = &args.json {
        let doc = to_json(&report).map_err(|e| format!("serialising report: {e}"))?;
        std::fs::write(path, doc).map_err(|e| format!("writing {}: {e}", path.display()))?;
    }
    Ok(summary.errors == 0)
}

// `items_ingested` and `ingest_ms_total` stay far below 2^53, so the f64
// conversions below cannot lose precision.
#[allow(clippy::cast_precision_loss)]
fn ingest_items_per_second(items_ingested: usize, ingest_ms_total: u64) -> f64 {
    if ingest_ms_total == 0 {
        0.0
    } else {
        items_ingested as f64 / (ingest_ms_total as f64 / 1000.0)
    }
}

// A dataset's question count stays far below 2^53, so this conversion
// cannot lose precision.
#[allow(clippy::cast_precision_loss)]
const fn question_count_as_f64(n: usize) -> f64 {
    n as f64
}
