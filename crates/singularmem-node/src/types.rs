//! TypeScript-facing object types and their conversions from core types.
//!
//! Only `Item` is exposed in 5a; `NewItem` is deferred to 5c.

/// Lowercase string name for a search/retrieve score kind. Stable across
/// the JS API; matches the spec's TS `ScoreKind` type union.
pub fn score_kind_to_str(k: singularmem_search::ScoreKind) -> String {
    match k {
        singularmem_search::ScoreKind::Rrf => "rrf".to_string(),
        singularmem_search::ScoreKind::Bm25 => "bm25".to_string(),
        singularmem_search::ScoreKind::Cosine => "cosine".to_string(),
    }
}

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

/// One result from `Store.search`. The full `Item` is always populated.
#[napi(object)]
pub struct SearchHit {
    /// The matched item.
    pub item: Item,
    /// Final score after fusion (RRF) or single-ranker (BM25 / cosine).
    pub score: f64,
    /// Which ranker produced the score: "rrf" | "bm25" | "cosine".
    pub kind: String,
    /// 1-based rank in the lexical (Tantivy) ranker, present only when
    /// the lexical ranker ran (hybrid + lexical modes).
    pub lexical_rank: Option<u32>,
    /// 1-based rank in the semantic (`USearch`) ranker, present only when
    /// the semantic ranker ran (hybrid + semantic modes).
    pub semantic_rank: Option<u32>,
}

impl SearchHit {
    /// Construct from a `HybridHit` + the item it points at (caller fetched).
    /// `f32 → f64` widening is lossless. `usize → u32` truncates with the
    /// allow attribute; rank values realistically fit in u32 (typically <100).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn from_parts(hit: singularmem_search::HybridHit, item: singularmem_core::Item) -> Self {
        Self {
            item: item.into(),
            score: f64::from(hit.score),
            kind: score_kind_to_str(hit.score_kind),
            lexical_rank: hit.lexical_rank.map(|n| n as u32),
            semantic_rank: hit.semantic_rank.map(|n| n as u32),
        }
    }
}

/// Results returned by `Store.search`.
#[napi(object)]
pub struct SearchResults {
    /// The query string echoed back from the search call.
    pub query: String,
    /// Ranked list of search hits, sorted by descending score. May be empty
    /// if no items matched the query.
    pub hits: Vec<SearchHit>,
}

/// One block in a `RetrievedContext`. Flat shape matching
/// `singularmem_retrieve::MemoryBlock` (no nested Item).
#[napi(object)]
pub struct MemoryBlock {
    /// 26-character Crockford base32 ULID.
    pub id: String,
    /// Full UTF-8 content from the store (not a snippet).
    pub content: String,
    /// Score whose meaning depends on `kind`.
    pub score: f64,
    /// "rrf" | "bm25" | "cosine".
    pub kind: String,
    /// Free-form provenance label from the matched item.
    pub source: Option<String>,
    /// Tags from the matched item. Sorted, deduplicated.
    pub tags: Vec<String>,
    /// Wall-clock time the matched item was ingested, as a JS `Date`.
    ///
    /// **Precision caveat:** the core layer stores timestamps at nanosecond
    /// precision. Any sub-millisecond component is silently truncated when
    /// crossing the native boundary (same behaviour as `Item.createdAt`).
    #[napi(ts_type = "Date")]
    pub created_at: f64,
}

impl From<singularmem_retrieve::MemoryBlock> for MemoryBlock {
    #[allow(clippy::cast_precision_loss)]
    fn from(b: singularmem_retrieve::MemoryBlock) -> Self {
        Self {
            id: b.id.to_string(),
            content: b.content,
            score: f64::from(b.score),
            kind: score_kind_to_str(b.score_kind),
            source: b.source,
            tags: b.tags,
            created_at: b.created_at.as_millisecond() as f64,
        }
    }
}

/// Structured retrieval context returned by `Store.retrieve`.
#[napi(object)]
pub struct RetrievedContext {
    /// The query string echoed back from the retrieve call.
    pub query: String,
    /// Ordered list of memory blocks, sorted by descending score and filtered
    /// by `minScore`. Pass this directly to an adapter's `format()` method.
    pub blocks: Vec<MemoryBlock>,
}

impl From<singularmem_retrieve::RetrievedContext> for RetrievedContext {
    fn from(ctx: singularmem_retrieve::RetrievedContext) -> Self {
        Self {
            query: ctx.query,
            blocks: ctx.blocks.into_iter().map(Into::into).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use singularmem_core::item::ItemId;
    use std::str::FromStr;

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
        assert_eq!(
            item.supersedes,
            Some("01HXCCCCCCCCCCCCCCCCCCCCC0".to_string())
        );
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

    #[test]
    fn score_kind_rrf_lowercase() {
        assert_eq!(score_kind_to_str(singularmem_search::ScoreKind::Rrf), "rrf");
    }

    #[test]
    fn score_kind_bm25_lowercase() {
        assert_eq!(
            score_kind_to_str(singularmem_search::ScoreKind::Bm25),
            "bm25"
        );
    }

    #[test]
    fn score_kind_cosine_lowercase() {
        assert_eq!(
            score_kind_to_str(singularmem_search::ScoreKind::Cosine),
            "cosine"
        );
    }

    #[test]
    fn search_hit_passes_ranks_when_hybrid() {
        let id =
            singularmem_core::item::ItemId::from_str("01HXAAAAAAAAAAAAAAAAAAAAA0").unwrap();
        let hit = singularmem_search::HybridHit {
            id,
            score: 0.5_f32,
            score_kind: singularmem_search::ScoreKind::Rrf,
            lexical_rank: Some(1),
            semantic_rank: Some(2),
            snippet: None,
        };
        let sh = SearchHit::from_parts(hit, sample_core_item());
        assert_eq!(sh.lexical_rank, Some(1));
        assert_eq!(sh.semantic_rank, Some(2));
        assert_eq!(sh.kind, "rrf");
    }

    #[test]
    fn search_hit_omits_ranks_for_single_ranker_lexical() {
        let id =
            singularmem_core::item::ItemId::from_str("01HXAAAAAAAAAAAAAAAAAAAAA0").unwrap();
        let hit = singularmem_search::HybridHit {
            id,
            score: 0.5_f32,
            score_kind: singularmem_search::ScoreKind::Bm25,
            lexical_rank: Some(1),
            semantic_rank: None,
            snippet: None,
        };
        let sh = SearchHit::from_parts(hit, sample_core_item());
        assert_eq!(sh.lexical_rank, Some(1));
        assert!(sh.semantic_rank.is_none());
        assert_eq!(sh.kind, "bm25");
    }

    #[test]
    fn retrieved_context_round_trips_empty() {
        let core_ctx = singularmem_retrieve::RetrievedContext {
            query: "hello".to_string(),
            blocks: vec![],
            elapsed: std::time::Duration::ZERO,
            total_considered: 0,
        };
        let napi_ctx: RetrievedContext = core_ctx.into();
        assert_eq!(napi_ctx.query, "hello");
        assert!(napi_ctx.blocks.is_empty());
    }
}
