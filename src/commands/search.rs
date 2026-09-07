//! Full-text/semantic search verbs: `search`, `retrieve`, `semantic-search`
//! (a deprecated forwarder onto `search`), and `reindex`. Also owns the
//! adapter registry (`known_adapters`/`find_adapter`), which `commands::wakeup`
//! shares for `wake-up`'s `--adapter`.

use std::io::{self, Write};
use std::path::Path;

use singularmem_core::Store;

use crate::commands::index::{
    derive_index_path, derive_vectors_path, open_or_rebuild_index, resolve_search_mode,
    ResolvedSearchMode,
};
use crate::commands::{
    ListFormat, ReindexArgs, RetrieveArgs, ScopeArgs, SearchArgs, SearchMode, SemanticSearchArgs,
};
use crate::CliError;

/// Items per [`singularmem_core::IndexHook::on_ingest_batch`] call in the
/// `reindex --with-embeddings` embedding loop.
const REINDEX_BATCH: usize = 500;

pub fn cmd_search(store: &Store, store_path: &Path, args: &SearchArgs) -> Result<(), CliError> {
    use singularmem_search::{EmbedderIndex, HybridSearchOptions, HybridSearcher, Index};

    let filter = args.scope.to_filter()?;
    let resolved = resolve_search_mode(store_path, args.mode)?;
    let ResolvedSearchMode {
        mode: resolved_mode,
        tantivy_path,
        vectors_path,
    } = resolved;

    let query_str = args.queries.join(" ");
    let opts = HybridSearchOptions {
        limit: args.limit,
        fetch_multiplier: args.fetch_multiplier,
        rrf_k: args.rrf_k,
        include_snippets: !args.no_snippets,
        scope: filter.clone(),
    };

    // Open whichever indexes the resolved mode requires.
    let lex_opt: Option<Index> =
        if matches!(resolved_mode, SearchMode::Lexical | SearchMode::Hybrid) {
            Some(Index::open(&tantivy_path)?)
        } else {
            None
        };
    let sem_opt: Option<EmbedderIndex> =
        if matches!(resolved_mode, SearchMode::Semantic | SearchMode::Hybrid) {
            let embedder: Box<dyn singularmem_search::Embedder> =
                match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                    Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                    _ => Box::new(singularmem_search::FastembedEmbedder::new()?),
                };
            Some(EmbedderIndex::open(&vectors_path, embedder)?)
        } else {
            None
        };

    let searcher = match (&lex_opt, &sem_opt) {
        (Some(l), Some(s)) => HybridSearcher::new(l, s),
        (Some(l), None) => HybridSearcher::lexical_only(l),
        (None, Some(s)) => HybridSearcher::semantic_only(s),
        (None, None) => unreachable!("pre-flight guarantees at least one index"),
    };
    let searcher = if filter.is_some() && sem_opt.is_some() {
        searcher.with_scope_lookup(store)
    } else {
        searcher
    };
    let results = searcher.search(&query_str, &opts)?;

    render_search_results(&results, args)?;
    Ok(())
}

fn render_search_results(
    results: &singularmem_search::HybridSearchResults,
    args: &SearchArgs,
) -> Result<(), CliError> {
    use singularmem_search::ScoreKind;

    if results.hits.is_empty() {
        tracing::info!("0 matches");
        return Ok(());
    }

    let mut out = io::stdout().lock();
    if args.json {
        serde_json::to_writer(&mut out, results)?;
        writeln!(out)?;
        return Ok(());
    }

    for hit in &results.hits {
        let tag = match hit.score_kind {
            ScoreKind::Rrf => "rrf",
            ScoreKind::Bm25 => "bm25",
            ScoreKind::Cosine => "cos",
        };
        let snip = hit.snippet.as_deref().unwrap_or("").replace('\n', " ");
        if args.show_ranks {
            let lex = hit
                .lexical_rank
                .map_or_else(|| "—".to_string(), |r| r.to_string());
            let sem = hit
                .semantic_rank
                .map_or_else(|| "—".to_string(), |r| r.to_string());
            writeln!(
                out,
                "{}  {}={:.4}  lex={}  sem={}  {}",
                hit.id, tag, hit.score, lex, sem, snip
            )?;
        } else {
            writeln!(out, "{}  {}={:.4}  {}", hit.id, tag, hit.score, snip)?;
        }
    }
    Ok(())
}

