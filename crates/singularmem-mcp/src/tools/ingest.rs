//! `memory_ingest` tool — write a new memory to the store, with index
//! auto-wiring so the new memory is immediately retrievable.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use jiff::Timestamp;
use rmcp::model::{Tool, ToolAnnotations};
use serde::{Deserialize, Serialize};

use singularmem_core::{hook::MultiHook, IndexHook, ItemId, NewItem, Store};
use singularmem_search::{Embedder, EmbedderIndex, FastembedEmbedder, Index};

use crate::{Config, Error, Result};

/// JSON-deserialised arguments for the `memory_ingest` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryIngestArgs {
    /// Memory body text. Non-empty, max 1 MiB.
    pub content: String,
    /// Optional tag labels (non-empty strings, max 64 bytes each, deduplicated).
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Optional provenance label. Max 256 bytes.
    #[serde(default)]
    pub source: Option<String>,
    /// Optional ULID of an existing memory this one corrects.
    #[serde(default)]
    pub supersedes: Option<String>,
    /// Optional user-defined JSON object.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Handler output: the new memory's ID and timestamp.
#[derive(Debug, Clone)]
pub struct MemoryIngestOutput {
    /// The new memory's stable identifier.
    pub id: ItemId,
    /// Wall-clock timestamp assigned by the store.
    pub created_at: Timestamp,
    /// Formatted text block per the spec.
    pub text: String,
}

/// Build the rmcp tool descriptor for `memory_ingest`. Only registered
/// in `list_tools` when `config.read_only == false`.
///
/// # Panics
///
/// Panics if the hard-coded JSON schema literal is not an object (never happens).
#[must_use]
pub fn tool_descriptor() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "content": {
                "type": "string",
                "description": "Memory body text. Non-empty, max 1 MiB."
            },
            "tags": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional tag labels (non-empty strings, max 64 bytes each, deduplicated)."
            },
            "source": {
                "type": "string",
                "description": "Optional provenance label. Max 256 bytes."
            },
            "supersedes": {
                "type": "string",
                "description": "Optional ULID of an existing memory this one corrects. Must exist in the store."
            },
            "metadata": {
                "type": "object",
                "description": "Optional user-defined JSON object. Soft warning threshold 64 KiB."
            }
        },
        "required": ["content"]
    })
    .as_object()
    .expect("schema is object")
    .clone();
    Tool::new(
        "memory_ingest",
        "Add a new memory to the user's local Singularmem store. Returns the new memory's ID \
         and timestamp. Memories are private to this user.",
        std::sync::Arc::new(schema),
    )
    .annotate(ToolAnnotations::new().read_only(false))
}

/// Handle a `tools/call` for `memory_ingest`.
///
/// # Errors
///
/// - [`Error::ReadOnly`] when `config.read_only` is `true`.
/// - [`Error::InvalidId`] when `args.supersedes` doesn't parse as a ULID.
/// - [`Error::Core`] wrapping [`singularmem_core::Error::Validation`]
///   for empty/oversized content, oversized source, invalid tags, etc.
/// - [`Error::Core`] wrapping [`singularmem_core::Error::SupersedesNotFound`]
///   when the supersedes ID doesn't exist in the store.
/// - [`Error::Core`] for other store I/O failures.
/// - [`Error::Search`] when an index hook fails to open (rare; logged).
pub fn handle_memory_ingest(args: MemoryIngestArgs, config: &Config) -> Result<MemoryIngestOutput> {
    if config.read_only {
        return Err(Error::ReadOnly);
    }

    let supersedes = args
        .supersedes
        .as_deref()
        .map(ItemId::from_str)
        .transpose()
        .map_err(|e| Error::InvalidId(e.to_string()))?;

    let mut item = NewItem::text(args.content);
    item.tags = args.tags.unwrap_or_default();
    item.source = args.source;
    item.supersedes = supersedes;
    if let Some(meta) = args.metadata {
        item.metadata = meta;
    }

    let store = open_store_with_hooks(&config.store_path)?;
    let stored = store.ingest(item)?;

    tracing::info!(id = %stored.id, "memory ingested via MCP");

    let text = format!("Ingested memory {} at {}\n", stored.id, stored.created_at);

    Ok(MemoryIngestOutput {
        id: stored.id,
        created_at: stored.created_at,
        text,
    })
}

