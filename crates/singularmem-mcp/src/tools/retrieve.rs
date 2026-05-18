//! `memory_retrieve` tool: takes a query + optional limit + optional
//! adapter; returns adapter-formatted memory blocks ready to inject
//! into an LLM prompt.

use serde::{Deserialize, Serialize};

use singularmem_retrieve::{Adapter, RetrieveOptions, Retriever};
use singularmem_search::{EmbedderIndex, HybridSearchOptions, HybridSearcher, Index};

use crate::{Config, Error, Result};

/// JSON-deserialised arguments for the `memory_retrieve` tool.
///
/// Matches the JSON schema declared when the tool is registered with rmcp.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryRetrieveArgs {
    /// Natural-language query for the memory search.
    pub query: String,
    /// Maximum number of blocks to return. Clamped to `[1, 50]`.
    /// Default: 10.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Which adapter to format with. Falls back to the server's
    /// `default_adapter` when absent.
    #[serde(default)]
    pub adapter: Option<String>,
}

/// Output of the `memory_retrieve` handler. The MCP transport layer
/// wraps this in a `CallToolResult` text content block; the handler
/// itself just returns the formatted string.
#[derive(Debug, Clone)]
pub struct MemoryRetrieveOutput {
    /// Adapter-formatted memory blocks ready to embed in a prompt.
    pub text: String,
}

/// Handle a `tools/call` for `memory_retrieve`.
///
/// # Errors
///
/// - [`Error::Retrieve`] wrapping [`singularmem_retrieve::Error::EmptyQuery`]
///   for empty/whitespace queries.
/// - [`Error::UnknownAdapter`] if `args.adapter` is not in the registry.
/// - [`Error::Search`] / [`Error::Core`] for downstream failures
///   (missing indexes, store I/O, etc.).
pub fn handle_memory_retrieve(
    args: &MemoryRetrieveArgs,
    config: &Config,
) -> Result<MemoryRetrieveOutput> {
    // 1. Resolve adapter (request arg → server default).
    let adapter_name = args.adapter.as_deref().unwrap_or(&config.default_adapter);
    let adapter: &dyn Adapter = config
        .known_adapters
        .iter()
        .find(|a| a.name() == adapter_name)
        .map(std::convert::AsRef::as_ref)
        .ok_or_else(|| Error::UnknownAdapter(adapter_name.to_string()))?;

    // 2. Clamp limit to [1, 50] per spec.
    let limit = args.limit.unwrap_or(10).clamp(1, 50);
    let opts = RetrieveOptions {
        max_blocks: limit,
        min_score: 0.0,
        search: HybridSearchOptions::default(),
    };

    // 3. Open store + indexes per-request. The spec is explicit: no
    // caching. Microsecond-scale per the v0.1.0 bench numbers.
    let store = crate::tools::util::open_store_for_reading(config)?;
    let tantivy_path = derive_index_path(&config.store_path);
    let vectors_path = derive_vectors_path(&config.store_path);
    let has_lex = tantivy_path.exists();
    let has_sem = vectors_path.exists();

    if !has_lex && !has_sem {
        return Err(Error::Search(singularmem_search::Error::NoIndexes));
    }

    let lex = if has_lex {
        Some(Index::open(&tantivy_path)?)
    } else {
        None
    };
    let sem = if has_sem {
        let embedder: Box<dyn singularmem_search::Embedder> =
            match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                _ => Box::new(singularmem_search::FastembedEmbedder::new()?),
            };
        Some(EmbedderIndex::open(&vectors_path, embedder)?)
    } else {
        None
    };

    let searcher = match (&lex, &sem) {
        (Some(l), Some(s)) => HybridSearcher::new(l, s),
        (Some(l), None) => HybridSearcher::lexical_only(l),
        (None, Some(s)) => HybridSearcher::semantic_only(s),
        (None, None) => unreachable!("checked above"),
    };

    // 4. Retrieve + format.
    let retriever = Retriever::new(&store, &searcher);
    let ctx = retriever.retrieve(&args.query, &opts)?;
    let text = adapter.format(&ctx);

    Ok(MemoryRetrieveOutput { text })
}

/// Derive the Tantivy sidecar path from a store path. Mirrors the
/// root binary's `derive_index_path()`.
fn derive_index_path(store_path: &std::path::Path) -> std::path::PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".tantivy");
    std::path::PathBuf::from(s)
}

