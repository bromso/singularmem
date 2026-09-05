//! MCP resources: `singularmem://memory/{id}` read-only views of items.
//!
//! The server enumerates no resources (`resources/list` stays empty by
//! design — a memory store is not a browsing experience); it only
//! advertises the one template and serves reads against it. See
//! `docs/superpowers/specs/2026-09-05-mcp-surface-16-design.md` §
//! "Resources".

use std::str::FromStr;

use rmcp::model::{
    Annotated, RawResourceTemplate, ReadResourceResult, ResourceContents, ResourceTemplate,
};
use singularmem_core::ItemId;

use crate::tools::util::open_store_for_reading;
use crate::Config;

/// URI prefix every valid resource URI starts with; the remainder is the
/// item's ULID.
pub const URI_PREFIX: &str = "singularmem://memory/";

/// Errors from reading a `singularmem://memory/{id}` resource.
#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    /// The URI is not `singularmem://memory/<ulid>` of an item that exists
    /// in the store (wrong scheme, wrong path, malformed ULID, or unknown
    /// ULID all collapse to this — the wire contract does not distinguish
    /// them, matching `resource_not_found`'s single error code).
    #[error("resource not found: {0}")]
    NotFound(String),
    /// Store I/O failure while reading an otherwise well-formed URI.
    #[error(transparent)]
    Other(#[from] crate::Error),
}

/// The `singularmem://memory/{id}` resource template advertised by
/// `resources/templates/list`.
#[must_use]
pub fn template() -> ResourceTemplate {
    Annotated::new(
        RawResourceTemplate {
            uri_template: "singularmem://memory/{id}".into(),
            name: "memory".into(),
            title: None,
            description: Some("A single memory by ULID".into()),
            mime_type: Some("text/plain".into()),
            icons: None,
        },
        None,
    )
}

/// Read one memory by URI. Any URI that is not `singularmem://memory/<ulid>`
/// of an item that exists in the store is [`ResourceError::NotFound`].
///
/// # Errors
///
/// - [`ResourceError::NotFound`] for a wrong scheme, a wrong resource path,
///   a malformed ULID, or a well-formed ULID with no matching item.
/// - [`ResourceError::Other`] for store I/O failures.
pub fn read(config: &Config, uri: &str) -> Result<ReadResourceResult, ResourceError> {
    let raw_id = uri
        .strip_prefix(URI_PREFIX)
        .ok_or_else(|| ResourceError::NotFound(uri.to_string()))?;
    let id = ItemId::from_str(raw_id).map_err(|_| ResourceError::NotFound(uri.to_string()))?;
    let store = open_store_for_reading(config)?;
    let item = match store.get(id) {
        Ok(item) => item,
        Err(singularmem_core::Error::NotFound { .. }) => {
            return Err(ResourceError::NotFound(uri.to_string()));
        }
        Err(e) => return Err(ResourceError::Other(crate::Error::Core(e))),
    };

    let dash = |s: Option<&str>| s.map_or_else(|| "-".to_string(), str::to_string);
    let tags = if item.tags.is_empty() {
        "-".to_string()
    } else {
        item.tags.join(", ")
    };
    let text = format!(
        "id: {}\ncreated_at: {}\nscope: {}\nsource: {}\ntags: {}\n\n{}",
        item.id,
        item.created_at,
        dash(item.scope.as_deref()),
        dash(item.source.as_deref()),
        tags,
        item.content
    );
    let contents = ResourceContents::text(text, uri.to_string()).with_mime_type("text/plain");
    Ok(ReadResourceResult::new(vec![contents]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::{NewItem, Store};
    use tempfile::TempDir;

    /// Store with one item; returns the temp root (kept alive), a `Config`
    /// pointing at it, and the item's ID.
    fn seeded_one(content: &str) -> (TempDir, Config, ItemId) {
        let dir = TempDir::new().unwrap();
        let store_path = dir.path().join("store.db");
        let store = Store::open(&store_path).unwrap();
        let stored = store.ingest(NewItem::text(content.to_string())).unwrap();
        drop(store);
        let config = Config::new(store_path, "plain".to_string(), false);
        (dir, config, stored.id)
    }

    #[test]
    fn read_known_item_returns_header_and_content() {
        let (_d, config, id) = seeded_one("hello memory");
        let res = read(&config, &format!("singularmem://memory/{id}")).unwrap();
        let text = match &res.contents[0] {
            ResourceContents::TextResourceContents {
                text,
                mime_type,
                uri,
                ..
            } => {
                assert_eq!(mime_type.as_deref(), Some("text/plain"));
                assert!(uri.ends_with(&id.to_string()));
                text.clone()
            }
            other @ ResourceContents::BlobResourceContents { .. } => {
                panic!("expected text, got {other:?}")
            }
        };
        assert!(
            text.starts_with(&format!("id: {id}\ncreated_at: ")),
            "{text}"
        );
        assert!(
            text.contains("\nscope: -\nsource: -\ntags: -\n\nhello memory"),
            "{text}"
        );
    }

    #[test]
    fn unknown_id_wrong_scheme_and_malformed_id_are_not_found() {
        let (_d, config, _id) = seeded_one("x");
        for uri in [
            "singularmem://memory/01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "singularmem://memory/nope",
            "file:///etc/passwd",
            "singularmem://scope/a",
        ] {
            let err = read(&config, uri).unwrap_err();
            assert!(matches!(err, ResourceError::NotFound(_)), "{uri}: {err:?}");
        }
    }

    #[test]
    fn template_is_the_memory_uri() {
        let t = template();
        assert_eq!(t.raw.uri_template, "singularmem://memory/{id}");
        assert_eq!(t.raw.mime_type.as_deref(), Some("text/plain"));
        assert_eq!(t.raw.name, "memory");
    }

    #[test]
    fn read_only_server_still_serves_resources() {
        let (_d, config, id) = seeded_one("read-only memory");
        let config = Config::new(config.store_path, "plain".into(), true);
        let res = read(&config, &format!("singularmem://memory/{id}")).unwrap();
        assert_eq!(res.contents.len(), 1);
    }
}
