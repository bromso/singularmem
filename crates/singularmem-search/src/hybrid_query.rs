//! Hybrid (lexical + semantic) search via Reciprocal Rank Fusion.
//!
//! See `docs/superpowers/specs/2026-05-17-search-v0-hybrid-design.md`
//! for the design rationale.

use serde::Serialize;
use singularmem_core::ItemId;
use std::time::Duration;

/// Options controlling a hybrid search query.
#[derive(Debug, Clone)]
pub struct HybridSearchOptions {
    /// Maximum number of fused hits to return. Default: 20.
    pub limit: usize,
    /// Per-ranker overfetch factor. Each underlying ranker fetches
    /// `limit * fetch_multiplier` candidates before fusion. Default: 3.
    pub fetch_multiplier: usize,
    /// RRF damping constant `k`. Larger → flatter weighting between
    /// top-1 and top-N. Default: 60 (Cormack et al. 2009).
    pub rrf_k: usize,
    /// Include lexical snippet highlights (if available). Default: true.
    pub include_snippets: bool,
}

impl Default for HybridSearchOptions {
    fn default() -> Self {
        Self {
            limit: 20,
            fetch_multiplier: 3,
            rrf_k: 60,
            include_snippets: true,
        }
    }
}

/// Discriminator naming which kind of score `HybridHit::score` carries.
///
/// Lets `--json` consumers interpret the float correctly without inspecting
/// rank fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScoreKind {
    /// Fused Reciprocal Rank Fusion score (both rankers ran).
    Rrf,
    /// Tantivy BM25 score (lexical-only mode).
    Bm25,
    /// `USearch` cosine similarity (semantic-only mode).
    Cosine,
}

/// One hit in a `HybridSearchResults`.
#[derive(Debug, Clone, Serialize)]
pub struct HybridHit {
    /// The matched item's ID. Call `Store::get(hit.id)` for the full payload.
    pub id: ItemId,
    /// Score whose meaning depends on `score_kind`.
    pub score: f32,
    /// Tells the consumer what `score` represents.
    pub score_kind: ScoreKind,
    /// 1-based rank in the lexical ranker, or `None` if absent.
    pub lexical_rank: Option<usize>,
    /// 1-based rank in the semantic ranker, or `None` if absent.
    pub semantic_rank: Option<usize>,
    /// Highlighted snippet from the lexical hit, when available.
    /// `None` when `include_snippets` is false OR when the hit did not appear
    /// in the lexical ranker.
    pub snippet: Option<String>,
}

/// Results of a hybrid search query.
#[derive(Debug, Clone, Serialize)]
pub struct HybridSearchResults {
    /// Hits in descending `score` order, with `ItemId` ascending as tie-break.
    pub hits: Vec<HybridHit>,
    /// Wall-clock duration of the entire `HybridSearcher::search` call.
    pub elapsed: Duration,
    /// Number of distinct documents considered for fusion (lexical ∪ semantic).
    pub total_fused: usize,
    /// Number of hits the lexical ranker returned (before fusion), or `None`
    /// if the lexical ranker did not run.
    pub lexical_hits: Option<u64>,
    /// Number of hits the semantic ranker returned (before fusion), or `None`
    /// if the semantic ranker did not run.
    pub semantic_hits: Option<u64>,
}

use crate::index::Index;
use crate::vector_index::EmbedderIndex;

/// Combines an optional lexical ([`Index`]) and an optional semantic
/// ([`EmbedderIndex`]) backend, dispatching `search` to the appropriate code path
/// based on which references are present.
///
/// Construct via [`HybridSearcher::new`], [`HybridSearcher::lexical_only`], or
/// [`HybridSearcher::semantic_only`] depending on what's available at the call
/// site. The CLI's `cmd_search` chooses based on directory probes when
/// `--mode auto`; explicit modes pick directly.
pub struct HybridSearcher<'a> {
    /// Lexical (Tantivy) index, when available.
    pub lexical: Option<&'a Index>,
    /// Semantic (`USearch` + embedder) index, when available.
    pub semantic: Option<&'a EmbedderIndex>,
}

impl<'a> HybridSearcher<'a> {
    /// Both rankers present; [`Self::search`] will fuse via RRF.
    #[must_use]
    pub const fn new(lexical: &'a Index, semantic: &'a EmbedderIndex) -> Self {
        Self {
            lexical: Some(lexical),
            semantic: Some(semantic),
        }
    }

