//! Evaluate one `LongMemEval` question in an isolated temporary store.

use std::collections::BTreeMap;
use std::fmt::Write as _;
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
    /// Build a `RunConfig`, sorting and deduplicating `ks` and rejecting an
    /// empty `ks` or a `k == 0` — the invariant the `ks` field doc promises.
    ///
    /// # Errors
    /// Returns a message naming the problem when `ks` is empty (after
    /// dedup) or contains a `0`.
    pub fn new(modes: Vec<SearchMode>, mut ks: Vec<usize>) -> Result<Self, String> {
        ks.sort_unstable();
        ks.dedup();
        if ks.is_empty() {
            return Err("ks must not be empty".to_string());
        }
        if ks.contains(&0) {
            return Err("k must be >= 1, got 0".to_string());
        }
        Ok(Self { modes, ks })
    }

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
pub fn run_question(
    q: &Question,
    cfg: &RunConfig,
    embedder: Option<&SharedEmbedder>,
) -> QuestionResult {
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
        items_ingested: 0,
        query_us: BTreeMap::new(),
        error: None,
    };
    result.evidence.sort();
    if let Err(e) = evaluate(q, cfg, embedder, scratch_root, &mut result) {
        result.hits.clear();
        result.query_us.clear();
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
    let (ingest_ms, n_ingested) = ingest_question(
        q,
        embedder,
        needs_embedder,
        &store_path,
        &lex_path,
        &sem_path,
    )?;
    out.ingest_ms = ingest_ms;
    out.items_ingested = n_ingested;

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

    // `Store::ingest`/`ingest_many` log and swallow `on_ingest`/`commit`
    // hook failures (see `singularmem_core::ingest`), so a broken index
    // would otherwise silently yield empty hits with `error: None`. Catch
    // that here by checking that every ingested item actually landed in
    // each index that is in use.
    let lex_count = lex
        .doc_count()
        .map_err(|e| format!("counting lexical docs: {e}"))?;
    let sem_count = match sem.as_ref() {
        Some(s) => Some(
            s.vector_index()
                .doc_count()
                .map_err(|e| format!("counting vector docs: {e}"))?,
        ),
        None => None,
    };
    verify_doc_counts(n_ingested, lex_count, sem_count)?;

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
            // Semantic cosine similarity can be negative, so a floor of
            // 0.0 does real work there, culling anti-correlated
            // candidates; BM25 and RRF scores are always non-negative, so
            // this floor never excludes anything in the lexical or hybrid
            // columns — it only ever affects the semantic column.
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
        let us = t.elapsed().as_micros().try_into().unwrap_or(u64::MAX);

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
        out.query_us.insert(mode, us);
    }
    Ok(())
}

