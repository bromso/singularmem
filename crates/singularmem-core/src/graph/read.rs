//! Graph read operations on [`Store`]: entity/predicate queries, timeline,
//! stats, entity listing, and revision history.
//!
//! Spec: `docs/superpowers/specs/2026-09-05-knowledge-graph-14-design.md`
//! § "Revisions and the two time axes" (which the [`fact_where`] helper
//! encodes once for every read) and § "Operations".

use rusqlite::{Connection, OptionalExtension, Row};

use crate::error::{Error, Result};
use crate::graph::normalise;
use crate::graph::time::parse_point;
use crate::graph::types::{
    Direction, Entity, EntityRef, EntitySummary, Fact, FactObject, GraphQuery, GraphStats,
    TimelineEntry,
};
use crate::id::FactId;
use crate::scope::ScopeFilter;
use crate::store::Store;

/// Most rows a `timeline` call returns (spec § "Operations": "Cap 500").
const TIMELINE_LIMIT: usize = 500;

/// Column list and joins shared by every fact read. Column order is the one
/// [`read_fact_columns`] expects.
const FACT_SELECT: &str = "SELECT f.id, s.id, s.name, f.predicate, f.object_id, o.name, \
     f.object_value, f.valid_from, f.valid_to, f.confidence, f.source_item_id, f.scope, \
     f.supersedes, f.recorded_at \
     FROM facts f \
     JOIN entities s ON f.subject_id = s.id \
     LEFT JOIN entities o ON f.object_id = o.id";

/// Which fact revisions a read sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Validity {
    /// Apply the query's validity window: open facts only, or — with
    /// `as_of` — the half-open window around that instant.
    Window,
    /// Every head revision, open and closed alike (`timeline`, `stats`).
    AnyHead,
}

/// The shared read predicate: head selection, the two time axes, and scope.
/// Every read in this module builds its `WHERE` clause from this so the
/// spec's rules live in exactly one place.
///
/// - **head** (no `recorded_at`): nothing supersedes the row.
/// - **recorded-at `R`**: the newest revision of the chain recorded at or
///   before `R` — the row itself is at or before `R` and nothing recorded by
///   then supersedes it.
/// - **as-of `T`** (`Validity::Window`): half-open `[valid_from, valid_to)`,
///   with `NULL` meaning "since unknown" / "still valid".
/// - **default** (`Validity::Window`, no `as_of`): open facts only.
/// - **scope**: [`ScopeFilter::sql_clause`] rebound to `facts.scope`.
///
/// Returns the clause (each fragment parenthesised, `AND`-joined, never
/// empty) and its bind values in placeholder order.
pub(super) fn fact_where(q: &GraphQuery, validity: Validity) -> (String, Vec<String>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(recorded_at) = q.recorded_at {
        clauses.push(
            "(f.recorded_at <= ? AND NOT EXISTS \
             (SELECT 1 FROM facts g WHERE g.supersedes = f.id AND g.recorded_at <= ?))"
                .to_string(),
        );
        let r = recorded_at.to_string();
        binds.push(r.clone());
        binds.push(r);
    } else {
        clauses.push("(NOT EXISTS (SELECT 1 FROM facts g WHERE g.supersedes = f.id))".to_string());
    }

    if validity == Validity::Window {
        if let Some(as_of) = q.as_of {
            clauses.push(
                "((f.valid_from IS NULL OR f.valid_from <= ?) \
                 AND (f.valid_to IS NULL OR ? < f.valid_to))"
                    .to_string(),
            );
            let t = as_of.to_string();
            binds.push(t.clone());
            binds.push(t);
        } else {
            clauses.push("(f.valid_to IS NULL)".to_string());
        }
    }

    if let Some(filter) = q.scope.as_ref() {
        let (clause, values) = filter.sql_clause();
        clauses.push(format!("({})", clause.replace("scope", "f.scope")));
        binds.extend(values);
    }

    (clauses.join(" AND "), binds)
}

/// A `GraphQuery` carrying nothing but a scope filter — the shape
/// `timeline`, `graph_stats`, and `entities` need from [`fact_where`].
fn scope_only(scope: Option<&ScopeFilter>) -> GraphQuery {
    GraphQuery {
        scope: scope.cloned(),
        ..GraphQuery::default()
    }
}