    /// Lexical only; [`Self::search`] returns BM25-scored hits with
    /// `semantic_rank == None`.
    #[must_use]
    pub const fn lexical_only(lexical: &'a Index) -> Self {
        Self {
            lexical: Some(lexical),
            semantic: None,
        }
    }

    /// Semantic only; [`Self::search`] returns cosine-scored hits with
    /// `lexical_rank == None` and `snippet == None`.
    #[must_use]
    pub const fn semantic_only(semantic: &'a EmbedderIndex) -> Self {
        Self {
            lexical: None,
            semantic: Some(semantic),
        }
    }
}

use std::collections::HashMap;

/// Compute Reciprocal Rank Fusion scores for the union of two ranked
/// `ItemId` lists.
///
/// For each unique `ItemId` `d` appearing at rank `r_i` in ranker `i` (1-based),
/// the RRF score is `Σ_i 1 / (k + r_i)`.
///
/// Returns `(id, score)` pairs sorted by `score` descending, with `ItemId`
/// ascending as the deterministic tie-break.
///
/// # Panics
///
/// Does not panic. `k = 0` is allowed (degenerate but well-defined).
#[must_use]
pub fn rrf_fuse(lexical: &[ItemId], semantic: &[ItemId], k: usize) -> Vec<(ItemId, f32)> {
    #[allow(clippy::cast_precision_loss)]
    let k_f = k as f32;
    let mut scores: HashMap<ItemId, f32> = HashMap::new();
    for (rank0, id) in lexical.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let r = (rank0 + 1) as f32;
        *scores.entry(*id).or_insert(0.0) += 1.0 / (k_f + r);
    }
    for (rank0, id) in semantic.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let r = (rank0 + 1) as f32;
        *scores.entry(*id).or_insert(0.0) += 1.0 / (k_f + r);
    }
    let mut fused: Vec<(ItemId, f32)> = scores.into_iter().collect();
    // Sort by score descending; tie-break by ItemId ascending.
    // `f32` is not `Ord` so we use `partial_cmp` with `Equal` fallback.
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    fused
}

use crate::error::{Error, Result};
use crate::result::SearchOptions;
use crate::semantic_query::SemanticSearchOptions;

