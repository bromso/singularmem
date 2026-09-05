//! TypeScript-facing object types and their conversions from core types.
//!
//! Only `Item` is exposed in 5a; `NewItem` is deferred to 5c.

use singularmem_core::graph::NewObject;

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
    /// Caller-supplied stable identity used for idempotent bulk ingest
    /// (e.g. `"claude-code:<session>:<uuid>"`), unique across the store.
    /// `undefined` for items ingested without one. Read-only: `NewItem`
    /// has no counterpart, so the JS API cannot set it.
    pub external_id: Option<String>,
    /// Hierarchical scope path (e.g. `"team/backend"`), already lowercased
    /// and normalised by the store. `undefined` for unscoped items.
    pub scope: Option<String>,
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
            external_id: core.external_id,
            scope: core.scope,
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
    /// Hierarchical scope path from the matched item, if any. `undefined`
    /// for unscoped items.
    pub scope: Option<String>,
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
            scope: b.scope,
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

/// Input to `Store.ingest`. Only `content` is required; other fields apply
/// sensible defaults when omitted.
#[napi(object)]
pub struct NewItem {
    /// Required: UTF-8 text content. Must be non-empty, ≤ 1 MiB.
    pub content: String,
    /// Optional: ULID of the item this supersedes (revision chain).
    pub supersedes: Option<String>,
    /// Optional: tags to attach. Default: `[]`. Duplicates are silently deduped.
    pub tags: Option<Vec<String>>,
    /// Optional: free-form provenance label, ≤ 256 bytes.
    pub source: Option<String>,
    /// Optional: arbitrary JSON object (must be an object, not an array or
    /// scalar). Default: `{}` when omitted.
    pub metadata: Option<serde_json::Value>,
    /// Optional: hierarchical scope path (e.g. `"Team/X"`). Validated and
    /// normalised (lowercased) by the store at ingest time.
    pub scope: Option<String>,
}

/// One entry in `Store.scopes()`: a distinct scope path and how many items
/// carry it.
#[napi(object)]
pub struct ScopeCount {
    /// Normalised scope path.
    pub path: String,
    /// Number of items whose `scope` equals `path`.
    pub count: u32,
}

/// Convert a JS-sent `NewItem` into the core's `NewItem`. Performs early
/// validation of the `supersedes` ULID format; returns a coded `NapiError`
/// (with `.code === "InvalidId"`) if the string isn't a valid ULID.
/// Empty-string `supersedes: ""` is normalized to `None` (defensive: TS
/// callers might pass empty strings from form fields).
///
/// # Errors
///
/// Returns a `NapiError<&'static str>` with `.code === "InvalidId"` if
/// `supersedes` is set to a malformed ULID string.
#[allow(dead_code)] // used by Store::ingest, added in Task 4
pub fn js_new_item_to_core(
    item: NewItem,
) -> Result<singularmem_core::NewItem, napi::Error<&'static str>> {
    use std::str::FromStr;

    let supersedes = match item.supersedes.as_deref() {
        Some(s) if !s.is_empty() => {
            Some(singularmem_core::item::ItemId::from_str(s).map_err(|e| {
                let core_err = singularmem_core::Error::from(e);
                let node_err = crate::error::NodeError::from(core_err);
                napi::Error::<&'static str>::from(node_err)
            })?)
        }
        _ => None,
    };

    Ok(singularmem_core::NewItem {
        content: item.content,
        supersedes,
        tags: item.tags.unwrap_or_default(),
        source: item.source,
        metadata: item.metadata.unwrap_or_else(|| serde_json::json!({})),
        external_id: None,
        scope: item.scope,
    })
}

// ── Knowledge graph ───────────────────────────────────────────────────────────
//
// Spec: `docs/superpowers/specs/2026-09-05-mcp-surface-16-design.md`
// § "Node binding". Every type here is a flat `#[napi(object)]` mirror of a
// `singularmem_core::graph` type; napi camel-cases the field names in the
// generated TypeScript (`valid_from` → `validFrom`).

/// A reference to a graph entity: its id and display name.
#[napi(object)]
pub struct EntityRef {
    /// 26-character Crockford base32 ULID of the entity.
    pub id: String,
    /// The entity's display name, as first written.
    pub name: String,
}

