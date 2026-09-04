//! Public types for the knowledge graph: entities, facts, and the query
//! surface built on top of them. Spec: `docs/superpowers/specs/2026-09-05-knowledge-graph-14-design.md`
//! § "Data model".

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::{EntityId, FactId, ItemId, ScopeFilter};

/// A node in the knowledge graph: a named thing facts can refer to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    /// Stable identifier, minted at first creation.
    pub id: EntityId,
    /// The name as first written (display form).
    pub name: String,
    /// The normalised form used for identity and lookup — see
    /// [`crate::graph::normalise::entity_name`].
    pub normalised_name: String,
    /// Free-form kind, set on first creation and immutable afterwards.
    pub kind: Option<String>,
    /// Scope path the entity was created under, if any.
    pub scope: Option<String>,
    /// When this entity was first created.
    pub created_at: Timestamp,
}

/// A lightweight reference to an [`Entity`] — used where a fact needs to
/// point at an entity without pulling in its full record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRef {
    /// The referenced entity's id.
    pub id: EntityId,
    /// The referenced entity's display name.
    pub name: String,
}

/// The object side of a fact: either another entity or a literal value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactObject {
    /// The object is another entity in the graph.
    Entity(EntityRef),
    /// The object is a literal string value, not an entity.
    Value(String),
}

/// One revision of a fact: `subject predicate object`, with a validity
/// window and provenance. Spec § "Revisions and the two time axes".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    /// Stable identifier of this specific revision.
    pub id: FactId,
    /// The entity this fact is about.
    pub subject: EntityRef,
    /// Normalised predicate, e.g. `"works_at"`.
    pub predicate: String,
    /// The fact's object: an entity or a literal value.
    pub object: FactObject,
    /// Start of the validity window. `None` means "since unknown".
    pub valid_from: Option<Timestamp>,
    /// End of the validity window. `None` means "still valid" (open).
    pub valid_to: Option<Timestamp>,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// The item this fact was extracted from, if any.
    pub source_item_id: Option<ItemId>,
    /// Scope path this fact was recorded under, if any.
    pub scope: Option<String>,
    /// The prior revision this one supersedes, if any.
    pub supersedes: Option<FactId>,
    /// When this revision was recorded (append time, distinct from
    /// `valid_from`/`valid_to`).
    pub recorded_at: Timestamp,
}

/// The object side of a not-yet-persisted fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewObject {
    /// The object is an entity, identified by name; created if it does not
    /// already exist.
    Entity {
        /// The entity's display name (normalised at write time).
        name: String,
        /// Kind to set if the entity is being created for the first time.
        kind: Option<String>,
    },
    /// The object is a literal string value, not an entity.
    Value(String),
}

/// A fact submitted for persistence. The store resolves (and, if needed,
/// creates) `subject`/object entities and assigns `id`, `recorded_at`.
#[derive(Debug, Clone, PartialEq)]
pub struct NewFact {
    /// The subject entity's display name (normalised at write time).
    pub subject: String,
    /// Kind to set on the subject if it is being created for the first time.
    pub subject_kind: Option<String>,
    /// Predicate (normalised at write time).
    pub predicate: String,
    /// The fact's object.
    pub object: NewObject,
    /// Start of the validity window. `None` means "since unknown".
    pub valid_from: Option<Timestamp>,
    /// End of the validity window. `None` means "still valid" (open).
    pub valid_to: Option<Timestamp>,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// The item this fact was extracted from, if any.
    pub source_item_id: Option<ItemId>,
    /// Scope path to record this fact under, if any.
    pub scope: Option<String>,
}

impl NewFact {
    /// Entity-object fact with confidence 1.0 and no window.
    #[must_use]
    pub fn triple(subject: &str, predicate: &str, object: &str) -> Self {
        Self {
            subject: subject.into(),
            subject_kind: None,
            predicate: predicate.into(),
            object: NewObject::Entity {
                name: object.into(),
                kind: None,
            },
            valid_from: None,
            valid_to: None,
            confidence: 1.0,
            source_item_id: None,
            scope: None,
        }
    }
}

/// Which side of a fact an entity query should match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// The entity is the subject.
    Outgoing,
    /// The entity is the object.
    Incoming,
    /// Either side.
    #[default]
    Both,
}

/// Filters shared by the graph's read operations.
#[derive(Debug, Clone, Default)]
pub struct GraphQuery {
    /// Restrict results to this scope.
    pub scope: Option<ScopeFilter>,
    /// Restrict to facts valid at this instant (see spec's "as-of" rule).
    pub as_of: Option<Timestamp>,
    /// Restrict to facts believed as of this record time (see spec's
    /// "recorded-at" rule).
    pub recorded_at: Option<Timestamp>,
    /// Which side of the fact to match against the queried entity.
    pub direction: Direction,
}

/// One row of a [`timeline`](crate::graph) result: a fact plus whether it is
/// currently the open head.
#[derive(Debug, Clone, Serialize)]
pub struct TimelineEntry {
    /// The fact itself.
    #[serde(flatten)]
    pub fact: Fact,
    /// Whether this revision is the current, open head.
    pub current: bool,
}

/// Aggregate counts over the graph (optionally scoped).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct GraphStats {
    /// Total number of entities.
    pub entities: usize,
    /// Number of open (currently valid) fact heads.
    pub open_facts: usize,
    /// Number of closed (superseded or explicitly ended) fact heads.
    pub closed_facts: usize,
    /// Number of distinct predicates in use.
    pub predicates: usize,
}

/// An entity plus how many facts reference it.
#[derive(Debug, Clone, Serialize)]
pub struct EntitySummary {
    /// The entity itself.
    #[serde(flatten)]
    pub entity: Entity,
    /// Number of facts (any state) where this entity is subject or object.
    pub fact_count: usize,
}