pub fn cmd_retrieve(store: &Store, store_path: &Path, args: &RetrieveArgs) -> Result<(), CliError> {
    use singularmem_retrieve::{RetrieveOptions, Retriever};
    use singularmem_search::{EmbedderIndex, HybridSearchOptions, HybridSearcher, Index};

    // Adapter lookup before any I/O so unknown-adapter errors fail fast.
    let adapter = find_adapter(&args.adapter)?;
    let adapter = &*adapter;

    let filter = args.scope.to_filter()?;

    // Mode resolution + sidecar probing — same helper cmd_search uses.
    let ResolvedSearchMode {
        mode: resolved_mode,
        tantivy_path,
        vectors_path,
    } = resolve_search_mode(store_path, args.mode)?;

    let query_str = args.queries.join(" ");
    let search_opts = HybridSearchOptions {
        limit: args
            .limit
            .saturating_mul(args.fetch_multiplier)
            .max(args.limit),
        fetch_multiplier: args.fetch_multiplier,
        rrf_k: args.rrf_k,
        include_snippets: false, // we use full content, not snippets
        scope: filter.clone(),
    };
    let opts = RetrieveOptions {
        max_blocks: args.limit,
        min_score: args.min_score,
        search: search_opts,
        scope: filter,
    };

    // Open whichever indexes the resolved mode requires.
    let lex_opt: Option<Index> =
        if matches!(resolved_mode, SearchMode::Lexical | SearchMode::Hybrid) {
            Some(Index::open(&tantivy_path)?)
        } else {
            None
        };
    let sem_opt: Option<EmbedderIndex> =
        if matches!(resolved_mode, SearchMode::Semantic | SearchMode::Hybrid) {
            let embedder: Box<dyn singularmem_search::Embedder> =
                match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                    Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                    _ => Box::new(singularmem_search::FastembedEmbedder::new()?),
                };
            Some(EmbedderIndex::open(&vectors_path, embedder)?)
        } else {
            None
        };

    let searcher = match (&lex_opt, &sem_opt) {
        (Some(l), Some(s)) => HybridSearcher::new(l, s),
        (Some(l), None) => HybridSearcher::lexical_only(l),
        (None, Some(s)) => HybridSearcher::semantic_only(s),
        (None, None) => unreachable!("pre-flight guarantees at least one index"),
    };
    let retriever = Retriever::new(store, &searcher);
    let context = retriever.retrieve(&query_str, &opts)?;

    let mut out = io::stdout().lock();
    if args.json {
        serde_json::to_writer(&mut out, &context)?;
        writeln!(out)?;
    } else {
        let formatted = adapter.format(&context);
        write!(out, "{formatted}")?;
    }
    drop(out);

    if args.show_elapsed {
        eprintln!(
            "Retrieved {} blocks in {:.2}ms (considered {})",
            context.blocks.len(),
            context.elapsed.as_secs_f64() * 1000.0,
            context.total_considered
        );
    }
    Ok(())
}

pub fn cmd_semantic_search(
    store: &Store,
    store_path: &Path,
    args: &SemanticSearchArgs,
) -> Result<(), CliError> {
    use std::sync::OnceLock;
    static DEPRECATION_NOTICE: OnceLock<()> = OnceLock::new();
    DEPRECATION_NOTICE.get_or_init(|| {
        eprintln!("note: 'semantic-search' is deprecated; use 'search --mode semantic'");
    });

    // Forward through cmd_search with mode=Semantic.
    let forwarded = SearchArgs {
        queries: args.queries.clone(),
        mode: SearchMode::Semantic,
        limit: args.limit,
        offset: 0,
        fetch_multiplier: 3,
        rrf_k: 60,
        no_snippets: true, // semantic mode has no snippets anyway
        show_ranks: false,
        json: matches!(args.format, ListFormat::Jsonl),
        format: args.format,
        scope: ScopeArgs {
            scope: None,
            scope_exact: false,
        },
    };
    cmd_search(store, store_path, &forwarded)
}

/// Embed and index one batch of items via
/// [`singularmem_core::IndexHook::on_ingest_batch`], then clear `batch` and
/// report progress. A no-op if `batch` is empty (used as the trailing flush
/// after the main loop).
fn flush_embedding_batch(
    embedder_idx: &singularmem_search::EmbedderIndex,
    batch: &mut Vec<singularmem_core::Item>,
    done: &mut usize,
    quiet: bool,
) -> Result<(), CliError> {
    if batch.is_empty() {
        return Ok(());
    }
    singularmem_core::IndexHook::on_ingest_batch(embedder_idx, batch)
        .map_err(|e| CliError::IndexOpen(e.to_string()))?;
    *done += batch.len();
    batch.clear();
    if !quiet {
        tracing::info!("reindex (embeddings): {} items", done);
    }
    Ok(())
}

