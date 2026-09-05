//! Knowledge-graph methods on `Store` — a second `#[napi] impl Store` block
//! that napi-rs merges with the one in [`crate::store`].
//!
//! Spec: `docs/superpowers/specs/2026-09-05-mcp-surface-16-design.md`
//! § "Node binding". Every method delegates straight to
//! `singularmem_core::Store`; nothing here holds domain logic.
//!
//! # Design note — deferred argument errors
//!
//! Argument validation (a malformed timestamp, an unknown `direction`, a
//! bad ULID) happens on the JS thread *before* the task is queued, but its
//! error is stashed in the task's `pre_error` rather than returned from the
//! method. `compute` then fails immediately and `reject` surfaces the coded
//! error, so `store.addFact({validFrom: 'nope'})` returns a **rejected
//! Promise** instead of throwing synchronously — which is what
//! `assert.rejects` and `.catch()` callers expect. This mirrors
//! `OpenStoreTask::pre_error` in `store.rs`.

use std::str::FromStr;
use std::sync::Arc;

use jiff::Timestamp;
use napi::bindgen_prelude::AsyncTask;
use napi::{Env, Error as NapiError, Task};
use singularmem_core::graph::{
    EntitySummary as CoreEntitySummary, Fact as CoreFact, GraphQuery, GraphStats as CoreGraphStats,
    NewFact as CoreNewFact, NewObject, TimelineEntry as CoreTimelineEntry,
};
use singularmem_core::{Error as CoreError, FactId, ScopeFilter, Store as CoreStore};

use crate::error::{coded_error_to_napi_raw, NodeError};
use crate::store::{scope_filter, Store};
use crate::types::{
    self, EntityListOptions, EntitySummary, Fact, FactChangeOptions, GraphQueryOptions,
    GraphScopeOptions, GraphStats, NewFact, SupersedeResult, TimelineEntry,
};

// ── Shared task plumbing ─────────────────────────────────────────────────────

/// The trigger error `compute` returns once it has stashed the real,
/// coded error in `failed` — `reject` replaces it with that one.
fn trigger(op: &'static str) -> NapiError {
    NapiError::new(napi::Status::GenericFailure, format!("{op} failed"))
}

/// The shared `reject` body: a pre-validation error wins, then the error
/// `compute` stashed, then a last-resort placeholder so a task can never
/// reject without a `.code`.
///
/// Generic over the stashed error's type so `wakeup.rs`'s `WakeupTask` —
/// whose `failed` is already a coded `NapiError<&'static str>` (see
/// `from_retrieve_error`) — can share this with the graph tasks here, whose
/// `failed` is a `NodeError` awaiting conversion.
pub fn reject_coded<F: Into<NapiError<&'static str>>>(
    env: Env,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<F>,
    op: &'static str,
) -> NapiError {
    if let Some(coded) = pre_error {
        return coded_error_to_napi_raw(env, coded);
    }
    let coded: NapiError<&'static str> = failed.map_or_else(
        || {
            NodeError::from(CoreError::Io(std::io::Error::other(format!(
                "unknown {op} error"
            ))))
            .into()
        },
        Into::into,
    );
    coded_error_to_napi_raw(env, coded)
}

/// Stash `e` as the coded error for `reject` and return the trigger.
fn stash(failed: &mut Option<NodeError>, e: CoreError, op: &'static str) -> NapiError {
    *failed = Some(NodeError::from(e));
    trigger(op)
}

/// `Err(trigger)` when a pre-validation error is pending, so `compute`
/// short-circuits without touching the store.
fn short_circuit(pre_error: Option<&NapiError<&'static str>>) -> napi::Result<()> {
    if pre_error.is_some() {
        return Err(trigger("pre-validation"));
    }
    Ok(())
}

/// Render a slice of core facts as their JS mirrors.
fn facts_to_js(facts: &[CoreFact]) -> Vec<Fact> {
    facts.iter().map(types::fact_to_js).collect()
}

// ── AddFactTask ──────────────────────────────────────────────────────────────