/// Derive the `USearch` sidecar path from a store path. Mirrors the
/// root binary's `derive_vectors_path()`.
fn derive_vectors_path(store_path: &std::path::Path) -> std::path::PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".vectors");
    std::path::PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::{NewItem, Store};
    use tempfile::TempDir;

    /// Seed a fresh store + both sidecars with `n` items using
    /// `MockEmbedder`, then return the tempdir + a `Config` pointing at
    /// the store. The `TempDir` guard must outlive the test.
    #[allow(clippy::missing_panics_doc)]
    fn seeded(n: usize, default_adapter: &str) -> (TempDir, Config) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let lex_path = dir.path().join("store.db.tantivy");
        let sem_path = dir.path().join("store.db.vectors");

        // Wire MultiHook so ingests populate both sidecars.
        let lex_hook = Index::open(&lex_path).unwrap();
        let sem_hook = EmbedderIndex::open(
            &sem_path,
            Box::new(singularmem_search::testing::MockEmbedder::default()),
        )
        .unwrap();
        let multi =
            singularmem_core::hook::MultiHook::new(vec![Box::new(lex_hook), Box::new(sem_hook)]);
        let store = Store::open_with_hook(&store_path, Box::new(multi)).unwrap();
        for i in 0..n {
            store
                .ingest(NewItem::text(format!("seed memory number {i}")))
                .unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(store);

        // Set the env var so the handler picks MockEmbedder.
        std::env::set_var("SINGULARMEM_TEST_EMBEDDER", "mock");

        let config = Config::new(store_path, default_adapter.to_string(), false);
        (dir, config)
    }

    #[test]
    fn handler_uses_default_adapter_when_arg_absent() {
        let (_dir, config) = seeded(3, "claude");
        let args = MemoryRetrieveArgs {
            query: "seed memory".to_string(),
            limit: None,
            adapter: None,
        };
        let out = handle_memory_retrieve(&args, &config).expect("ok");
        assert!(
            out.text.contains("<documents>"),
            "expected Claude XML shape: {}",
            out.text
        );
    }

    #[test]
    fn handler_uses_per_call_adapter_when_specified() {
        let (_dir, config) = seeded(3, "claude");
        let args = MemoryRetrieveArgs {
            query: "seed memory".to_string(),
            limit: None,
            adapter: Some("openai".to_string()),
        };
        let out = handle_memory_retrieve(&args, &config).expect("ok");
        assert!(
            out.text.contains("[1]"),
            "expected OpenAI bracket markers: {}",
            out.text
        );
        assert!(
            !out.text.contains("<documents>"),
            "should NOT have used Claude XML: {}",
            out.text
        );
    }

    #[test]
    fn handler_unknown_adapter_returns_unknown_adapter_error() {
        let (_dir, config) = seeded(1, "plain");
        let args = MemoryRetrieveArgs {
            query: "seed memory".to_string(),
            limit: None,
            adapter: Some("nonexistent".to_string()),
        };
        let r = handle_memory_retrieve(&args, &config);
        assert!(
            matches!(r, Err(Error::UnknownAdapter(ref s)) if s == "nonexistent"),
            "expected UnknownAdapter('nonexistent'): {r:?}"
        );
    }

    #[test]
    fn handler_respects_limit_arg() {
        let (_dir, config) = seeded(10, "plain");
        let args = MemoryRetrieveArgs {
            query: "seed memory".to_string(),
            limit: Some(3),
            adapter: None,
        };
        let out = handle_memory_retrieve(&args, &config).expect("ok");
        let heading_count = out.text.matches("## memory").count();
        assert!(
            heading_count <= 3,
            "expected ≤3 blocks, got {heading_count}: {}",
            out.text
        );
    }

    #[test]
    fn handler_caps_limit_at_50() {
        let (_dir, config) = seeded(60, "plain");
        let args = MemoryRetrieveArgs {
            query: "seed memory".to_string(),
            limit: Some(1000),
            adapter: None,
        };
        let out = handle_memory_retrieve(&args, &config).expect("ok");
        let heading_count = out.text.matches("## memory").count();
        assert!(
            heading_count <= 50,
            "expected ≤50 blocks (limit clamped), got {heading_count}"
        );
    }

    #[test]
    fn handler_empty_query_returns_empty_query_error() {
        let (_dir, config) = seeded(1, "plain");
        let args = MemoryRetrieveArgs {
            query: String::new(),
            limit: None,
            adapter: None,
        };
        let r = handle_memory_retrieve(&args, &config);
        assert!(
            matches!(
                r,
                Err(Error::Retrieve(singularmem_retrieve::Error::EmptyQuery))
            ),
            "expected Retrieve(EmptyQuery): {r:?}"
        );
    }

    #[test]
    fn handler_no_indexes_returns_search_no_indexes_error() {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let _store = Store::open(&store_path).unwrap();
        let config = Config::new(store_path, "plain".to_string(), false);

        let args = MemoryRetrieveArgs {
            query: "anything".to_string(),
            limit: None,
            adapter: None,
        };
        let r = handle_memory_retrieve(&args, &config);
        assert!(
            matches!(r, Err(Error::Search(singularmem_search::Error::NoIndexes))),
            "expected Search(NoIndexes): {r:?}"
        );
    }
}