pub fn cmd_reindex(store: &Store, store_path: &Path, args: &ReindexArgs) -> Result<(), CliError> {
    // Phase 1: Tantivy lexical reindex (always).
    let index_path = derive_index_path(store_path);
    let index = open_or_rebuild_index(&index_path)?;
    let progress = |n: u64| {
        if !args.quiet {
            tracing::info!("reindex (tantivy): {n} items processed");
        }
    };
    let count = index
        .reindex_from(store.list()?.filter_map(Result::ok), progress)
        .map_err(|e| CliError::IndexOpen(e.to_string()))?;
    tracing::info!("reindex (tantivy): {count} items total");

    // Phase 2: Embedder / vector reindex (only when --with-embeddings is given).
    if args.with_embeddings {
        let vectors_path = derive_vectors_path(store_path);

        if args.reset_vectors {
            if !args.force {
                return Err(CliError::Usage(
                    "--reset-vectors requires --force to confirm the destructive operation".into(),
                ));
            }
            if vectors_path.exists() {
                std::fs::remove_dir_all(&vectors_path).map_err(CliError::Io)?;
                tracing::warn!(
                    path = %vectors_path.display(),
                    "deleted existing vector index"
                );
            }
        }

        let model = match args.embedding_model.as_str() {
            "all-mini-lm-l6-v2" => singularmem_search::EmbeddingModel::AllMiniLmL6V2,
            "bge-small-en" => singularmem_search::EmbeddingModel::BgeSmallEnV15,
            "nomic-embed" => singularmem_search::EmbeddingModel::NomicEmbedTextV15,
            other => {
                return Err(CliError::Usage(format!(
                    "unknown --embedding-model: {other}"
                )))
            }
        };

        let embedder: Box<dyn singularmem_search::Embedder> =
            match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                _ => Box::new(
                    singularmem_search::FastembedEmbedder::with_model(model)
                        .map_err(|e| CliError::IndexOpen(format!("embedder init: {e}")))?,
                ),
            };

        let embedder_idx = singularmem_search::EmbedderIndex::open(&vectors_path, embedder)
            .map_err(|e| CliError::IndexOpen(e.to_string()))?;

        let mut batch: Vec<singularmem_core::Item> = Vec::with_capacity(REINDEX_BATCH);
        let mut done = 0usize;
        for item_r in store.list()? {
            batch.push(item_r?);
            if batch.len() == REINDEX_BATCH {
                flush_embedding_batch(&embedder_idx, &mut batch, &mut done, args.quiet)?;
            }
        }
        flush_embedding_batch(&embedder_idx, &mut batch, &mut done, args.quiet)?;
        singularmem_core::IndexHook::commit(&embedder_idx)
            .map_err(|e| CliError::IndexOpen(e.to_string()))?;
        tracing::info!("reindex (embeddings) complete");
    }

    Ok(())
}

/// Registry of available adapters. Sub-projects 3b/3c/3d each add one line
/// here AND one line to the root `Cargo.toml` `[dependencies]` section.
///
/// Order matters for the unknown-adapter error message: list adapters in
/// the order they should appear when the CLI tells the user what's
/// available.
fn known_adapters() -> Vec<Box<dyn singularmem_retrieve::Adapter>> {
    vec![
        Box::new(singularmem_retrieve::PlainAdapter),
        Box::new(singularmem_adapter_claude::ClaudeAdapter),
        Box::new(singularmem_adapter_openai::OpenAiAdapter),
        Box::new(singularmem_adapter_gemini::GeminiAdapter),
    ]
}

/// Look up an adapter by name in [`known_adapters`].
///
/// # Errors
/// `CliError::Usage` naming the known adapters when `name` matches none.
pub fn find_adapter(name: &str) -> Result<Box<dyn singularmem_retrieve::Adapter>, CliError> {
    let adapters = known_adapters();
    let Some(pos) = adapters.iter().position(|a| a.name() == name) else {
        let known: Vec<&str> = adapters.iter().map(|a| a.name()).collect();
        return Err(CliError::Usage(format!(
            "unknown adapter '{name}'; known adapters: {}",
            known.join(", ")
        )));
    };
    Ok(adapters
        .into_iter()
        .nth(pos)
        .unwrap_or_else(|| unreachable!()))
}