// `pub` so the `private_interfaces` lint is satisfied: the `pub fn` returning
// `AsyncTask<Self>` names this type. The `graph` module is not re-exported,
// so external crates cannot reach it.
pub struct AddFactTask {
    store: Arc<CoreStore>,
    /// `None` once `compute` has taken it, or when `pre_error` is set.
    input: Option<CoreNewFact>,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for AddFactTask {
    type Output = CoreFact;
    type JsValue = Fact;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        short_circuit(self.pre_error.as_ref())?;
        let input = self
            .input
            .take()
            .expect("input must be Some when pre_error is None");
        self.store
            .add_fact(input)
            .map_err(|e| stash(&mut self.failed, e, "addFact"))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(types::fact_to_js(&output))
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        Err(reject_coded(
            env,
            self.pre_error.take(),
            self.failed.take(),
            "addFact",
        ))
    }
}

// ── QueryEntityTask ──────────────────────────────────────────────────────────

pub struct QueryEntityTask {
    store: Arc<CoreStore>,
    name: String,
    query: GraphQuery,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for QueryEntityTask {
    type Output = Vec<CoreFact>;
    type JsValue = Vec<Fact>;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        short_circuit(self.pre_error.as_ref())?;
        self.store
            .query_entity(&self.name, &self.query)
            .map_err(|e| stash(&mut self.failed, e, "queryEntity"))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(facts_to_js(&output))
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        Err(reject_coded(
            env,
            self.pre_error.take(),
            self.failed.take(),
            "queryEntity",
        ))
    }
}

// ── QueryPredicateTask ───────────────────────────────────────────────────────

pub struct QueryPredicateTask {
    store: Arc<CoreStore>,
    predicate: String,
    query: GraphQuery,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for QueryPredicateTask {
    type Output = Vec<CoreFact>;
    type JsValue = Vec<Fact>;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        short_circuit(self.pre_error.as_ref())?;
        self.store
            .query_predicate(&self.predicate, &self.query)
            .map_err(|e| stash(&mut self.failed, e, "queryPredicate"))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(facts_to_js(&output))
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        Err(reject_coded(
            env,
            self.pre_error.take(),
            self.failed.take(),
            "queryPredicate",
        ))
    }
}

// ── InvalidateFactTask ───────────────────────────────────────────────────────

pub struct InvalidateFactTask {
    store: Arc<CoreStore>,
    subject: String,
    predicate: String,
    object: NewObject,
    scope: Option<String>,
    at: Option<Timestamp>,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for InvalidateFactTask {
    type Output = CoreFact;
    type JsValue = Fact;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        short_circuit(self.pre_error.as_ref())?;
        self.store
            .invalidate_fact(
                &self.subject,
                &self.predicate,
                &self.object,
                self.scope.as_deref(),
                self.at,
            )
            .map_err(|e| stash(&mut self.failed, e, "invalidateFact"))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(types::fact_to_js(&output))
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        Err(reject_coded(
            env,
            self.pre_error.take(),
            self.failed.take(),
            "invalidateFact",
        ))
    }
}

// ── SupersedeFactTask ────────────────────────────────────────────────────────

pub struct SupersedeFactTask {
    store: Arc<CoreStore>,
    subject: String,
    predicate: String,
    old: NewObject,
    /// `None` once `compute` has taken it, or when `pre_error` is set.
    new: Option<NewObject>,
    scope: Option<String>,
    at: Option<Timestamp>,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for SupersedeFactTask {
    type Output = (Option<CoreFact>, CoreFact);
    type JsValue = SupersedeResult;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        short_circuit(self.pre_error.as_ref())?;
        let new = self
            .new
            .take()
            .expect("new must be Some when pre_error is None");
        self.store
            .supersede_fact(
                &self.subject,
                &self.predicate,
                &self.old,
                new,
                self.scope.as_deref(),
                self.at,
            )
            .map_err(|e| stash(&mut self.failed, e, "supersedeFact"))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        let (closed, opened) = output;
        Ok(SupersedeResult {
            closed: closed.as_ref().map(types::fact_to_js),
            opened: types::fact_to_js(&opened),
        })
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        Err(reject_coded(
            env,
            self.pre_error.take(),
            self.failed.take(),
            "supersedeFact",
        ))
    }
}

// ── TimelineTask ─────────────────────────────────────────────────────────────

pub struct TimelineTask {
    store: Arc<CoreStore>,
    entity: Option<String>,
    scope: Option<ScopeFilter>,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for TimelineTask {
    type Output = Vec<CoreTimelineEntry>;
    type JsValue = Vec<TimelineEntry>;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        short_circuit(self.pre_error.as_ref())?;
        self.store
            .timeline(self.entity.as_deref(), self.scope.as_ref())
            .map_err(|e| stash(&mut self.failed, e, "timeline"))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output.iter().map(types::timeline_entry_to_js).collect())
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        Err(reject_coded(
            env,
            self.pre_error.take(),
            self.failed.take(),
            "timeline",
        ))
    }
}