/// Open the store with index hooks auto-wired.
///
/// **Intentional duplication with the root binary's `open_store()`**
/// (see `src/main.rs`). ~30 lines that mirror Tantivy + `USearch` sidecar
/// detection and embedder selection. A future extraction into a shared
/// "store opener" library is a fine idea once a third consumer arrives;
/// for now, YAGNI says keep the duplication.
fn open_store_with_hooks(store_path: &Path) -> Result<Store> {
    let mut hooks: Vec<Box<dyn IndexHook>> = Vec::new();

    let tantivy_path = derive_index_path(store_path);
    if tantivy_path.exists() {
        match Index::open(&tantivy_path) {
            Ok(idx) => hooks.push(Box::new(idx)),
            Err(e) => tracing::warn!(
                error = %e,
                "tantivy hook open failed; lexical writes for this ingest will be skipped"
            ),
        }
    }

    let vectors_path = derive_vectors_path(store_path);
    if vectors_path.exists() {
        let embedder: Box<dyn Embedder> =
            match std::env::var("SINGULARMEM_TEST_EMBEDDER").ok().as_deref() {
                Some("mock") => Box::new(singularmem_search::testing::MockEmbedder::default()),
                _ => Box::new(FastembedEmbedder::new()?),
            };
        match EmbedderIndex::open(&vectors_path, embedder) {
            Ok(idx) => hooks.push(Box::new(idx)),
            Err(e) => tracing::warn!(
                error = %e,
                "vector hook open failed; semantic writes for this ingest will be skipped"
            ),
        }
    }

    if hooks.is_empty() {
        Ok(Store::open(store_path)?)
    } else {
        Ok(Store::open_with_hook(
            store_path,
            Box::new(MultiHook::new(hooks)),
        )?)
    }
}

fn derive_index_path(store_path: &Path) -> PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".tantivy");
    PathBuf::from(s)
}

fn derive_vectors_path(store_path: &Path) -> PathBuf {
    let mut s = store_path.to_path_buf().into_os_string();
    s.push(".vectors");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_config(read_only: bool) -> (TempDir, Config) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let config = Config::new(store_path, "plain".to_string(), read_only);
        (dir, config)
    }

    #[test]
    fn ingest_succeeds_returns_id_and_timestamp() {
        let (_dir, config) = fresh_config(false);
        let args = MemoryIngestArgs {
            content: "hello world".to_string(),
            tags: None,
            source: None,
            supersedes: None,
            metadata: None,
        };
        let out = handle_memory_ingest(args, &config).expect("ok");
        assert!(
            out.text.contains("Ingested memory "),
            "missing prefix: {}",
            out.text
        );
        assert!(
            out.text.contains(&out.id.to_string()),
            "ID not in text: {}",
            out.text
        );
    }

    #[test]
    fn ingest_empty_content_returns_validation_error() {
        let (_dir, config) = fresh_config(false);
        let args = MemoryIngestArgs {
            content: String::new(),
            tags: None,
            source: None,
            supersedes: None,
            metadata: None,
        };
        let r = handle_memory_ingest(args, &config);
        assert!(
            matches!(
                r,
                Err(Error::Core(singularmem_core::Error::Validation {
                    field: "content",
                    ..
                }))
            ),
            "expected Core(Validation{{field: 'content'}}), got {r:?}"
        );
    }

    #[test]
    fn ingest_with_supersedes_links_to_existing() {
        let (_dir, config) = fresh_config(false);

        // First ingest.
        let first = handle_memory_ingest(
            MemoryIngestArgs {
                content: "first version".to_string(),
                tags: None,
                source: None,
                supersedes: None,
                metadata: None,
            },
            &config,
        )
        .expect("first ok");

        // Second ingest supersedes the first.
        let second = handle_memory_ingest(
            MemoryIngestArgs {
                content: "second version".to_string(),
                tags: None,
                source: None,
                supersedes: Some(first.id.to_string()),
                metadata: None,
            },
            &config,
        )
        .expect("second ok");

        // Verify the link by reading the second item back from the store.
        let store = Store::open(&config.store_path).unwrap();
        let item = store.get(second.id).unwrap();
        assert_eq!(item.supersedes, Some(first.id));
    }

    #[test]
    fn ingest_with_unknown_supersedes_returns_error() {
        let (_dir, config) = fresh_config(false);
        let args = MemoryIngestArgs {
            content: "orphan".to_string(),
            tags: None,
            source: None,
            supersedes: Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string()),
            metadata: None,
        };
        let r = handle_memory_ingest(args, &config);
        assert!(
            matches!(
                r,
                Err(Error::Core(
                    singularmem_core::Error::SupersedesNotFound { .. }
                ))
            ),
            "expected Core(SupersedesNotFound), got {r:?}"
        );
    }

    #[test]
    fn ingest_with_tags_and_source_persists_them() {
        let (_dir, config) = fresh_config(false);
        let args = MemoryIngestArgs {
            content: "tagged content".to_string(),
            tags: Some(vec!["foo".to_string(), "bar".to_string()]),
            source: Some("test-source".to_string()),
            supersedes: None,
            metadata: None,
        };
        let out = handle_memory_ingest(args, &config).expect("ok");

        let store = Store::open(&config.store_path).unwrap();
        let item = store.get(out.id).unwrap();
        assert_eq!(item.source, Some("test-source".to_string()));
        // Tags get sorted/deduped by Store::ingest's validation.
        let mut got_tags = item.tags;
        got_tags.sort();
        assert_eq!(got_tags, vec!["bar".to_string(), "foo".to_string()]);
    }

    #[test]
    fn ingest_rejected_in_read_only_mode() {
        let (_dir, config) = fresh_config(true);
        let args = MemoryIngestArgs {
            content: "should be rejected".to_string(),
            tags: None,
            source: None,
            supersedes: None,
            metadata: None,
        };
        let r = handle_memory_ingest(args, &config);
        assert!(
            matches!(r, Err(Error::ReadOnly)),
            "expected ReadOnly, got {r:?}"
        );
    }
}