/// `IN (?, ?, …)` with one placeholder per element of `n`.
fn placeholders(n: usize) -> String {
    let mut out = String::with_capacity(n * 3);
    for i in 0..n {
        if i > 0 {
            out.push_str(", ");
        }
        out.push('?');
    }
    out
}

/// One `facts` row exactly as `SQLite` stores it, before any parsing.
struct RawFact {
    id: String,
    subject_id: String,
    subject_name: String,
    predicate: String,
    object_id: Option<String>,
    object_name: Option<String>,
    object_value: Option<String>,
    valid_from: Option<String>,
    valid_to: Option<String>,
    confidence: f64,
    source_item_id: Option<String>,
    scope: Option<String>,
    supersedes: Option<String>,
    recorded_at: String,
}

/// Read a [`FACT_SELECT`] row into its raw column values.
fn read_fact_columns(row: &Row<'_>) -> rusqlite::Result<RawFact> {
    Ok(RawFact {
        id: row.get(0)?,
        subject_id: row.get(1)?,
        subject_name: row.get(2)?,
        predicate: row.get(3)?,
        object_id: row.get(4)?,
        object_name: row.get(5)?,
        object_value: row.get(6)?,
        valid_from: row.get(7)?,
        valid_to: row.get(8)?,
        confidence: row.get(9)?,
        source_item_id: row.get(10)?,
        scope: row.get(11)?,
        supersedes: row.get(12)?,
        recorded_at: row.get(13)?,
    })
}

impl RawFact {
    /// Parse ids and timestamps into a [`Fact`].
    ///
    /// `confidence` is stored as `REAL` (an f64) and the domain type is an
    /// f32; the DDL constrains the column to `[0.0, 1.0]`, so the narrowing
    /// costs precision, never magnitude.
    #[allow(clippy::cast_possible_truncation)]
    fn into_fact(self) -> Result<Fact> {
        let object = match (self.object_id, self.object_name, self.object_value) {
            (Some(id), name, _) => FactObject::Entity(EntityRef {
                id: id.parse()?,
                name: name.unwrap_or_default(),
            }),
            (None, _, Some(value)) => FactObject::Value(value),
            (None, _, None) => {
                return Err(Error::Validation {
                    field: "object",
                    reason: format!("fact {} has neither object_id nor object_value", self.id),
                })
            }
        };
        Ok(Fact {
            id: self.id.parse()?,
            subject: EntityRef {
                id: self.subject_id.parse()?,
                name: self.subject_name,
            },
            predicate: self.predicate,
            object,
            valid_from: self.valid_from.as_deref().map(parse_point).transpose()?,
            valid_to: self.valid_to.as_deref().map(parse_point).transpose()?,
            confidence: self.confidence as f32,
            source_item_id: self.source_item_id.map(|s| s.parse()).transpose()?,
            scope: self.scope,
            supersedes: self.supersedes.map(|s| s.parse()).transpose()?,
            recorded_at: parse_point(&self.recorded_at)?,
        })
    }
}

/// Run a [`FACT_SELECT`]-shaped query and parse every row.
pub(super) fn select_facts(
    conn: &Connection,
    sql: &str,
    binds: &[String],
    context: &'static str,
) -> Result<Vec<Fact>> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| Error::Sqlite { context, source: e })?;
    let raws = stmt
        .query_map(rusqlite::params_from_iter(binds.iter()), |row| {
            read_fact_columns(row)
        })
        .map_err(|e| Error::Sqlite { context, source: e })?
        .collect::<rusqlite::Result<Vec<RawFact>>>()
        .map_err(|e| Error::Sqlite { context, source: e })?;
    drop(stmt);
    raws.into_iter().map(RawFact::into_fact).collect()
}

/// Load one fact by id, or `None` if the id is unknown. Takes a connection
/// (not `&Store`) so the write path can call it inside its transaction.
pub(super) fn load_fact(conn: &Connection, id: FactId) -> Result<Option<Fact>> {
    let sql = format!("{FACT_SELECT} WHERE f.id = ?");
    let mut facts = select_facts(conn, &sql, &[id.to_string()], "loading fact by id")?;
    Ok(facts.pop())
}

