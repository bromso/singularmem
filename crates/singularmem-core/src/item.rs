//! Memory item types: `ItemId`, `Item`, `NewItem`.
//!
//! `Item` is the persisted form (immutable, has an assigned ID and timestamp).
//! `NewItem` is the to-be-ingested form — the type system prevents callers
//! from setting an ID that the store does not control.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Stable, opaque identifier for a memory item.
///
/// Implemented as a [ULID](https://github.com/ulid/spec): 26 characters of
/// Crockford base32, time-sortable, URL-safe.
///
/// # Display and parsing
///
/// `Display` always emits uppercase. `FromStr` accepts either case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemId(Ulid);

impl ItemId {
    /// Wrap a raw `Ulid`. Crate-internal — public API uses `ingest` to mint IDs.
    #[must_use]
    #[allow(dead_code)]
    pub(crate) const fn from_ulid(u: Ulid) -> Self {
        Self(u)
    }

    /// Underlying ULID. Useful for callers that want to inspect the time
    /// component or convert to bytes.
    #[must_use]
    pub const fn as_ulid(&self) -> Ulid {
        self.0
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // ulid::Ulid::Display emits uppercase by default.
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for ItemId {
    type Err = ulid::DecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // ulid::Ulid::from_string accepts case-insensitive Crockford base32.
        Ulid::from_string(s).map(Self)
    }
}

/// A persisted memory item. Immutable once stored.
///
/// All fields are public; this is a data record, not a behaviour-bearing type.
/// SDK consumers may read every field directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// Stable, opaque identifier minted by the store at ingest time.
    pub id: ItemId,
    /// UTF-8 text content of the item. Non-empty, ≤ 1 MiB.
    pub content: String,
    /// Wall-clock timestamp the store assigned at ingest, RFC 3339 nanos.
    pub created_at: jiff::Timestamp,
    /// Pointer to the prior item this one corrects (the supersedes chain).
    /// `None` for items with no prior version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<ItemId>,
    /// Tags attached to the item. Sorted, deduplicated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Free-form provenance label (e.g. `"claude-conversation:abc-123"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Arbitrary user-defined metadata. Always a JSON object (possibly empty).
    #[serde(default = "default_metadata", skip_serializing_if = "is_empty_object")]
    pub metadata: serde_json::Value,
}

/// The "to be ingested" form of an item. The store assigns `id` and
/// `created_at`; callers cannot override them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewItem {
    /// UTF-8 text content. Non-empty, ≤ 1 MiB.
    pub content: String,
    /// Optional pointer to the prior item this corrects.
    pub supersedes: Option<ItemId>,
    /// Tags to attach. Order does not matter; duplicates are silently deduped.
    pub tags: Vec<String>,
    /// Optional free-form provenance label, ≤ 256 bytes.
    pub source: Option<String>,
    /// Arbitrary user-defined JSON object. Defaults to `{}`.
    pub metadata: serde_json::Value,
}

impl NewItem {
    /// Convenience: a `NewItem` with just text content and default everything else.
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            supersedes: None,
            tags: Vec::new(),
            source: None,
            metadata: default_metadata(),
        }
    }
}

fn default_metadata() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

fn is_empty_object(v: &serde_json::Value) -> bool {
    matches!(v, serde_json::Value::Object(m) if m.is_empty())
}

/// Maximum content length in bytes (1 MiB). Enforced by both the lib and the
/// `items.content` SQL `CHECK` constraint.
#[allow(dead_code)]
pub(crate) const MAX_CONTENT_BYTES: usize = 1_048_576;

/// Maximum tag length in bytes.
#[allow(dead_code)]
pub(crate) const MAX_TAG_BYTES: usize = 64;

/// Maximum source length in bytes.
#[allow(dead_code)]
pub(crate) const MAX_SOURCE_BYTES: usize = 256;

/// Soft warning threshold for metadata size — emits a `tracing::warn!` if a
/// single item's metadata exceeds this. Ingest still succeeds.
#[allow(dead_code)]
pub(crate) const METADATA_WARN_BYTES: usize = 65_536;