/// The object side of a fact: another entity, or a literal string value.
///
/// **Exactly one of `entity` / `value` is set**; the other is absent
/// (`undefined`, napi's rendering of `None` — the same convention as
/// `Item.supersedes`).
#[napi(object)]
pub struct FactObject {
    /// Set when the object is another entity.
    pub entity: Option<EntityRef>,
    /// Set when the object is a literal string value.
    pub value: Option<String>,
}

/// One revision of a fact: `subject predicate object`, with a validity
/// window and provenance.
///
/// Timestamps are RFC 3339 UTC strings (jiff `Display`), e.g.
/// `"2026-05-16T00:00:00Z"`. An absent `validFrom` means "since unknown";
/// an absent `validTo` means the fact is still open.
#[napi(object)]
pub struct Fact {
    /// 26-character Crockford base32 ULID of this revision.
    pub id: String,
    /// The entity this fact is about.
    pub subject: EntityRef,
    /// Normalised predicate, e.g. `"works_at"`.
    pub predicate: String,
    /// The fact's object: an entity or a literal value.
    pub object: FactObject,
    /// Start of the validity window; absent for "since unknown".
    pub valid_from: Option<String>,
    /// End of the validity window; absent while the fact is open.
    pub valid_to: Option<String>,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: f64,
    /// ULID of the item this fact was extracted from, if any.
    pub source_item_id: Option<String>,
    /// Scope path this fact was recorded under, if any.
    pub scope: Option<String>,
    /// ULID of the prior revision this one supersedes, if any.
    pub supersedes: Option<String>,
    /// When this revision was recorded (append time).
    pub recorded_at: String,
}

/// Input to `Store.addFact`. Only `subject`, `predicate` and `object` are
/// required.
#[napi(object)]
pub struct NewFact {
    /// The subject entity's display name (normalised at write time).
    pub subject: String,
    /// The predicate (normalised at write time).
    pub predicate: String,
    /// The object: an entity name, or a literal value when `objectIsValue`.
    pub object: String,
    /// When `true`, `object` is stored as a literal value rather than
    /// resolved to an entity. Default `false`.
    pub object_is_value: Option<bool>,
    /// Kind to set on the subject if it is created by this call.
    pub subject_kind: Option<String>,
    /// Kind to set on the object entity if it is created by this call.
    /// Ignored when `objectIsValue` is `true`.
    pub object_kind: Option<String>,
    /// Start of the validity window: `YYYY-MM-DD` or RFC 3339.
    pub valid_from: Option<String>,
    /// End of the validity window: `YYYY-MM-DD` or RFC 3339.
    pub valid_to: Option<String>,
    /// Confidence in `[0.0, 1.0]`. Default `1.0`.
    pub confidence: Option<f64>,
    /// ULID of the item this fact was extracted from.
    pub source_item_id: Option<String>,
    /// Scope path to record the fact under.
    pub scope: Option<String>,
}

/// Filters shared by `Store.queryEntity` and `Store.queryPredicate`.
#[napi(object)]
#[derive(Default)]
pub struct GraphQueryOptions {
    /// Which side of the fact to match: `"outgoing"`, `"incoming"` or
    /// `"both"` (default). Ignored by `queryPredicate`.
    pub direction: Option<String>,
    /// Only facts valid at this instant: `YYYY-MM-DD` or RFC 3339.
    pub as_of: Option<String>,
    /// Only facts believed as of this record time: `YYYY-MM-DD` or RFC 3339.
    pub recorded_at: Option<String>,
    /// Restrict to this scope path.
    pub scope: Option<String>,
    /// When `true`, `scope` matches exactly rather than including
    /// descendants. Default `false`.
    pub scope_exact: Option<bool>,
}

/// Options for `Store.invalidateFact` and `Store.supersedeFact`.
#[napi(object)]
#[derive(Default)]
pub struct FactChangeOptions {
    /// When `true`, the object arguments are literal values rather than
    /// entity names. Applies to *both* objects of `supersedeFact`.
    pub object_is_value: Option<bool>,
    /// Instant the change takes effect: `YYYY-MM-DD` or RFC 3339.
    /// Defaults to now.
    pub at: Option<String>,
    /// Scope the affected fact was recorded under.
    pub scope: Option<String>,
}

