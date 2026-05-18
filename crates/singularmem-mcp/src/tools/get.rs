//! `memory_get` tool — fetch a single memory by ID.

use std::fmt::Write as _;
use std::str::FromStr;

use rmcp::model::{Tool, ToolAnnotations};
use serde::{Deserialize, Serialize};

use singularmem_core::ItemId;

use crate::tools::util::open_store_for_reading;
use crate::{Config, Error, Result};

/// JSON-deserialised arguments for the `memory_get` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryGetArgs {
    /// `ULID` of the memory to fetch (26 characters, Crockford base32).
    pub id: String,
}

/// Handler output: a single text block with the memory's content +
/// metadata. The MCP transport layer wraps this in a `CallToolResult`.
#[derive(Debug, Clone)]
pub struct MemoryGetOutput {
    /// Formatted text block per the spec.
    pub text: String,
}

/// Build the rmcp tool descriptor for `memory_get`. Wired into
/// `ServerHandler::list_tools` in `src/server.rs`.
///
/// # Panics
///
/// Panics if the hard-coded JSON schema literal is not an object (never happens).
#[must_use]
pub fn tool_descriptor() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "ULID of the memory to fetch (26 characters, Crockford base32)."
            }
        },
        "required": ["id"]
    });
    Tool::new(
        "memory_get",
        "Fetch a single memory by ID. Returns the memory's content and metadata as text.",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Handle a `tools/call` for `memory_get`.
///
/// # Errors
///
/// - [`Error::InvalidId`] when `args.id` doesn't parse as a `ULID`.
/// - [`Error::Core`] wrapping [`singularmem_core::Error::NotFound`]
///   when no item with that ID exists.
/// - [`Error::Core`] for other store I/O failures.
pub fn handle_memory_get(args: &MemoryGetArgs, config: &Config) -> Result<MemoryGetOutput> {
    let id = ItemId::from_str(&args.id).map_err(|e| Error::InvalidId(e.to_string()))?;
    let store = open_store_for_reading(config)?;
    let item = store.get(id)?;

    let mut text = String::new();
    writeln!(text, "Memory {}", item.id).expect("write to String is infallible");
    writeln!(text, "Created: {}", item.created_at).expect("write to String is infallible");
    if let Some(source) = &item.source {
        writeln!(text, "Source: {source}").expect("write to String is infallible");
    }
    if !item.tags.is_empty() {
        writeln!(text, "Tags: {}", item.tags.join(", ")).expect("write to String is infallible");
    }
    text.push('\n');
    text.push_str(&item.content);

    Ok(MemoryGetOutput { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::{NewItem, Store};
    use tempfile::TempDir;

    #[allow(clippy::missing_panics_doc)]
    fn seeded(default_adapter: &str, read_only: bool) -> (TempDir, Config, ItemId) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let store = Store::open(&store_path).unwrap();
        let mut item = NewItem::text("the quick brown fox jumps over the lazy dog");
        item.tags = vec!["fox".to_string(), "animals".to_string()];
        item.source = Some("test-source".to_string());
        let stored = store.ingest(item).unwrap();
        drop(store);
        let config = Config::new(store_path, default_adapter.to_string(), read_only);
        (dir, config, stored.id)
    }

    #[test]
    fn get_returns_full_item() {
        let (_dir, config, id) = seeded("plain", false);
        let args = MemoryGetArgs { id: id.to_string() };
        let out = handle_memory_get(&args, &config).expect("ok");
        assert!(
            out.text.contains(&format!("Memory {id}")),
            "missing ID header: {}",
            out.text
        );
        assert!(
            out.text.contains("the quick brown fox"),
            "missing content: {}",
            out.text
        );
        assert!(
            out.text.contains("Source: test-source"),
            "missing source: {}",
            out.text
        );
        assert!(
            out.text.contains("Tags:") && out.text.contains("fox") && out.text.contains("animals"),
            "missing tags: {}",
            out.text
        );
    }

    #[test]
    fn get_invalid_ulid_returns_error() {
        let (_dir, config, _) = seeded("plain", false);
        let args = MemoryGetArgs {
            id: "not-a-ulid".to_string(),
        };
        let r = handle_memory_get(&args, &config);
        assert!(
            matches!(r, Err(Error::InvalidId(_))),
            "expected InvalidId, got {r:?}"
        );
    }

    #[test]
    fn get_not_found_returns_error() {
        let (_dir, config, _) = seeded("plain", false);
        // Valid ULID format but doesn't exist in the store.
        let args = MemoryGetArgs {
            id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
        };
        let r = handle_memory_get(&args, &config);
        assert!(
            matches!(
                r,
                Err(Error::Core(singularmem_core::Error::NotFound { .. }))
            ),
            "expected Core(NotFound), got {r:?}"
        );
    }
}