// ── GraphStatsTask ───────────────────────────────────────────────────────────

pub struct GraphStatsTask {
    store: Arc<CoreStore>,
    scope: Option<ScopeFilter>,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for GraphStatsTask {
    type Output = CoreGraphStats;
    type JsValue = GraphStats;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        short_circuit(self.pre_error.as_ref())?;
        self.store
            .graph_stats(self.scope.as_ref())
            .map_err(|e| stash(&mut self.failed, e, "graphStats"))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(types::graph_stats_to_js(output))
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        Err(reject_coded(
            env,
            self.pre_error.take(),
            self.failed.take(),
            "graphStats",
        ))
    }
}

// ── EntitiesTask ─────────────────────────────────────────────────────────────

pub struct EntitiesTask {
    store: Arc<CoreStore>,
    scope: Option<ScopeFilter>,
    kind: Option<String>,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for EntitiesTask {
    type Output = Vec<CoreEntitySummary>;
    type JsValue = Vec<EntitySummary>;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        short_circuit(self.pre_error.as_ref())?;
        self.store
            .entities(self.scope.as_ref(), self.kind.as_deref())
            .map_err(|e| stash(&mut self.failed, e, "entities"))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(output.iter().map(types::entity_summary_to_js).collect())
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        Err(reject_coded(
            env,
            self.pre_error.take(),
            self.failed.take(),
            "entities",
        ))
    }
}

// ── FactHistoryTask ──────────────────────────────────────────────────────────

pub struct FactHistoryTask {
    store: Arc<CoreStore>,
    /// `None` when the id failed to parse; `pre_error` short-circuits
    /// `compute` in that case, so the value is never read.
    id: Option<FactId>,
    pre_error: Option<NapiError<&'static str>>,
    failed: Option<NodeError>,
}

#[napi]
impl Task for FactHistoryTask {
    type Output = Vec<CoreFact>;
    type JsValue = Vec<Fact>;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        short_circuit(self.pre_error.as_ref())?;
        let id = self.id.expect("id must be Some when pre_error is None");
        self.store
            .fact_history(id)
            .map_err(|e| stash(&mut self.failed, e, "factHistory"))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(facts_to_js(&output))
    }

    fn reject(&mut self, env: Env, _trigger: NapiError) -> napi::Result<Self::JsValue> {
        Err(reject_coded(
            env,
            self.pre_error.take(),
            self.failed.take(),
            "factHistory",
        ))
    }
}

// ── The methods ──────────────────────────────────────────────────────────────