/// Scope filter for `Store.timeline` and `Store.graphStats`.
#[napi(object)]
#[derive(Default)]
pub struct GraphScopeOptions {
    /// Restrict to this scope path.
    pub scope: Option<String>,
    /// When `true`, `scope` matches exactly rather than including
    /// descendants. Default `false`.
    pub scope_exact: Option<bool>,
}

/// Options for `Store.entities`.
#[napi(object)]
#[derive(Default)]
pub struct EntityListOptions {
    /// Only entities with this exact `kind`.
    pub kind: Option<String>,
    /// Only entities taking part in at least one fact in this scope.
    pub scope: Option<String>,
    /// When `true`, `scope` matches exactly rather than including
    /// descendants. Default `false`.
    pub scope_exact: Option<bool>,
}

/// One row of `Store.timeline`: a fact head plus whether it is still open.
#[napi(object)]
pub struct TimelineEntry {
    /// The fact revision.
    pub fact: Fact,
    /// `true` when this revision is the current, open head.
    pub current: bool,
}

/// Aggregate counts returned by `Store.graphStats`.
#[napi(object)]
pub struct GraphStats {
    /// Total number of entities.
    pub entities: u32,
    /// Number of open (currently valid) fact heads.
    pub open_facts: u32,
    /// Number of closed fact heads.
    pub closed_facts: u32,
    /// Number of distinct predicates in use.
    pub predicates: u32,
}

/// One row of `Store.entities`: an entity plus its head-fact count.
#[napi(object)]
pub struct EntitySummary {
    /// 26-character Crockford base32 ULID of the entity.
    pub id: String,
    /// The entity's display name.
    pub name: String,
    /// Free-form kind, set when the entity was first created.
    pub kind: Option<String>,
    /// Number of head facts where this entity is subject or object.
    pub fact_count: u32,
}

/// Result of `Store.supersedeFact`.
#[napi(object)]
pub struct SupersedeResult {
    /// The closing revision of the replaced fact; absent when no matching
    /// open fact existed (the new fact is still opened).
    pub closed: Option<Fact>,
    /// The newly opened fact.
    pub opened: Fact,
}

// ── Wake-up ────────────────────────────────────────────────────────────────

/// Options for `Store.wakeup`.
#[napi(object)]
#[derive(Default)]
pub struct WakeupOptions {
    /// Project directory whose default scopes (`claude-code/<b>`,
    /// `codex/<b>`, `cursor/<b>`, and `files/<b>` when `includeFiles`) are
    /// read. Defaults to `process.cwd()` — the binding has no server config
    /// to fall back to.
    pub project: Option<String>,
    /// Also read `files/<basename>` (ingest-dir output). Default `false`.
    pub include_files: Option<bool>,
    /// Most recent items to consider, across all scopes. Default `20`.
    pub limit: Option<u32>,
    /// Output budget in bytes; oldest blocks are dropped first, the header
    /// always survives. Default `8192`.
    pub max_bytes: Option<u32>,
    /// Prompt formatter: `"plain"` (default), `"claude"`, `"openai"` or
    /// `"gemini"`.
    pub adapter: Option<String>,
}

/// Result of `Store.wakeup`: the rendered prompt plus the counts and scopes
/// behind it.
#[napi(object)]
pub struct Wakeup {
    /// The rendered wake-up text: a one-line header followed by the
    /// adapter-formatted blocks, budgeted to `maxBytes`.
    pub text: String,
    /// Items matching the scope set in total (before `limit`).
    pub total: u32,
    /// Blocks actually rendered into `text` (after `limit` and the
    /// `maxBytes` budget).
    pub shown: u32,
    /// The scope paths that were queried, in order.
    pub scopes: Vec<String>,
}

/// Convert a core `EntityRef` into its JS mirror.
fn entity_ref_to_js(r: &singularmem_core::graph::EntityRef) -> EntityRef {
    EntityRef {
        id: r.id.to_string(),
        name: r.name.clone(),
    }
}

