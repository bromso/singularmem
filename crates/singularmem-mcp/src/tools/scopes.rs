//! `memory_scopes` tool — list every scope with its item count.
use std::fmt::Write as _;

use rmcp::model::{Tool, ToolAnnotations};

use crate::tools::util::open_store_for_reading;
use crate::{Config, Result};

/// Handler output: one `path<TAB>count` line per scope.
#[derive(Debug, Clone)]
pub struct MemoryScopesOutput {
    /// Formatted listing text.
    pub text: String,
}

/// Build the rmcp tool descriptor for `memory_scopes`.
///
/// # Panics
/// Never: the schema literal is an object.
#[must_use]
pub fn tool_descriptor() -> Tool {
    let schema = serde_json::json!({ "type": "object", "properties": {}, "required": [] });
    Tool::new(
        "memory_scopes",
        "List every scope path in the store with its item count, sorted by path. \
         Use a returned path as the `scope` argument of memory_list or memory_retrieve.",
        schema.as_object().expect("schema is object").clone(),
    )
    .annotate(ToolAnnotations::new().read_only(true))
}

/// Handle a `tools/call` for `memory_scopes`.
///
/// # Errors
/// [`crate::Error::Core`] on store I/O failure.
pub fn handle_memory_scopes(config: &Config) -> Result<MemoryScopesOutput> {
    let store = open_store_for_reading(config)?;
    let scopes = store.scopes()?;
    let mut text = String::new();
    if scopes.is_empty() {
        text.push_str("No scopes (all items are unscoped).");
    } else {
        for (path, count) in scopes {
            writeln!(text, "{path}\t{count}").expect("write to String is infallible");
        }
    }
    Ok(MemoryScopesOutput { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::{NewItem, Store};
    use tempfile::TempDir;

    #[allow(clippy::missing_panics_doc)]
    fn seeded(scopes: &[Option<&str>]) -> (TempDir, Config) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let store = Store::open(&store_path).unwrap();
        for (i, scope) in scopes.iter().enumerate() {
            let mut item = NewItem::text(format!("seed memory number {i}"));
            item.scope = scope.map(str::to_string);
            store.ingest(item).unwrap();
        }
        drop(store);
        let config = Config::new(store_path, "plain".to_string(), false);
        (dir, config)
    }

    #[test]
    fn scopes_lists_paths_and_counts() {
        let (_dir, config) = seeded(&[Some("a"), Some("a"), Some("b"), None]);
        let out = handle_memory_scopes(&config).expect("ok");
        assert!(
            out.text.contains("a\t2"),
            "expected 'a\\t2' in output: {}",
            out.text
        );
        assert!(
            out.text.contains("b\t1"),
            "expected 'b\\t1' in output: {}",
            out.text
        );
    }

    #[test]
    fn scopes_empty_store_reports_no_scopes() {
        let (_dir, config) = seeded(&[]);
        let out = handle_memory_scopes(&config).expect("ok");
        assert!(
            out.text.contains("No scopes"),
            "expected 'No scopes' for empty store: {}",
            out.text
        );
    }
}