/// Ids of every entity whose normalised name is `name` — at most one, since
/// entities are store-global. Returned as a list so the callers' `IN (…)`
/// clauses stay uniform; a query's scope filter narrows the *facts*, never
/// which entity the name resolves to (spec § "Operations").
pub(super) fn entity_ids_by_name(conn: &Connection, name: &str) -> Result<Vec<String>> {
    let normalised = normalise::entity_name(name)?;
    let mut stmt = conn
        .prepare("SELECT id FROM entities WHERE normalised_name = ? ORDER BY id")
        .map_err(|e| Error::Sqlite {
            context: "preparing entity lookup",
            source: e,
        })?;
    let ids = stmt
        .query_map([normalised], |row| row.get::<_, String>(0))
        .map_err(|e| Error::Sqlite {
            context: "looking up entity by name",
            source: e,
        })?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|e| Error::Sqlite {
            context: "collecting entity ids",
            source: e,
        })?;
    drop(stmt);
    Ok(ids)
}

/// `f.subject_id IN (…)` / `f.object_id IN (…)` / either, plus the bind
/// values (repeated once per side for [`Direction::Both`]).
fn direction_clause(direction: Direction, ids: &[String]) -> (String, Vec<String>) {
    let list = placeholders(ids.len());
    match direction {
        Direction::Outgoing => (format!("f.subject_id IN ({list})"), ids.to_vec()),
        Direction::Incoming => (format!("f.object_id IN ({list})"), ids.to_vec()),
        Direction::Both => {
            let mut binds = ids.to_vec();
            binds.extend_from_slice(ids);
            (
                format!("(f.subject_id IN ({list}) OR f.object_id IN ({list}))"),
                binds,
            )
        }
    }
}

/// One `entities` row exactly as `SQLite` stores it, before any parsing.
struct RawEntity {
    id: String,
    name: String,
    normalised_name: String,
    kind: Option<String>,
    created_at: String,
}

/// Read one `entities` row (the column order [`Store::entities`] and
/// [`Store::get_entity`] both select).
fn read_entity(row: &Row<'_>) -> rusqlite::Result<RawEntity> {
    Ok(RawEntity {
        id: row.get(0)?,
        name: row.get(1)?,
        normalised_name: row.get(2)?,
        kind: row.get(3)?,
        created_at: row.get(4)?,
    })
}

impl RawEntity {
    /// Parse the id and creation time into an [`Entity`].
    fn into_entity(self) -> Result<Entity> {
        Ok(Entity {
            id: self.id.parse()?,
            name: self.name,
            normalised_name: self.normalised_name,
            kind: self.kind,
            created_at: parse_point(&self.created_at)?,
        })
    }
}

/// Every revision that supersedes `id`, oldest recorded first. More than
/// one means the chain forks — see [`Store::fact_history`].
fn successors_of(conn: &Connection, id: FactId) -> Result<Vec<FactId>> {
    let mut stmt = conn
        .prepare("SELECT id FROM facts WHERE supersedes = ? ORDER BY recorded_at ASC, id ASC")
        .map_err(|e| Error::Sqlite {
            context: "walking the fact chain forwards",
            source: e,
        })?;
    let ids = stmt
        .query_map([id.to_string()], |row| row.get::<_, String>(0))
        .map_err(|e| Error::Sqlite {
            context: "walking the fact chain forwards",
            source: e,
        })?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|e| Error::Sqlite {
            context: "collecting fact chain successors",
            source: e,
        })?;
    drop(stmt);
    ids.into_iter()
        .map(|s| s.parse::<FactId>().map_err(Into::into))
        .collect()
}

/// `COUNT(*)` of head facts (matching `filter`) touching entity `e`.
fn fact_count_subquery(filter: &str) -> String {
    format!(
        "(SELECT COUNT(*) FROM facts f \
         WHERE (f.subject_id = e.id OR f.object_id = e.id) AND {filter})"
    )
}

