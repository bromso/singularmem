//! TypeScript-facing object types and their conversions from core types.
//!
//! Only `Item` is exposed in 5a; `NewItem` is deferred to 5c.

/// An item retrieved from the store.
///
/// All string values are UTF-8. `createdAt` is a JS `Date` constructed from
/// the millisecond-precision wall-clock time the store assigned at ingest.
///
/// **Precision caveat:** the core layer stores timestamps at nanosecond
/// precision, but the JS `Date` type only supports millisecond precision.
/// Any sub-millisecond component of `createdAt` is silently truncated when
/// crossing the native boundary.
#[napi(object)]
pub struct Item {
    /// Unique item identifier: a 26-character Crockford base32 ULID string.
    ///
    /// ULIDs are lexicographically sortable by creation time. The string is
    /// always uppercase and exactly 26 characters long.
    pub id: String,
    /// The item's main text payload, encoded as UTF-8.
    pub content: String,
    /// Wall-clock time the store assigned at ingest, as a JS `Date`.
    ///
    /// **Precision caveat:** the underlying store records nanosecond precision;
    /// sub-millisecond digits are lost when the value crosses the native
    /// boundary and is represented as a JS `Date` (millisecond precision only).
    #[napi(ts_type = "Date")]
    pub created_at: f64,
    /// ULID of the item this item supersedes, or `undefined` if this item does
    /// not replace a prior one.
    ///
    /// Following the chain of `supersedes` links from newest to oldest
    /// reconstructs the full revision history of a logical memory entry.
    pub supersedes: Option<String>,
    /// Tags attached to the item.
    ///
    /// The array is always sorted lexicographically and deduplicated; no tag
    /// appears more than once.
    pub tags: Vec<String>,
    /// Optional free-form provenance label identifying the source of the item
    /// (e.g. `"user"`, `"llm"`, `"import"`). `undefined` if not set.
    pub source: Option<String>,
    /// Arbitrary user-defined JSON object attached to the item.
    ///
    /// The value is always an object (never `null`, an array, or a scalar).
    /// Defaults to an empty object `{}` when no metadata was provided at
    /// ingest time.
    pub metadata: serde_json::Value,
}

impl From<singularmem_core::Item> for Item {
    fn from(core: singularmem_core::Item) -> Self {
        Self {
            id: core.id.to_string(),
            content: core.content,
            #[allow(clippy::cast_precision_loss)]
            created_at: core.created_at.as_millisecond() as f64,
            supersedes: core.supersedes.map(|id| id.to_string()),
            tags: core.tags,
            source: core.source,
            metadata: core.metadata,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use singularmem_core::item::ItemId;

    fn sample_core_item() -> singularmem_core::Item {
        singularmem_core::Item {
            id: ItemId::from_str("01HXAAAAAAAAAAAAAAAAAAAAA0").unwrap(),
            content: "hello".to_string(),
            created_at: jiff::Timestamp::from_millisecond(1_700_000_000_000).unwrap(),
            supersedes: None,
            tags: vec!["a".to_string(), "b".to_string()],
            source: Some("test".to_string()),
            metadata: serde_json::json!({"k": "v"}),
        }
    }

    #[test]
    fn item_id_serializes_as_string() {
        let item: Item = sample_core_item().into();
        assert_eq!(item.id, "01HXAAAAAAAAAAAAAAAAAAAAA0");
    }

    #[test]
    fn item_content_round_trips() {
        let item: Item = sample_core_item().into();
        assert_eq!(item.content, "hello");
    }

    #[test]
    fn item_created_at_is_ms_since_epoch() {
        let item: Item = sample_core_item().into();
        #[allow(clippy::cast_possible_truncation)]
        let ms = item.created_at as i64;
        assert_eq!(ms, 1_700_000_000_000);
    }

    #[test]
    fn item_supersedes_none_becomes_none() {
        let item: Item = sample_core_item().into();
        assert!(item.supersedes.is_none());
    }

    #[test]
    fn item_tags_preserved() {
        let item: Item = sample_core_item().into();
        assert_eq!(item.tags, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn item_metadata_preserved() {
        let item: Item = sample_core_item().into();
        assert_eq!(item.metadata, serde_json::json!({"k": "v"}));
    }

    #[test]
    fn item_supersedes_some_round_trips() {
        let core = singularmem_core::Item {
            supersedes: Some(ItemId::from_str("01HXCCCCCCCCCCCCCCCCCCCCC0").unwrap()),
            ..sample_core_item()
        };
        let item: Item = core.into();
        assert_eq!(item.supersedes, Some("01HXCCCCCCCCCCCCCCCCCCCCC0".to_string()));
    }

    #[test]
    fn item_source_none_round_trips() {
        let core = singularmem_core::Item {
            source: None,
            ..sample_core_item()
        };
        let item: Item = core.into();
        assert!(item.source.is_none());
    }
}
