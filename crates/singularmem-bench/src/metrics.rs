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
    /// Number of items (post-chunking) actually ingested for this
    /// question — the length of the `Vec<Item>` `Store::ingest_many`
    /// returned, not the raw turn count.
    pub items_ingested: usize,
    pub query_us: BTreeMap<SearchMode, u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeMetrics {
    /// Recall@k keyed by k.
    pub recall: BTreeMap<usize, f64>,
    pub mrr: f64,
    /// Scored questions.
    pub n: usize,
    /// Retrieval call throughput; excludes ingestion.
    pub retrieve_queries_per_s: f64,
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
    query_us: u64,
}

impl Acc {
    fn new(ks: &[usize]) -> Self {
        Self {
            hit_at: ks.iter().map(|&k| (k, 0)).collect(),
            rr_sum: 0.0,
            n: 0,
            query_us: 0,
        }
    }

    // Counts and elapsed microseconds stay far below 2^53, so the f64
    // conversions below cannot lose precision.
    #[allow(clippy::cast_precision_loss)]
    fn add(&mut self, rank: Option<usize>, query_us: u64) {
        self.n += 1;
        self.query_us += query_us;
        if let Some(r) = rank {
            self.rr_sum += 1.0 / r as f64;
            for (k, count) in &mut self.hit_at {
                if r <= *k {
                    *count += 1;
                }
            }
        }
    }

    // Counts and elapsed microseconds stay far below 2^53, so the f64
    // conversions below cannot lose precision.
    #[allow(clippy::cast_precision_loss)]
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
            retrieve_queries_per_s: if self.query_us == 0 {
                0.0
            } else {
                n / (self.query_us as f64 / 1_000_000.0)
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
            let us = r.query_us.get(mode).copied().unwrap_or(0);
            overall
                .entry(*mode)
                .or_insert_with(|| Acc::new(ks))
                .add(rank, us);
            by_type
                .entry(r.kind.clone())
                .or_default()
                .entry(*mode)
                .or_insert_with(|| Acc::new(ks))
                .add(rank, us);
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