/// Convert a core `Fact` into its JS mirror. Timestamps render through
/// jiff's `Display` (RFC 3339, UTC); `confidence` widens `f32 → f64`
/// losslessly.
#[must_use]
pub fn fact_to_js(f: &singularmem_core::graph::Fact) -> Fact {
    use singularmem_core::graph::FactObject as CoreFactObject;
    Fact {
        id: f.id.to_string(),
        subject: entity_ref_to_js(&f.subject),
        predicate: f.predicate.clone(),
        object: match &f.object {
            CoreFactObject::Entity(e) => FactObject {
                entity: Some(entity_ref_to_js(e)),
                value: None,
            },
            CoreFactObject::Value(v) => FactObject {
                entity: None,
                value: Some(v.clone()),
            },
        },
        valid_from: f.valid_from.map(|t| t.to_string()),
        valid_to: f.valid_to.map(|t| t.to_string()),
        confidence: f64::from(f.confidence),
        source_item_id: f.source_item_id.as_ref().map(ToString::to_string),
        scope: f.scope.clone(),
        supersedes: f.supersedes.as_ref().map(ToString::to_string),
        recorded_at: f.recorded_at.to_string(),
    }
}

/// Convert a core `TimelineEntry` into its JS mirror.
#[must_use]
pub fn timeline_entry_to_js(e: &singularmem_core::graph::TimelineEntry) -> TimelineEntry {
    TimelineEntry {
        fact: fact_to_js(&e.fact),
        current: e.current,
    }
}

/// Convert core `GraphStats` into its JS mirror. Counts saturate at
/// `u32::MAX`; a store with four billion facts has other problems.
#[must_use]
pub fn graph_stats_to_js(s: singularmem_core::graph::GraphStats) -> GraphStats {
    GraphStats {
        entities: u32::try_from(s.entities).unwrap_or(u32::MAX),
        open_facts: u32::try_from(s.open_facts).unwrap_or(u32::MAX),
        closed_facts: u32::try_from(s.closed_facts).unwrap_or(u32::MAX),
        predicates: u32::try_from(s.predicates).unwrap_or(u32::MAX),
    }
}

/// Flatten a core `EntitySummary` (which nests the entity) into the flat JS
/// shape the spec's TypeScript declares.
#[must_use]
pub fn entity_summary_to_js(s: &singularmem_core::graph::EntitySummary) -> EntitySummary {
    EntitySummary {
        id: s.entity.id.to_string(),
        name: s.entity.name.clone(),
        kind: s.entity.kind.clone(),
        fact_count: u32::try_from(s.fact_count).unwrap_or(u32::MAX),
    }
}

/// Parse an optional caller-supplied time point (`YYYY-MM-DD` or RFC 3339).
///
/// `singularmem_core::graph::time::parse_point` always reports the field as
/// `"timestamp"`; this re-labels it with the JS option name so the thrown
/// error says which option was wrong (spec § "Errors").
///
/// # Errors
///
/// `NapiError<&'static str>` with `.code === "Validation"` when `raw` is set
/// but parses as neither form.
pub fn parse_time(
    field: &'static str,
    raw: Option<&str>,
) -> Result<Option<jiff::Timestamp>, napi::Error<&'static str>> {
    let Some(s) = raw else { return Ok(None) };
    singularmem_core::graph::time::parse_point(s)
        .map(Some)
        .map_err(|e| {
            let relabelled = match e {
                singularmem_core::Error::Validation { reason, .. } => {
                    singularmem_core::Error::Validation { field, reason }
                }
                other => other,
            };
            crate::error::NodeError::from(relabelled).into()
        })
}

/// Map the JS `direction` string onto the core enum. `None` and `"both"`
/// both mean [`Direction::Both`].
///
/// # Errors
///
/// `NapiError<&'static str>` with `.code === "Validation"` and field
/// `direction` for any other string.
pub fn direction_from(
    raw: Option<&str>,
) -> Result<singularmem_core::graph::Direction, napi::Error<&'static str>> {
    use singularmem_core::graph::Direction;
    match raw {
        None | Some("both") => Ok(Direction::Both),
        Some("outgoing") => Ok(Direction::Outgoing),
        Some("incoming") => Ok(Direction::Incoming),
        Some(other) => {
            let err = singularmem_core::Error::Validation {
                field: "direction",
                reason: format!("{other:?} is not one of \"outgoing\", \"incoming\", \"both\""),
            };
            Err(crate::error::NodeError::from(err).into())
        }
    }
}