/// Validate a `NewItem`. Returns the normalised tag list (deduped, sorted) on
/// success. Returns `Error::Validation` with the field name and a reason on
/// failure. Does not touch the store.
#[allow(dead_code)]
pub(crate) fn validate(item: &NewItem) -> crate::Result<Vec<String>> {
    use crate::Error;

    if item.content.is_empty() {
        return Err(Error::Validation {
            field: "content",
            reason: "must be non-empty".to_string(),
        });
    }
    if item.content.len() > MAX_CONTENT_BYTES {
        return Err(Error::Validation {
            field: "content",
            reason: format!(
                "exceeds {MAX_CONTENT_BYTES}-byte cap (got {} bytes)",
                item.content.len()
            ),
        });
    }

    if let Some(src) = &item.source {
        if src.len() > MAX_SOURCE_BYTES {
            return Err(Error::Validation {
                field: "source",
                reason: format!(
                    "exceeds {MAX_SOURCE_BYTES}-byte cap (got {} bytes)",
                    src.len()
                ),
            });
        }
    }

    if !matches!(item.metadata, serde_json::Value::Object(_)) {
        return Err(Error::Validation {
            field: "metadata",
            reason: format!(
                "must be a JSON object (got {})",
                json_type_name(&item.metadata)
            ),
        });
    }
    let metadata_bytes = serde_json::to_vec(&item.metadata)
        .map(|v| v.len())
        .unwrap_or(0);
    if metadata_bytes > METADATA_WARN_BYTES {
        tracing::warn!(
            target: "singularmem_core::ingest",
            metadata_bytes,
            threshold = METADATA_WARN_BYTES,
            "ingest item carries unusually large metadata payload"
        );
    }

    let mut normalised = Vec::with_capacity(item.tags.len());
    for tag in &item.tags {
        if tag.is_empty() {
            return Err(Error::Validation {
                field: "tags",
                reason: "tag must be non-empty".to_string(),
            });
        }
        if tag.len() > MAX_TAG_BYTES {
            return Err(Error::Validation {
                field: "tags",
                reason: format!(
                    "tag exceeds {MAX_TAG_BYTES}-byte cap (got {} bytes)",
                    tag.len()
                ),
            });
        }
        if tag.contains('\0') {
            return Err(Error::Validation {
                field: "tags",
                reason: "tag must not contain NUL bytes".to_string(),
            });
        }
        normalised.push(tag.clone());
    }
    normalised.sort();
    normalised.dedup();

    Ok(normalised)
}

#[allow(dead_code)]
const fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn empty_content_rejected() {
        let item = NewItem::text("");
        assert!(matches!(
            validate(&item),
            Err(Error::Validation {
                field: "content",
                ..
            })
        ));
    }

    #[test]
    fn oversized_content_rejected() {
        let item = NewItem::text("x".repeat(MAX_CONTENT_BYTES + 1));
        assert!(matches!(
            validate(&item),
            Err(Error::Validation {
                field: "content",
                ..
            })
        ));
    }

    #[test]
    fn long_source_rejected() {
        let mut item = NewItem::text("hello");
        item.source = Some("s".repeat(MAX_SOURCE_BYTES + 1));
        assert!(matches!(
            validate(&item),
            Err(Error::Validation {
                field: "source",
                ..
            })
        ));
    }

    #[test]
    fn metadata_must_be_object() {
        let mut item = NewItem::text("hello");
        item.metadata = serde_json::json!([1, 2, 3]);
        assert!(matches!(
            validate(&item),
            Err(Error::Validation {
                field: "metadata",
                ..
            })
        ));
    }

    #[test]
    fn duplicate_tags_dedup() {
        let mut item = NewItem::text("hello");
        item.tags = vec!["a".into(), "a".into(), "b".into(), "a".into()];
        let normalised = validate(&item).expect("valid");
        assert_eq!(normalised, vec!["a", "b"]);
    }

    #[test]
    fn empty_tag_rejected() {
        let mut item = NewItem::text("hello");
        item.tags = vec!["valid".into(), String::new(), "another".into()];
        assert!(matches!(
            validate(&item),
            Err(Error::Validation { field: "tags", .. })
        ));
    }

    #[test]
    fn null_byte_in_tag_rejected() {
        let mut item = NewItem::text("hello");
        item.tags = vec!["nul\0byte".into()];
        assert!(matches!(
            validate(&item),
            Err(Error::Validation { field: "tags", .. })
        ));
    }

    #[test]
    fn happy_path_validates() {
        let mut item = NewItem::text("hello");
        item.tags = vec!["foo".into(), "bar".into()];
        item.source = Some("test".into());
        item.metadata = serde_json::json!({"k": "v"});
        let normalised = validate(&item).expect("valid");
        assert_eq!(normalised, vec!["bar", "foo"]); // sorted
    }
}