#[napi]
impl Store {
    /// Record a fact in the knowledge graph, creating its entities on demand.
    ///
    /// Idempotent: re-adding an identical open fact returns it unchanged.
    /// A triple that matches an open head but disagrees about the validity
    /// window or confidence is rejected — use `invalidateFact` or
    /// `supersedeFact` to change a standing fact.
    ///
    /// @param fact The fact to record (see `NewFact`).
    /// @returns The persisted `Fact` revision.
    /// @throws `{ code: "Validation" }` — bad entity name, predicate, confidence, timestamp, scope, kind, or a conflicting open head.
    /// @throws `{ code: "InvalidId" }` — `sourceItemId` is not a valid ULID.
    /// @throws `{ code: "ReadOnly" }` — the store was opened with `{ readOnly: true }`.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn add_fact(&self, fact: NewFact) -> napi::Result<AsyncTask<AddFactTask>> {
        let (input, pre_error) = match types::new_fact_to_core(fact) {
            Ok(f) => (Some(f), None),
            Err(e) => (None, Some(e)),
        };
        Ok(AsyncTask::new(AddFactTask {
            store: self.inner.clone(),
            input,
            pre_error,
            failed: None,
        }))
    }

    /// Facts touching the entity `name`, oldest recorded first.
    ///
    /// The name resolves across scopes; `options.scope` filters the facts.
    /// An unknown entity yields an empty array rather than an error.
    ///
    /// @param name The entity's name (normalised for lookup).
    /// @param options Direction, time-travel and scope filters (see `GraphQueryOptions`).
    /// @returns Matching `Fact` revisions.
    /// @throws `{ code: "Validation" }` — `name` does not normalise, or `direction` / `asOf` / `recordedAt` / `scope` is malformed.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn query_entity(
        &self,
        name: String,
        options: Option<GraphQueryOptions>,
    ) -> napi::Result<AsyncTask<QueryEntityTask>> {
        let (query, pre_error) = match types::graph_query_from(options.unwrap_or_default()) {
            Ok(q) => (q, None),
            Err(e) => (GraphQuery::default(), Some(e)),
        };
        Ok(AsyncTask::new(QueryEntityTask {
            store: self.inner.clone(),
            name,
            query,
            pre_error,
            failed: None,
        }))
    }

    /// Every fact with `predicate`, oldest recorded first.
    ///
    /// `options.direction` is validated but has no effect for predicate
    /// queries — a predicate query has no side.
    ///
    /// @param predicate The predicate (normalised for lookup).
    /// @param options Time-travel and scope filters (see `GraphQueryOptions`).
    /// @returns Matching `Fact` revisions.
    /// @throws `{ code: "Validation" }` — `predicate` does not normalise, or `asOf` / `recordedAt` / `scope` is malformed.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn query_predicate(
        &self,
        predicate: String,
        options: Option<GraphQueryOptions>,
    ) -> napi::Result<AsyncTask<QueryPredicateTask>> {
        let (query, pre_error) = match types::graph_query_from(options.unwrap_or_default()) {
            Ok(q) => (q, None),
            Err(e) => (GraphQuery::default(), Some(e)),
        };
        Ok(AsyncTask::new(QueryPredicateTask {
            store: self.inner.clone(),
            predicate,
            query,
            pre_error,
            failed: None,
        }))
    }

    /// End the open fact `subject —predicate→ object` by appending a closing
    /// revision. The original row is never modified.
    ///
    /// @param subject The subject entity's name.
    /// @param predicate The predicate.
    /// @param object The object: an entity name, or a literal when `options.objectIsValue`.
    /// @param options `objectIsValue`, `at` (default: now) and `scope` (see `FactChangeOptions`).
    /// @returns The closing `Fact` revision.
    /// @throws `{ code: "FactNotFound" }` — no open head matches the triple.
    /// @throws `{ code: "Validation" }` — `at` is malformed or precedes the fact's `validFrom`.
    /// @throws `{ code: "ReadOnly" }` — the store was opened with `{ readOnly: true }`.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn invalidate_fact(
        &self,
        subject: String,
        predicate: String,
        object: String,
        options: Option<FactChangeOptions>,
    ) -> napi::Result<AsyncTask<InvalidateFactTask>> {
        let o = options.unwrap_or_default();
        let (at, pre_error) = match types::parse_time("at", o.at.as_deref()) {
            Ok(t) => (t, None),
            Err(e) => (None, Some(e)),
        };
        Ok(AsyncTask::new(InvalidateFactTask {
            store: self.inner.clone(),
            subject,
            predicate,
            object: types::change_object(object, o.object_is_value),
            scope: o.scope,
            at,
            pre_error,
            failed: None,
        }))
    }

    /// Replace one fact with another in a single transaction: close
    /// `oldObject` at `options.at` and open `newObject` from the same
    /// instant.
    ///
    /// A missing old fact is tolerated: the result's `closed` is absent and
    /// the new fact is still opened. Any other failure rolls the whole thing
    /// back, so the old fact is never left closed without its replacement.
    ///
    /// `options.objectIsValue` applies to **both** objects.
    ///
    /// @param subject The subject entity's name.
    /// @param predicate The predicate.
    /// @param oldObject The object of the fact being replaced.
    /// @param newObject The object of the replacement fact.
    /// @param options `objectIsValue`, `at` (default: now) and `scope` (see `FactChangeOptions`).
    /// @returns `{ closed, opened }` — the closing revision (absent if there was none) and the new fact.
    /// @throws `{ code: "Validation" }` — `at` is malformed, or the new fact fails validation.
    /// @throws `{ code: "ReadOnly" }` — the store was opened with `{ readOnly: true }`.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn supersede_fact(
        &self,
        subject: String,
        predicate: String,
        old_object: String,
        new_object: String,
        options: Option<FactChangeOptions>,
    ) -> napi::Result<AsyncTask<SupersedeFactTask>> {
        let o = options.unwrap_or_default();
        let (at, pre_error) = match types::parse_time("at", o.at.as_deref()) {
            Ok(t) => (t, None),
            Err(e) => (None, Some(e)),
        };
        Ok(AsyncTask::new(SupersedeFactTask {
            store: self.inner.clone(),
            subject,
            predicate,
            old: types::change_object(old_object, o.object_is_value),
            new: Some(types::change_object(new_object, o.object_is_value)),
            scope: o.scope,
            at,
            pre_error,
            failed: None,
        }))
    }

    /// Head revisions — open and closed alike — ordered by `validFrom`
    /// ascending with "since unknown" (absent `validFrom`) first, then by
    /// record time, then by id. Capped at 500 rows.
    ///
    /// @param entity Restrict to facts touching this entity; omit for the whole graph.
    /// @param options Scope filter (see `GraphScopeOptions`).
    /// @returns `TimelineEntry` rows, each flagging whether it is still open.
    /// @throws `{ code: "Validation" }` — `entity` does not normalise, or `scope` is malformed.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn timeline(
        &self,
        entity: Option<String>,
        options: Option<GraphScopeOptions>,
    ) -> napi::Result<AsyncTask<TimelineTask>> {
        let o = options.unwrap_or_default();
        let (scope, pre_error) = match scope_filter(o.scope, o.scope_exact) {
            Ok(f) => (f, None),
            Err(e) => (None, Some(e)),
        };
        Ok(AsyncTask::new(TimelineTask {
            store: self.inner.clone(),
            entity,
            scope,
            pre_error,
            failed: None,
        }))
    }

    /// Entity, open-fact, closed-fact and distinct-predicate counts over head
    /// revisions. With a scope filter, only facts in scope are counted — and
    /// only entities taking part in one of them.
    ///
    /// @param options Scope filter (see `GraphScopeOptions`).
    /// @returns The aggregate counts.
    /// @throws `{ code: "Validation" }` — `scope` is malformed.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn graph_stats(
        &self,
        options: Option<GraphScopeOptions>,
    ) -> napi::Result<AsyncTask<GraphStatsTask>> {
        let o = options.unwrap_or_default();
        let (scope, pre_error) = match scope_filter(o.scope, o.scope_exact) {
            Ok(f) => (f, None),
            Err(e) => (None, Some(e)),
        };
        Ok(AsyncTask::new(GraphStatsTask {
            store: self.inner.clone(),
            scope,
            pre_error,
            failed: None,
        }))
    }

    /// Entities sorted by normalised name, each with the number of head facts
    /// it takes part in.
    ///
    /// Entities are store-global; `options.scope` filters on **fact** scope,
    /// restricting both the counted facts and the entities returned.
    ///
    /// @param options `kind`, `scope` and `scopeExact` filters (see `EntityListOptions`).
    /// @returns One `EntitySummary` per entity, name ascending.
    /// @throws `{ code: "Validation" }` — `scope` is malformed.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn entities(
        &self,
        options: Option<EntityListOptions>,
    ) -> napi::Result<AsyncTask<EntitiesTask>> {
        let o = options.unwrap_or_default();
        let (scope, pre_error) = match scope_filter(o.scope, o.scope_exact) {
            Ok(f) => (f, None),
            Err(e) => (None, Some(e)),
        };
        Ok(AsyncTask::new(EntitiesTask {
            store: self.inner.clone(),
            scope,
            kind: o.kind,
            pre_error,
            failed: None,
        }))
    }

    /// The revision chain containing `factId`, oldest first.
    ///
    /// @param factId A 26-character Crockford base32 ULID naming any revision in the chain.
    /// @returns Every revision of that fact, oldest first.
    /// @throws `{ code: "InvalidId" }` — `factId` is not a valid ULID.
    /// @throws `{ code: "FactIdNotFound" }` — no fact revision has that id.
    /// @throws `{ code: "AmbiguousFactRevision" }` — the chain forks; the library refuses to pick a branch.
    /// @throws `{ code: "Sqlite" }` — underlying `SQLite` error.
    #[napi]
    #[allow(clippy::missing_errors_doc)]
    pub fn fact_history(&self, fact_id: String) -> napi::Result<AsyncTask<FactHistoryTask>> {
        let (id, pre_error) = match FactId::from_str(&fact_id) {
            Ok(id) => (Some(id), None),
            Err(e) => {
                let coded: NapiError<&'static str> = NodeError::from(CoreError::from(e)).into();
                (None, Some(coded))
            }
        };
        Ok(AsyncTask::new(FactHistoryTask {
            store: self.inner.clone(),
            id,
            pre_error,
            failed: None,
        }))
    }
}