/// Open the lexical (and, when needed, vector) index hooks over a fresh
/// store at `store_path` and ingest every item across all of `q`'s haystack
/// sessions in one [`Store::ingest_many`] call. Returns `(ingest_ms,
/// n_ingested)`, where `ingest_ms` is timed from just after the store and
/// hooks are open (excluding index setup) to just after ingestion returns.
///
/// One `ingest_many` call for the whole question, rather than one
/// `Store::ingest` call per item: `Store::ingest` fires `on_ingest` +
/// `commit` on the index hooks for every single item (a Tantivy commit +
/// reader reload, and a full `USearch` save, per item), while `ingest_many`
/// inserts all items in one `SQLite` transaction and fires the hooks'
/// `on_ingest` + `commit` once at the end — the difference between ~90
/// ms/item and one hook commit per question.
fn ingest_question(
    q: &Question,
    embedder: Option<&SharedEmbedder>,
    needs_embedder: bool,
    store_path: &Path,
    lex_path: &Path,
    sem_path: &Path,
) -> Result<(u64, usize), String> {
    let mut hooks: Vec<Box<dyn IndexHook>> = vec![Box::new(
        Index::open(lex_path).map_err(|e| format!("opening lexical index: {e}"))?,
    )];
    if let Some(emb) = embedder.filter(|_| needs_embedder) {
        hooks.push(Box::new(
            EmbedderIndex::open(sem_path, emb.boxed())
                .map_err(|e| format!("opening vector index: {e}"))?,
        ));
    }
    let store = Store::open_with_hook(store_path, Box::new(MultiHook::new(hooks)))
        .map_err(|e| format!("opening store: {e}"))?;

    // Start the clock after the store and hooks are open: `ingest_ms` is
    // meant to measure ingestion throughput, not index setup.
    let t0 = Instant::now();
    let items: Vec<NewItem> = q
        .haystack
        .iter()
        .enumerate()
        .flat_map(|(session_index, session)| items_for(q, session_index, session))
        .collect();
    // `n_ingested` is the length of the `Vec<Item>` `ingest_many` actually
    // returned, not the pre-chunking count computed above: the two agree
    // whenever the call succeeds, but the return value is the ground truth
    // for "items actually ingested" that `ingest_items_per_s` needs.
    let n_ingested = store
        .ingest_many(items)
        .map_err(|e| format!("ingesting: {e}"))?
        .len();
    let ingest_ms = t0.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    // `store` (and the hooks) drop here, closing the SQLite connection so
    // the indexes can be reopened cleanly for querying.
    Ok((ingest_ms, n_ingested))
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
                // `String::write_fmt` via the `Write` trait is infallible here.
                let _ = write!(external_id, "#{chunk}");
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
        .filter_map(|t| t.strip_prefix(SESSION_TAG_PREFIX))
        .find_map(|n| n.parse().ok())
}

/// Verify that every item ingested for this question actually landed in
/// each index hook that is in use. `Store::ingest`/`ingest_many` only log
/// (`tracing::warn!`) an `on_ingest`/`commit` hook failure and otherwise
/// swallow it, so a broken index hook would otherwise silently produce
/// empty search results with `error: None` instead of failing the
/// question. `sem_count` is `None` when no vector index is in use.
///
/// # Errors
/// Returns an error naming which index is short and by how much.
fn verify_doc_counts(
    n_ingested: usize,
    lex_count: u64,
    sem_count: Option<u64>,
) -> Result<(), String> {
    let n = n_ingested as u64;
    if lex_count != n {
        return Err(format!(
            "index hook dropped items: lexical has {lex_count} of {n} ingested items"
        ));
    }
    if let Some(sem_count) = sem_count {
        if sem_count != n {
            return Err(format!(
                "index hook dropped items: vector index has {sem_count} of {n} ingested items"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{verify_doc_counts, RunConfig};
    use crate::metrics::SearchMode;

    #[test]
    fn run_config_new_sorts_and_dedups_ks() {
        let cfg = RunConfig::new(vec![SearchMode::Lexical], vec![5, 1, 5, 10, 1]).unwrap();
        assert_eq!(cfg.ks, vec![1, 5, 10]);
    }

    #[test]
    fn run_config_new_rejects_empty_ks() {
        assert!(RunConfig::new(vec![SearchMode::Lexical], vec![]).is_err());
    }

    #[test]
    fn run_config_new_rejects_zero_k() {
        assert!(RunConfig::new(vec![SearchMode::Lexical], vec![0, 1]).is_err());
    }

    #[test]
    fn verify_doc_counts_passes_when_all_counts_match() {
        assert!(verify_doc_counts(3, 3, None).is_ok());
        assert!(verify_doc_counts(3, 3, Some(3)).is_ok());
    }

    #[test]
    fn verify_doc_counts_catches_a_short_lexical_index() {
        let err = verify_doc_counts(5, 2, None).unwrap_err();
        assert!(err.contains("lexical"), "{err}");
        assert!(err.contains("dropped"), "{err}");
        assert!(err.contains("2 of 5"), "{err}");
    }

    #[test]
    fn verify_doc_counts_catches_a_short_vector_index() {
        let err = verify_doc_counts(5, 5, Some(1)).unwrap_err();
        assert!(err.contains("vector index"), "{err}");
        assert!(err.contains("dropped"), "{err}");
        assert!(err.contains("1 of 5"), "{err}");
    }
}