/// Build the core `GraphQuery` from the JS options object.
///
/// # Errors
///
/// `NapiError<&'static str>` with `.code === "Validation"` for a bad
/// `direction`, `asOf`, `recordedAt`, or `scope`.
pub fn graph_query_from(
    o: GraphQueryOptions,
) -> Result<singularmem_core::graph::GraphQuery, napi::Error<&'static str>> {
    Ok(singularmem_core::graph::GraphQuery {
        scope: crate::store::scope_filter(o.scope, o.scope_exact)?,
        as_of: parse_time("asOf", o.as_of.as_deref())?,
        recorded_at: parse_time("recordedAt", o.recorded_at.as_deref())?,
        direction: direction_from(o.direction.as_deref())?,
    })
}

/// Build the object side of a write from an argument string plus the
/// `objectIsValue` / `objectKind` pair.
fn new_object(raw: String, is_value: Option<bool>, kind: Option<String>) -> NewObject {
    if is_value.unwrap_or(false) {
        NewObject::Value(raw)
    } else {
        NewObject::Entity { name: raw, kind }
    }
}

/// Build the object side of an `invalidateFact` / `supersedeFact` argument.
/// Those calls address an *existing* fact, so no `kind` is ever set.
#[must_use]
pub fn change_object(raw: String, is_value: Option<bool>) -> NewObject {
    new_object(raw, is_value, None)
}