impl HybridSearcher<'_> {
    /// Run a search against whichever rankers this `HybridSearcher` holds.
    ///
    /// - Both present: RRF-fused results, `score_kind = Rrf`.
    /// - Lexical only: BM25-scored hits, `score_kind = Bm25`.
    /// - Semantic only: cosine-scored hits, `score_kind = Cosine`.
    ///
    /// # Errors
    ///
    /// Returns whatever error the underlying ranker raises
    /// ([`Error::QueryParse`], [`Error::Tantivy`], [`Error::Embedding`],
    /// [`Error::Usearch`], etc.).
    pub fn search(
        &self,
        query: &str,
        opts: &HybridSearchOptions,
    ) -> Result<HybridSearchResults> {
        let start = std::time::Instant::now();
        let fetch_n = opts.limit.saturating_mul(opts.fetch_multiplier).max(1);

        match (self.lexical, self.semantic) {
            (Some(lex), None) => Self::search_lexical_only(lex, query, opts, fetch_n, start),
            (None, Some(sem)) => Self::search_semantic_only(sem, query, opts, fetch_n, start),
            (Some(_lex), Some(_sem)) => {
                // Task 7 replaces this stub with RRF fusion.
                Err(Error::NoIndexes)
            }
            (None, None) => Err(Error::NoIndexes),
        }
    }

    fn search_lexical_only(
        lex: &Index,
        query: &str,
        opts: &HybridSearchOptions,
        fetch_n: usize,
        start: std::time::Instant,
    ) -> Result<HybridSearchResults> {
        let parsed = crate::Query::parse(query)?;
        let lex_opts = SearchOptions {
            limit: opts.limit,
            offset: 0,
            include_snippets: opts.include_snippets,
        };
        let _ = fetch_n; // unused in lexical-only (we ask for `limit` directly)
        let res = lex.search(&parsed, lex_opts)?;
        let lexical_hits = Some(res.total_matched);
        let hits: Vec<HybridHit> = res
            .hits
            .into_iter()
            .enumerate()
            .map(|(rank0, h)| HybridHit {
                id: h.id,
                score: h.score,
                score_kind: ScoreKind::Bm25,
                lexical_rank: Some(rank0 + 1),
                semantic_rank: None,
                snippet: h.snippet,
            })
            .collect();
        let total_fused = hits.len();
        Ok(HybridSearchResults {
            hits,
            elapsed: start.elapsed(),
            total_fused,
            lexical_hits,
            semantic_hits: None,
        })
    }

    fn search_semantic_only(
        sem: &EmbedderIndex,
        query: &str,
        opts: &HybridSearchOptions,
        fetch_n: usize,
        start: std::time::Instant,
    ) -> Result<HybridSearchResults> {
        let sem_opts = SemanticSearchOptions {
            limit: opts.limit,
            min_score: 0.0,
        };
        let _ = fetch_n; // unused in semantic-only
        let res = sem.semantic_search(query, &sem_opts)?;
        let semantic_hits = Some(res.total_indexed);
        let hits: Vec<HybridHit> = res
            .hits
            .into_iter()
            .enumerate()
            .map(|(rank0, h)| HybridHit {
                id: h.id,
                score: h.score,
                score_kind: ScoreKind::Cosine,
                lexical_rank: None,
                semantic_rank: Some(rank0 + 1),
                snippet: None,
            })
            .collect();
        let total_fused = hits.len();
        Ok(HybridSearchResults {
            hits,
            elapsed: start.elapsed(),
            total_fused,
            lexical_hits: None,
            semantic_hits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testing::MockEmbedder;
    use crate::{EmbedderIndex, Index};
    use tempfile::TempDir;

    #[test]
    fn new_holds_both_index_references() {
        let dir = TempDir::new().unwrap();
        let lex = Index::open(dir.path().join("lex")).unwrap();
        let sem = EmbedderIndex::open(
            dir.path().join("sem"),
            Box::new(MockEmbedder::default()),
        )
        .unwrap();
        let s = HybridSearcher::new(&lex, &sem);
        assert!(s.lexical.is_some(), "lexical must be set");
        assert!(s.semantic.is_some(), "semantic must be set");
    }

    #[test]
    fn lexical_only_constructor_omits_semantic() {
        let dir = TempDir::new().unwrap();
        let lex = Index::open(dir.path().join("lex")).unwrap();
        let s = HybridSearcher::lexical_only(&lex);
        assert!(s.lexical.is_some());
        assert!(s.semantic.is_none());
    }

    #[test]
    fn semantic_only_constructor_omits_lexical() {
        let dir = TempDir::new().unwrap();
        let sem = EmbedderIndex::open(
            dir.path().join("sem"),
            Box::new(MockEmbedder::default()),
        )
        .unwrap();
        let s = HybridSearcher::semantic_only(&sem);
        assert!(s.lexical.is_none());
        assert!(s.semantic.is_some());
    }

    #[test]
    fn default_options_match_spec() {
        let o = HybridSearchOptions::default();
        assert_eq!(o.limit, 20);
        assert_eq!(o.fetch_multiplier, 3);
        assert_eq!(o.rrf_k, 60);
        assert!(o.include_snippets);
    }

    #[test]
    fn score_kind_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&ScoreKind::Rrf).unwrap(), "\"rrf\"");
        assert_eq!(serde_json::to_string(&ScoreKind::Bm25).unwrap(), "\"bm25\"");
        assert_eq!(
            serde_json::to_string(&ScoreKind::Cosine).unwrap(),
            "\"cosine\""
        );
    }

    use singularmem_core::ItemId;
    use std::str::FromStr;

    fn id(s: &str) -> ItemId {
        ItemId::from_str(s).expect("valid ULID")
    }

    #[test]
    fn rrf_fuse_overlapping_results() {
        let a = id("01ARZ3NDEKTSV4RRFFQ69G5FAV");
        let b = id("01BX5ZZKBKACTAV9WEVGEMMVRZ");
        // Lexical: a@1, b@2. Semantic: b@1, a@2.
        let lex = vec![a, b];
        let sem = vec![b, a];
        let fused = rrf_fuse(&lex, &sem, 60);
        // Both docs in both rankers; both get 1/(60+1) + 1/(60+2) = 0.032520...
        assert_eq!(fused.len(), 2);
        let (got_a, got_b) = if fused[0].0 == a {
            (&fused[0], &fused[1])
        } else {
            (&fused[1], &fused[0])
        };
        let expected = 1.0 / 61.0 + 1.0 / 62.0;
        assert!(
            (got_a.1 - expected).abs() < 1e-6,
            "a score {} should be {}",
            got_a.1,
            expected
        );
        assert!((got_b.1 - expected).abs() < 1e-6);
    }

    #[test]
    fn rrf_fuse_disjoint_results() {
        let a = id("01ARZ3NDEKTSV4RRFFQ69G5FAV");
        let b = id("01BX5ZZKBKACTAV9WEVGEMMVRZ");
        // a only in lexical (rank 1); b only in semantic (rank 1).
        let lex = vec![a];
        let sem = vec![b];
        let fused = rrf_fuse(&lex, &sem, 60);
        assert_eq!(fused.len(), 2);
        for (_id, score) in &fused {
            let expected = 1.0 / 61.0;
            assert!(
                (*score - expected).abs() < 1e-6,
                "single-ranker doc gets 1/(k+1): got {score}"
            );
        }
    }

    #[test]
    fn rrf_fuse_ties_break_by_item_id_ascending() {
        let a = id("01ARZ3NDEKTSV4RRFFQ69G5FAV");
        let b = id("01BX5ZZKBKACTAV9WEVGEMMVRZ");
        // Identical ranks → identical RRF scores → sort by ItemId ascending.
        let lex = vec![b, a]; // b@1, a@2
        let sem = vec![a, b]; // a@1, b@2
        let fused = rrf_fuse(&lex, &sem, 60);
        // Both have score 1/61 + 1/62. Tie-break by id ascending → a first.
        assert_eq!(fused[0].0, a, "lower ItemId first on tie");
        assert_eq!(fused[1].0, b);
    }

    #[test]
    fn rrf_fuse_empty_inputs_returns_empty() {
        let fused = rrf_fuse(&[], &[], 60);
        assert!(fused.is_empty());
    }

    #[allow(unused_imports)]
    use crate::query::Query as ParsedQuery;
    use singularmem_core::{NewItem, Store};

    #[test]
    fn lexical_only_search_returns_bm25_scored_hits() {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let lex_path = dir.path().join("lex");

        let hook = Index::open(&lex_path).unwrap();
        let store = Store::open_with_hook(&store_path, Box::new(hook)).unwrap();
        store
            .ingest(NewItem::text("the quick brown fox jumps"))
            .unwrap();
        store
            .ingest(NewItem::text("lazy dogs sleep all day"))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(store);

        let lex = Index::open(&lex_path).unwrap();
        let searcher = HybridSearcher::lexical_only(&lex);
        let opts = HybridSearchOptions::default();
        let r = searcher.search("fox", &opts).expect("search ok");

        assert!(!r.hits.is_empty(), "expected at least one hit");
        for hit in &r.hits {
            assert_eq!(hit.score_kind, ScoreKind::Bm25);
            assert!(hit.lexical_rank.is_some());
            assert!(
                hit.semantic_rank.is_none(),
                "lexical-only must not populate semantic_rank"
            );
        }
        assert_eq!(r.semantic_hits, None);
        assert!(r.lexical_hits.is_some());
    }

    #[test]
    fn semantic_only_search_returns_cosine_scored_hits() {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let sem_path = dir.path().join("sem");

        let hook = EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default())).unwrap();
        let store = Store::open_with_hook(&store_path, Box::new(hook)).unwrap();
        store
            .ingest(NewItem::text("the quick brown fox jumps"))
            .unwrap();
        store
            .ingest(NewItem::text("lazy dogs sleep all day"))
            .unwrap();
        drop(store);

        let sem = EmbedderIndex::open(&sem_path, Box::new(MockEmbedder::default())).unwrap();
        let searcher = HybridSearcher::semantic_only(&sem);
        let opts = HybridSearchOptions::default();
        let r = searcher.search("fox", &opts).expect("search ok");

        assert!(!r.hits.is_empty());
        for hit in &r.hits {
            assert_eq!(hit.score_kind, ScoreKind::Cosine);
            assert!(hit.semantic_rank.is_some());
            assert!(hit.lexical_rank.is_none());
            assert!(
                hit.snippet.is_none(),
                "semantic-only has no snippet source"
            );
        }
        assert_eq!(r.lexical_hits, None);
        assert!(r.semantic_hits.is_some());
    }
}
