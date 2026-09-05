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
        .find_map(|t| t.strip_prefix(SESSION_TAG_PREFIX))
        .and_then(|n| n.parse().ok())
}