/// Convert a JS-sent `NewFact` into the core's `NewFact`.
///
/// # Errors
///
/// `NapiError<&'static str>` with `.code === "Validation"` for a malformed
/// `validFrom` / `validTo`, or `.code === "InvalidId"` for a malformed
/// `sourceItemId`.
pub fn new_fact_to_core(
    f: NewFact,
) -> Result<singularmem_core::graph::NewFact, napi::Error<&'static str>> {
    use std::str::FromStr;

    let source_item_id = match f.source_item_id.as_deref() {
        Some(s) if !s.is_empty() => {
            Some(singularmem_core::item::ItemId::from_str(s).map_err(|e| {
                let node_err = crate::error::NodeError::from(singularmem_core::Error::from(e));
                napi::Error::<&'static str>::from(node_err)
            })?)
        }
        _ => None,
    };
    // f64 → f32 narrowing: the core domain type is f32 and the store
    // constrains the column to [0.0, 1.0], so this costs precision, never
    // magnitude (same reasoning as `read.rs`'s f64 → f32 on the way out).
    #[allow(clippy::cast_possible_truncation)]
    let confidence = f.confidence.unwrap_or(1.0) as f32;

    Ok(singularmem_core::graph::NewFact {
        subject: f.subject,
        subject_kind: f.subject_kind,
        predicate: f.predicate,
        object: new_object(f.object, f.object_is_value, f.object_kind),
        valid_from: parse_time("validFrom", f.valid_from.as_deref())?,
        valid_to: parse_time("validTo", f.valid_to.as_deref())?,
        confidence,
        source_item_id,
        scope: f.scope,
    })
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
            external_id: Some("k".to_string()),
            scope: None,
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
    fn item_external_id_round_trips() {
        let item: Item = sample_core_item().into();
        assert_eq!(item.external_id.as_deref(), Some("k"));
    }

    #[test]
    fn item_scope_round_trips() {
        let core = singularmem_core::Item {
            scope: Some("a/b".to_string()),
            ..sample_core_item()
        };
        let item: Item = core.into();
        assert_eq!(item.scope.as_deref(), Some("a/b"));
    }

    #[test]
    fn item_external_id_none_becomes_none() {
        let core = singularmem_core::Item {
            external_id: None,
            ..sample_core_item()
        };
        let item: Item = core.into();
        assert!(item.external_id.is_none());
    }

    #[test]
    fn js_new_item_never_sets_external_id() {
        let js = NewItem {
            content: "c".to_string(),
            supersedes: None,
            tags: None,
            source: None,
            metadata: None,
            scope: None,
        };
        let core = js_new_item_to_core(js).unwrap();
        assert!(core.external_id.is_none(), "write path stays read-only");
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
        let id = singularmem_core::item::ItemId::from_str("01HXAAAAAAAAAAAAAAAAAAAAA0").unwrap();
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
        let id = singularmem_core::item::ItemId::from_str("01HXAAAAAAAAAAAAAAAAAAAAA0").unwrap();
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

    #[test]
    fn new_item_minimal_only_content() {
        let js = NewItem {
            content: "hello".to_string(),
            supersedes: None,
            tags: None,
            source: None,
            metadata: None,
            scope: None,
        };
        let core = js_new_item_to_core(js).unwrap();
        assert_eq!(core.content, "hello");
        assert_eq!(core.tags, Vec::<String>::new());
        assert_eq!(core.metadata, serde_json::json!({}));
        assert!(core.supersedes.is_none());
        assert!(core.source.is_none());
    }

    #[test]
    fn new_item_full_fields() {
        let js = NewItem {
            content: "full".to_string(),
            supersedes: Some("01HXAAAAAAAAAAAAAAAAAAAAA0".to_string()),
            tags: Some(vec!["a".to_string(), "b".to_string()]),
            source: Some("test".to_string()),
            metadata: Some(serde_json::json!({"k": "v"})),
            scope: None,
        };
        let core = js_new_item_to_core(js).unwrap();
        assert_eq!(core.tags, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(core.source, Some("test".to_string()));
        assert_eq!(core.metadata, serde_json::json!({"k": "v"}));
        assert!(core.supersedes.is_some());
    }

    #[test]
    fn new_item_supersedes_valid_ulid_round_trips() {
        let js = NewItem {
            content: "c".to_string(),
            supersedes: Some("01HXAAAAAAAAAAAAAAAAAAAAA0".to_string()),
            tags: None,
            source: None,
            metadata: None,
            scope: None,
        };
        let core = js_new_item_to_core(js).unwrap();
        assert_eq!(
            core.supersedes.unwrap().to_string(),
            "01HXAAAAAAAAAAAAAAAAAAAAA0"
        );
    }

    #[test]
    fn new_item_supersedes_invalid_ulid_returns_error() {
        let js = NewItem {
            content: "c".to_string(),
            supersedes: Some("not-a-ulid".to_string()),
            tags: None,
            source: None,
            metadata: None,
            scope: None,
        };
        let napi_err = js_new_item_to_core(js).unwrap_err();
        assert_eq!(napi_err.status, "InvalidId");
    }

    #[test]
    fn new_item_supersedes_empty_string_treated_as_none() {
        let js = NewItem {
            content: "c".to_string(),
            supersedes: Some(String::new()),
            tags: None,
            source: None,
            metadata: None,
            scope: None,
        };
        let core = js_new_item_to_core(js).unwrap();
        assert!(core.supersedes.is_none());
    }

    #[test]
    fn new_item_metadata_default_empty_object() {
        let js = NewItem {
            content: "c".to_string(),
            supersedes: None,
            tags: None,
            source: None,
            metadata: None,
            scope: None,
        };
        let core = js_new_item_to_core(js).unwrap();
        assert_eq!(core.metadata, serde_json::json!({}));
    }

    #[test]
    fn new_item_scope_passes_through() {
        let js = NewItem {
            content: "c".to_string(),
            supersedes: None,
            tags: None,
            source: None,
            metadata: None,
            scope: Some("Team/X".to_string()),
        };
        let core = js_new_item_to_core(js).unwrap();
        assert_eq!(core.scope.as_deref(), Some("Team/X"));
    }

    // ── Knowledge graph ──────────────────────────────────────────────────

    fn core_entity_ref(id: &str, name: &str) -> singularmem_core::graph::EntityRef {
        singularmem_core::graph::EntityRef {
            id: singularmem_core::EntityId::from_str(id).unwrap(),
            name: name.to_string(),
        }
    }

    fn sample_core_fact() -> singularmem_core::graph::Fact {
        singularmem_core::graph::Fact {
            id: singularmem_core::FactId::from_str("01HXAAAAAAAAAAAAAAAAAAAAA1").unwrap(),
            subject: core_entity_ref("01HXAAAAAAAAAAAAAAAAAAAAA2", "Singularmem"),
            predicate: "uses".to_string(),
            object: singularmem_core::graph::FactObject::Entity(core_entity_ref(
                "01HXAAAAAAAAAAAAAAAAAAAAA3",
                "Tantivy",
            )),
            valid_from: Some(singularmem_core::graph::time::parse_point("2026-05-16").unwrap()),
            valid_to: None,
            confidence: 1.0,
            source_item_id: None,
            scope: Some("team/backend".to_string()),
            supersedes: None,
            recorded_at: jiff::Timestamp::from_millisecond(1_700_000_000_000).unwrap(),
        }
    }

    #[test]
    fn fact_to_js_maps_entity_objects() {
        let js = fact_to_js(&sample_core_fact());
        assert_eq!(js.subject.name, "Singularmem");
        assert_eq!(js.predicate, "uses");
        assert_eq!(
            js.object.entity.map(|e| e.name),
            Some("Tantivy".to_string())
        );
        assert!(js.object.value.is_none(), "entity objects carry no value");
        assert_eq!(js.scope.as_deref(), Some("team/backend"));
    }

    #[test]
    fn fact_to_js_maps_value_objects() {
        let core = singularmem_core::graph::Fact {
            object: singularmem_core::graph::FactObject::Value("Jonas".to_string()),
            ..sample_core_fact()
        };
        let js = fact_to_js(&core);
        assert_eq!(js.object.value.as_deref(), Some("Jonas"));
        assert!(js.object.entity.is_none(), "value objects carry no entity");
    }

    #[test]
    fn fact_to_js_renders_timestamps_as_rfc3339() {
        let js = fact_to_js(&sample_core_fact());
        assert_eq!(js.valid_from.as_deref(), Some("2026-05-16T00:00:00Z"));
        assert!(js.valid_to.is_none(), "an open fact has no validTo");
        assert_eq!(js.recorded_at, "2023-11-14T22:13:20Z");
    }

    #[test]
    fn parse_time_relabels_the_field_with_the_js_option_name() {
        let err = parse_time("validFrom", Some("not-a-date")).unwrap_err();
        assert_eq!(err.status, "Validation");
        assert!(
            err.reason.contains("validFrom"),
            "message names the option, got {:?}",
            err.reason
        );
    }

    #[test]
    fn parse_time_passes_through_none_and_valid_points() {
        assert!(parse_time("asOf", None).unwrap().is_none());
        assert!(parse_time("asOf", Some("2026-05-16")).unwrap().is_some());
    }

    #[test]
    fn direction_from_accepts_the_three_names_and_defaults_to_both() {
        use singularmem_core::graph::Direction;
        assert_eq!(direction_from(None).unwrap(), Direction::Both);
        assert_eq!(direction_from(Some("both")).unwrap(), Direction::Both);
        assert_eq!(
            direction_from(Some("outgoing")).unwrap(),
            Direction::Outgoing
        );
        assert_eq!(
            direction_from(Some("incoming")).unwrap(),
            Direction::Incoming
        );
    }

    #[test]
    fn direction_from_rejects_anything_else() {
        let err = direction_from(Some("sideways")).unwrap_err();
        assert_eq!(err.status, "Validation");
        assert!(err.reason.contains("direction"), "got {:?}", err.reason);
    }

    fn minimal_new_fact() -> NewFact {
        NewFact {
            subject: "A".to_string(),
            predicate: "uses".to_string(),
            object: "B".to_string(),
            object_is_value: None,
            subject_kind: None,
            object_kind: None,
            valid_from: None,
            valid_to: None,
            confidence: None,
            source_item_id: None,
            scope: None,
        }
    }

    #[test]
    fn new_fact_defaults_confidence_to_one_and_object_to_an_entity() {
        let core = new_fact_to_core(minimal_new_fact()).unwrap();
        assert!((core.confidence - 1.0).abs() < f32::EPSILON);
        assert_eq!(
            core.object,
            NewObject::Entity {
                name: "B".to_string(),
                kind: None
            }
        );
    }

    #[test]
    fn new_fact_object_is_value_switches_to_a_literal() {
        let js = NewFact {
            object_is_value: Some(true),
            object_kind: Some("ignored".to_string()),
            ..minimal_new_fact()
        };
        let core = new_fact_to_core(js).unwrap();
        assert_eq!(core.object, NewObject::Value("B".to_string()));
    }

    #[test]
    fn new_fact_object_kind_reaches_the_entity() {
        let js = NewFact {
            object_kind: Some("library".to_string()),
            subject_kind: Some("project".to_string()),
            ..minimal_new_fact()
        };
        let core = new_fact_to_core(js).unwrap();
        assert_eq!(core.subject_kind.as_deref(), Some("project"));
        assert_eq!(
            core.object,
            NewObject::Entity {
                name: "B".to_string(),
                kind: Some("library".to_string())
            }
        );
    }

    #[test]
    fn new_fact_bad_source_item_id_is_an_invalid_id_error() {
        let js = NewFact {
            source_item_id: Some("not-a-ulid".to_string()),
            ..minimal_new_fact()
        };
        assert_eq!(new_fact_to_core(js).unwrap_err().status, "InvalidId");
    }

    #[test]
    fn new_fact_empty_source_item_id_is_treated_as_none() {
        let js = NewFact {
            source_item_id: Some(String::new()),
            ..minimal_new_fact()
        };
        assert!(new_fact_to_core(js).unwrap().source_item_id.is_none());
    }

    #[test]
    fn change_object_honours_object_is_value() {
        assert_eq!(
            change_object("B".to_string(), None),
            NewObject::Entity {
                name: "B".to_string(),
                kind: None
            }
        );
        assert_eq!(
            change_object("B".to_string(), Some(true)),
            NewObject::Value("B".to_string())
        );
    }

    #[test]
    fn graph_query_from_defaults_to_both_and_no_filters() {
        let q = graph_query_from(GraphQueryOptions::default()).unwrap();
        assert_eq!(q.direction, singularmem_core::graph::Direction::Both);
        assert!(q.scope.is_none());
        assert!(q.as_of.is_none());
        assert!(q.recorded_at.is_none());
    }

    #[test]
    fn graph_query_from_surfaces_a_bad_as_of_by_option_name() {
        let opts = GraphQueryOptions {
            as_of: Some("nope".to_string()),
            ..GraphQueryOptions::default()
        };
        let err = graph_query_from(opts).unwrap_err();
        assert_eq!(err.status, "Validation");
        assert!(err.reason.contains("asOf"), "got {:?}", err.reason);
    }

    #[test]
    fn entity_summary_to_js_flattens_the_nested_entity() {
        let core = singularmem_core::graph::EntitySummary {
            entity: singularmem_core::graph::Entity {
                id: singularmem_core::EntityId::from_str("01HXAAAAAAAAAAAAAAAAAAAAA2").unwrap(),
                name: "Tantivy".to_string(),
                normalised_name: "tantivy".to_string(),
                kind: Some("library".to_string()),
                created_at: jiff::Timestamp::from_millisecond(0).unwrap(),
            },
            fact_count: 3,
        };
        let js = entity_summary_to_js(&core);
        assert_eq!(js.id, "01HXAAAAAAAAAAAAAAAAAAAAA2");
        assert_eq!(js.name, "Tantivy");
        assert_eq!(js.kind.as_deref(), Some("library"));
        assert_eq!(js.fact_count, 3);
    }

    #[test]
    fn graph_stats_to_js_narrows_usize_counts() {
        let js = graph_stats_to_js(singularmem_core::graph::GraphStats {
            entities: 2,
            open_facts: 1,
            closed_facts: 1,
            predicates: 2,
        });
        assert_eq!(js.entities, 2);
        assert_eq!(js.open_facts, 1);
        assert_eq!(js.closed_facts, 1);
        assert_eq!(js.predicates, 2);
    }

    #[test]
    fn timeline_entry_to_js_keeps_the_current_flag() {
        let core = singularmem_core::graph::TimelineEntry {
            fact: sample_core_fact(),
            current: true,
        };
        let js = timeline_entry_to_js(&core);
        assert!(js.current);
        assert_eq!(js.fact.predicate, "uses");
    }
}
