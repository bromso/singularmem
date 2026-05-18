//! `memory_revisions` tool — walk the supersedes chain newest-first.

use std::fmt::Write as _;
use std::str::FromStr;

use rmcp::model::{Tool, ToolAnnotations};
use serde::{Deserialize, Serialize};

use singularmem_core::ItemId;

use crate::tools::util::open_store_for_reading;
use crate::{Config, Error, Result};

/// JSON-deserialised arguments for the `memory_revisions` tool.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryRevisionsArgs {
    /// `ULID` of any item in the chain.
    pub id: String,
}

/// Handler output: a single text block with header + one line per revision.
#[derive(Debug, Clone)]
pub struct MemoryRevisionsOutput {
    /// Formatted revisions listing text.
    pub text: String,
}

/// Build the rmcp tool descriptor for `memory_revisions`.
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
                "description": "ULID of any item in the chain."
            }
        },
        "required": ["id"]
    });
    Tool::new(
        "memory_revisions",
        "Walk the supersedes chain for a memory, newest-first. Returns each revision in the \
         chain with ID and content snippet.",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Handle a `tools/call` for `memory_revisions`.
///
/// # Errors
///
/// - [`Error::InvalidId`] when `args.id` doesn't parse as a `ULID`.
/// - [`Error::Core`] wrapping [`singularmem_core::Error::NotFound`]
///   when no item with that ID exists.
/// - [`Error::Core`] for other store I/O failures.
pub fn handle_memory_revisions(
    args: &MemoryRevisionsArgs,
    config: &Config,
) -> Result<MemoryRevisionsOutput> {
    let id = ItemId::from_str(&args.id).map_err(|e| Error::InvalidId(e.to_string()))?;
    let store = open_store_for_reading(config)?;
    let history = store.revision_history(id)?;

    let count = history.len();
    let mut text = String::new();
    write!(
        text,
        "Revisions of {} ({count} item{}, newest first):\n\n",
        args.id,
        if count == 1 { "" } else { "s" }
    )
    .expect("write to String is infallible");
    for item in &history {
        let snippet: String = item.content.chars().take(80).collect();
        let snippet_one_line = snippet.replace('\n', " ");
        writeln!(text, "{}: {snippet_one_line}", item.id).expect("write to String is infallible");
    }

    Ok(MemoryRevisionsOutput { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::{NewItem, Store};
    use tempfile::TempDir;

    #[allow(clippy::missing_panics_doc)]
    fn seeded_with_chain(depth: usize) -> (TempDir, Config, ItemId) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let store = Store::open(&store_path).unwrap();
        let mut prev_id: Option<ItemId> = None;
        let mut newest_id: Option<ItemId> = None;
        for i in 0..depth {
            let mut item = NewItem::text(format!("revision {i}"));
            item.supersedes = prev_id;
            let stored = store.ingest(item).unwrap();
            prev_id = Some(stored.id);
            newest_id = Some(stored.id);
        }
        drop(store);
        let config = Config::new(store_path, "plain".to_string(), false);
        (dir, config, newest_id.unwrap())
    }

    #[test]
    fn revisions_walks_chain_newest_first() {
        let (_dir, config, newest) = seeded_with_chain(3);
        let args = MemoryRevisionsArgs {
            id: newest.to_string(),
        };
        let out = handle_memory_revisions(&args, &config).expect("ok");
        assert!(
            out.text.contains("3 items, newest first"),
            "missing header count: {}",
            out.text
        );
        // 3 revision lines + 1 header line + 1 blank line.
        assert_eq!(out.text.matches("revision ").count(), 3);
        // Newest content "revision 2" should appear before "revision 0" in the output.
        let pos_2 = out.text.find("revision 2").expect("revision 2 present");
        let pos_0 = out.text.find("revision 0").expect("revision 0 present");
        assert!(pos_2 < pos_0, "newest should appear first: {}", out.text);
    }

    #[test]
    fn revisions_for_single_item_returns_one() {
        let (_dir, config, only) = seeded_with_chain(1);
        let args = MemoryRevisionsArgs {
            id: only.to_string(),
        };
        let out = handle_memory_revisions(&args, &config).expect("ok");
        assert!(
            out.text.contains("(1 item, newest first)"),
            "expected singular: {}",
            out.text
        );
    }

    #[test]
    fn revisions_invalid_ulid_returns_error() {
        let (_dir, config, _) = seeded_with_chain(1);
        let args = MemoryRevisionsArgs {
            id: "not-a-ulid".to_string(),
        };
        let r = handle_memory_revisions(&args, &config);
        assert!(
            matches!(r, Err(Error::InvalidId(_))),
            "expected InvalidId, got {r:?}"
        );
    }

    #[test]
    fn revisions_not_found_returns_error() {
        let (_dir, config, _) = seeded_with_chain(1);
        let args = MemoryRevisionsArgs {
            id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
        };
        let r = handle_memory_revisions(&args, &config);
        assert!(
            matches!(
                r,
                Err(Error::Core(singularmem_core::Error::NotFound { .. }))
            ),
            "expected Core(NotFound), got {r:?}"
        );
    }
}
