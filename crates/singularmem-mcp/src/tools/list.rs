//! `memory_list` tool — enumerate memories, optionally filtered by tag.

use std::fmt::Write as _;

use rmcp::model::{Tool, ToolAnnotations};
use serde::{Deserialize, Serialize};

use crate::tools::util::open_store_for_reading;
use crate::{Config, Result};

/// JSON-deserialised arguments for the `memory_list` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryListArgs {
    /// AND-filter tags. When present, only items containing every
    /// listed tag are returned.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Maximum number of items to return. Clamped to `[1, 100]`.
    /// Default: 50.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Handler output: a single text block with one line per item.
#[derive(Debug, Clone)]
pub struct MemoryListOutput {
    /// Formatted listing text.
    pub text: String,
}

/// Build the rmcp tool descriptor for `memory_list`.
///
/// # Panics
///
/// Panics if the hard-coded JSON schema literal is not an object (never happens).
#[must_use]
pub fn tool_descriptor() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "tags": {
                "type": "array",
                "items": { "type": "string" },
                "description": "AND-filter tags."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "maximum": 100,
                "default": 50,
                "description": "Maximum number of items to return."
            }
        },
        "required": []
    });
    Tool::new(
        "memory_list",
        "Enumerate memories in the store, optionally filtered by tag (AND-semantics). \
         Returns a compact listing with IDs and content snippets.",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Handle a `tools/call` for `memory_list`.
///
/// # Errors
///
/// Returns [`crate::Error::Core`] for store I/O failures.
pub fn handle_memory_list(args: &MemoryListArgs, config: &Config) -> Result<MemoryListOutput> {
    let limit = args.limit.unwrap_or(50).clamp(1, 100);
    let store = open_store_for_reading(config)?;

    let iter: Box<dyn Iterator<Item = singularmem_core::Result<singularmem_core::Item>>> =
        match &args.tags {
            Some(tags) if !tags.is_empty() => {
                let tag_refs: Vec<&str> = tags.iter().map(String::as_str).collect();
                Box::new(store.list_by_tags(&tag_refs)?)
            }
            _ => Box::new(store.list()?),
        };

    let items: Vec<singularmem_core::Item> = iter
        .take(limit)
        .collect::<singularmem_core::Result<Vec<_>>>()?;

    let count = items.len();
    let mut text = String::new();
    write!(
        text,
        "Found {count} memor{} (limit {limit}):\n\n",
        if count == 1 { "y" } else { "ies" }
    )
    .expect("write to String is infallible");
    for item in &items {
        let snippet: String = item.content.chars().take(80).collect();
        let snippet_one_line = snippet.replace('\n', " ");
        writeln!(text, "{}: {snippet_one_line}", item.id).expect("write to String is infallible");
    }

    Ok(MemoryListOutput { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::{NewItem, Store};
    use tempfile::TempDir;

    #[allow(clippy::missing_panics_doc)]
    fn seeded(n: usize, with_tags: bool) -> (TempDir, Config) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let store = Store::open(&store_path).unwrap();
        for i in 0..n {
            let mut item = NewItem::text(format!("seed memory number {i}"));
            if with_tags {
                item.tags = if i % 2 == 0 {
                    vec!["even".to_string()]
                } else {
                    vec!["odd".to_string()]
                };
            }
            store.ingest(item).unwrap();
        }
        drop(store);
        let config = Config::new(store_path, "plain".to_string(), false);
        (dir, config)
    }

    #[test]
    fn list_returns_all_when_no_filter() {
        let (_dir, config) = seeded(3, false);
        let args = MemoryListArgs {
            tags: None,
            limit: None,
        };
        let out = handle_memory_list(&args, &config).expect("ok");
        assert!(
            out.text.contains("Found 3 memories"),
            "missing count: {}",
            out.text
        );
        assert_eq!(out.text.matches("seed memory number").count(), 3);
    }

    #[test]
    fn list_respects_tag_filter() {
        let (_dir, config) = seeded(6, true);
        let args = MemoryListArgs {
            tags: Some(vec!["even".to_string()]),
            limit: None,
        };
        let out = handle_memory_list(&args, &config).expect("ok");
        assert!(
            out.text.contains("Found 3 memories"),
            "expected 3 even, got: {}",
            out.text
        );
    }

    #[test]
    fn list_caps_limit_at_100() {
        let (_dir, config) = seeded(150, false);
        let args = MemoryListArgs {
            tags: None,
            limit: Some(500),
        };
        let out = handle_memory_list(&args, &config).expect("ok");
        assert!(
            out.text.contains("Found 100"),
            "expected limit clamp to 100, got: {}",
            out.text
        );
    }

    #[test]
    fn list_default_limit_50() {
        let (_dir, config) = seeded(100, false);
        let args = MemoryListArgs {
            tags: None,
            limit: None,
        };
        let out = handle_memory_list(&args, &config).expect("ok");
        assert!(
            out.text.contains("Found 50"),
            "expected default limit 50, got: {}",
            out.text
        );
    }
}