impl Store {
    /// Facts where `name` is the subject, the object, or either, per
    /// `q.direction`, oldest recorded first.
    ///
    /// The name resolves across scopes; `q.scope` filters the facts. An
    /// unknown entity yields an empty vector rather than an error.
    ///
    /// # Errors
    /// `Error::Validation { field: "entity" }` if `name` does not normalise;
    /// `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn query_entity(&self, name: &str, q: &GraphQuery) -> Result<Vec<Fact>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let ids = entity_ids_by_name(&conn, name)?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let (side, mut binds) = direction_clause(q.direction, &ids);
        let (filter, filter_binds) = fact_where(q, Validity::Window);
        binds.extend(filter_binds);
        let sql =
            format!("{FACT_SELECT} WHERE {side} AND {filter} ORDER BY f.recorded_at ASC, f.id ASC");
        let facts = select_facts(&conn, &sql, &binds, "querying facts by entity");
        drop(conn);
        facts
    }

    /// Every fact with `predicate` (normalised), oldest recorded first.
    ///
    /// # Errors
    /// `Error::Validation { field: "predicate" }` if `predicate` does not
    /// normalise; `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn query_predicate(&self, predicate: &str, q: &GraphQuery) -> Result<Vec<Fact>> {
        let normalised = normalise::predicate(predicate)?;
        let (filter, filter_binds) = fact_where(q, Validity::Window);
        let mut binds = vec![normalised];
        binds.extend(filter_binds);
        let sql = format!(
            "{FACT_SELECT} WHERE f.predicate = ? AND {filter} \
             ORDER BY f.recorded_at ASC, f.id ASC"
        );
        let conn = self.conn.lock().expect("store mutex poisoned");
        select_facts(&conn, &sql, &binds, "querying facts by predicate")
    }

    /// Head revisions — open and closed — ordered by `valid_from` ascending
    /// with unknown starts last, then by record time. Capped at 500 rows.
    ///
    /// # Errors
    /// `Error::Validation { field: "entity" }` if `entity` does not
    /// normalise; `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn timeline(
        &self,
        entity: Option<&str>,
        scope: Option<&ScopeFilter>,
    ) -> Result<Vec<TimelineEntry>> {
        let q = scope_only(scope);
        let (filter, mut binds) = fact_where(&q, Validity::AnyHead);
        let mut sql = format!("{FACT_SELECT} WHERE {filter}");
        let conn = self.conn.lock().expect("store mutex poisoned");
        if let Some(name) = entity {
            let ids = entity_ids_by_name(&conn, name)?;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let (side, side_binds) = direction_clause(Direction::Both, &ids);
            sql.push_str(" AND ");
            sql.push_str(&side);
            binds.extend(side_binds);
        }
        sql.push_str(" ORDER BY f.valid_from IS NULL, f.valid_from ASC, f.recorded_at ASC LIMIT ");
        sql.push_str(&TIMELINE_LIMIT.to_string());
        let facts = select_facts(&conn, &sql, &binds, "reading timeline")?;
        drop(conn);
        Ok(facts
            .into_iter()
            .map(|fact| TimelineEntry {
                current: fact.valid_to.is_none(),
                fact,
            })
            .collect())
    }

    /// Entity, open-fact, closed-fact, and distinct-predicate counts over
    /// head revisions. With a scope filter, only facts in scope are counted
    /// and only entities taking part in one of them.
    ///
    /// # Errors
    /// `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    // Two queries share one guard: dropping it between them would let
    // another writer interleave and make the counts mutually inconsistent.
    #[allow(clippy::significant_drop_tightening)]
    pub fn graph_stats(&self, scope: Option<&ScopeFilter>) -> Result<GraphStats> {
        let q = scope_only(scope);
        let (filter, binds) = fact_where(&q, Validity::AnyHead);
        let sql = format!(
            "SELECT \
             (SELECT COUNT(*) FROM facts f WHERE {filter} AND f.valid_to IS NULL), \
             (SELECT COUNT(*) FROM facts f WHERE {filter} AND f.valid_to IS NOT NULL), \
             (SELECT COUNT(DISTINCT f.predicate) FROM facts f WHERE {filter})"
        );
        // The clause appears three times, so its binds do too.
        let mut fact_binds = binds.clone();
        fact_binds.extend(binds.iter().cloned());
        fact_binds.extend(binds.iter().cloned());

        let conn = self.conn.lock().expect("store mutex poisoned");
        let (open, closed, predicates): (i64, i64, i64) = conn
            .query_row(&sql, rusqlite::params_from_iter(fact_binds.iter()), |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| Error::Sqlite {
                context: "counting graph facts",
                source: e,
            })?;

        let (entity_sql, entity_binds) = if scope.is_some() {
            (
                format!(
                    "SELECT COUNT(*) FROM entities e WHERE EXISTS \
                     (SELECT 1 FROM facts f \
                      WHERE (f.subject_id = e.id OR f.object_id = e.id) AND {filter})"
                ),
                binds,
            )
        } else {
            ("SELECT COUNT(*) FROM entities".to_string(), Vec::new())
        };
        let entities: i64 = conn
            .query_row(
                &entity_sql,
                rusqlite::params_from_iter(entity_binds.iter()),
                |row| row.get(0),
            )
            .map_err(|e| Error::Sqlite {
                context: "counting graph entities",
                source: e,
            })?;

        let count = |n: i64| usize::try_from(n).unwrap_or(0);
        Ok(GraphStats {
            entities: count(entities),
            open_facts: count(open),
            closed_facts: count(closed),
            predicates: count(predicates),
        })
    }

    /// Entities sorted by normalised name, each with the number of head
    /// facts it takes part in (as subject or object). `kind` filters by the
    /// entity's kind.
    ///
    /// Entities themselves are store-global; `scope` filters on **fact**
    /// scope, restricting both the counted facts and the entities returned
    /// to those taking part in a fact in scope.
    ///
    /// # Errors
    /// `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    // The statement borrows `conn`, so the guard cannot be dropped before
    // the rows are collected; tightening the scope would not compile.
    #[allow(clippy::significant_drop_tightening)]
    pub fn entities(
        &self,
        scope: Option<&ScopeFilter>,
        kind: Option<&str>,
    ) -> Result<Vec<EntitySummary>> {
        let q = scope_only(scope);
        let (filter, filter_binds) = fact_where(&q, Validity::AnyHead);
        let mut sql = format!(
            "SELECT e.id, e.name, e.normalised_name, e.kind, e.created_at, {} \
             FROM entities e WHERE 1=1",
            fact_count_subquery(&filter)
        );
        let mut binds = filter_binds.clone();
        if let Some(k) = kind {
            sql.push_str(" AND e.kind = ?");
            binds.push(k.to_string());
        }
        if scope.is_some() {
            sql.push_str(" AND EXISTS ");
            sql.push_str(&fact_count_subquery(&filter).replace("COUNT(*)", "1"));
            binds.extend(filter_binds);
        }
        sql.push_str(" ORDER BY e.normalised_name ASC");

        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&sql).map_err(|e| Error::Sqlite {
            context: "preparing entity listing",
            source: e,
        })?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(binds.iter()), |row| {
                Ok((read_entity(row)?, row.get::<_, i64>(5)?))
            })
            .map_err(|e| Error::Sqlite {
                context: "listing entities",
                source: e,
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| Error::Sqlite {
                context: "collecting entities",
                source: e,
            })?;
        drop(stmt);
        rows.into_iter()
            .map(|(raw, n)| {
                Ok(EntitySummary {
                    entity: raw.into_entity()?,
                    fact_count: usize::try_from(n).unwrap_or(0),
                })
            })
            .collect()
    }

    /// The revision chain containing `id`, oldest first.
    ///
    /// # Errors
    /// `Error::FactIdNotFound` if `id` is unknown;
    /// `Error::AmbiguousFactRevision` if more than one revision supersedes
    /// the same one (a forked chain — the library refuses to pick a branch);
    /// `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    // The whole chain walk reads under one guard so the revisions it
    // returns are a consistent snapshot rather than a stitched-together one.
    #[allow(clippy::significant_drop_tightening)]
    pub fn fact_history(&self, id: FactId) -> Result<Vec<Fact>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let start = load_fact(&conn, id)?.ok_or(Error::FactIdNotFound { id })?;

        // Backwards along `supersedes` to the root, then forwards along the
        // rows that supersede us. Each revision has exactly one predecessor
        // column, so the backward walk is linear by construction; the
        // forward walk can fan out in a hand-edited or externally-written
        // file, and surfaces that as `AmbiguousFactRevision` rather than
        // guessing. `seen` guards against a cycle either way.
        let mut seen = vec![start.id];
        let mut older = Vec::new();
        let mut cursor = start.supersedes;
        while let Some(prev) = cursor {
            if seen.contains(&prev) {
                break;
            }
            let fact = load_fact(&conn, prev)?.ok_or(Error::FactIdNotFound { id: prev })?;
            seen.push(fact.id);
            cursor = fact.supersedes;
            older.push(fact);
        }
        older.reverse();

        let mut chain = older;
        chain.push(start);
        loop {
            let last = chain.last().expect("chain always holds the start fact").id;
            let successors = successors_of(&conn, last)?;
            let next = match successors.as_slice() {
                [] => break,
                [only] => *only,
                _ => {
                    return Err(Error::AmbiguousFactRevision {
                        candidates: successors,
                    })
                }
            };
            if seen.contains(&next) {
                break;
            }
            seen.push(next);
            chain.push(load_fact(&conn, next)?.ok_or(Error::FactIdNotFound { id: next })?);
        }
        Ok(chain)
    }

    /// One fact revision by id.
    ///
    /// # Errors
    /// `Error::FactIdNotFound` if `id` is unknown; `Error::Sqlite` on
    /// database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn get_fact(&self, id: FactId) -> Result<Fact> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        load_fact(&conn, id)?.ok_or(Error::FactIdNotFound { id })
    }

    /// The entity with this name (normalised), if it exists. Entities are
    /// store-global, so there is at most one per normalised name.
    ///
    /// # Errors
    /// `Error::Validation { field: "entity" }` if `name` does not normalise;
    /// `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn get_entity(&self, name: &str) -> Result<Option<Entity>> {
        let normalised = normalise::entity_name(name)?;
        let conn = self.conn.lock().expect("store mutex poisoned");
        let row = conn
            .query_row(
                "SELECT id, name, normalised_name, kind, created_at FROM entities \
                 WHERE normalised_name = ?1",
                rusqlite::params![normalised],
                read_entity,
            )
            .optional()
            .map_err(|e| Error::Sqlite {
                context: "reading entity",
                source: e,
            })?;
        drop(conn);
        row.map(RawEntity::into_entity).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::{fact_where, placeholders, scope_only, Validity};
    use crate::graph::types::GraphQuery;

    /// The default read sees head rows that are still open, and binds
    /// nothing: the two most common fragments carry no parameters.
    #[test]
    fn default_clause_is_open_heads() {
        let (clause, binds) = fact_where(&GraphQuery::default(), Validity::Window);
        assert!(clause.contains("NOT EXISTS"), "{clause}");
        assert!(clause.contains("f.valid_to IS NULL"), "{clause}");
        assert!(binds.is_empty());
    }

    /// `recorded_at` and `as_of` each bind their instant twice, and the
    /// scope filter is rebound from `scope` to `f.scope`.
    #[test]
    fn recorded_at_and_as_of_bind_twice() {
        let q = GraphQuery {
            as_of: Some(crate::graph::time::parse_point("2026-08-01").unwrap()),
            recorded_at: Some(crate::graph::time::parse_point("2026-07-01").unwrap()),
            scope: Some(crate::scope::ScopeFilter::descendants("a/b").unwrap()),
            ..GraphQuery::default()
        };
        let (clause, binds) = fact_where(&q, Validity::Window);
        assert_eq!(binds.len(), 6, "{binds:?}");
        assert_eq!(binds[0], binds[1]);
        assert_eq!(binds[2], binds[3]);
        assert!(clause.contains("f.scope"), "{clause}");
        assert!(!clause.contains("(scope ="), "{clause}");
        assert_eq!(clause.matches('?').count(), 6, "{clause}");
    }

    /// `AnyHead` drops the validity window so closed heads stay visible.
    #[test]
    fn any_head_keeps_closed_revisions() {
        let (clause, _) = fact_where(&GraphQuery::default(), Validity::AnyHead);
        assert!(!clause.contains("valid_to"), "{clause}");
    }

    #[test]
    fn placeholder_lists_and_scope_only() {
        assert_eq!(placeholders(1), "?");
        assert_eq!(placeholders(3), "?, ?, ?");
        assert!(scope_only(None).scope.is_none());
    }
}
