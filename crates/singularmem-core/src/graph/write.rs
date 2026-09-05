//! Graph write operations on [`Store`]: `add_fact`, `invalidate_fact`, and
//! `supersede_fact`.
//!
//! The graph is append-only. Invalidating or superseding a fact never
//! touches the stored row: it appends a new revision that points at the old
//! one through `supersedes`, so both time axes stay answerable. Spec:
//! `docs/superpowers/specs/2026-09-05-knowledge-graph-14-design.md`
//! § "Revisions and the two time axes" and § "Operations".

use jiff::Timestamp;
use rusqlite::{params, OptionalExtension, Transaction};

use crate::error::{Error, Result};
use crate::graph::normalise;
use crate::graph::read::load_fact;
use crate::graph::time::to_sql;
use crate::graph::types::{EntityRef, Fact, FactObject, NewFact, NewObject};
use crate::id::{EntityId, FactId};
use crate::ingest::mint_raw_ulid;
use crate::scope;
use crate::store::Store;

/// The triple an invalidate or supersede addresses, as the caller wrote it.
/// Grouped so the in-transaction helpers keep a readable argument list.
struct TripleRef<'a> {
    subject: &'a str,
    predicate: &'a str,
    object: &'a NewObject,
    scope: Option<&'a str>,
}

impl TripleRef<'_> {
    /// The object as text, for error messages.
    fn object_text(&self) -> String {
        match self.object {
            NewObject::Entity { name, .. } => name.clone(),
            NewObject::Value(v) => v.clone(),
        }
    }

    /// `FactNotFound` naming this triple.
    fn not_found(&self) -> Error {
        Error::FactNotFound {
            subject: self.subject.to_string(),
            predicate: self.predicate.to_string(),
            object: self.object_text(),
        }
    }
}

/// The object side of a fact as the `facts` table stores it: exactly one of
/// `object_id` / `object_value` is set (a `CHECK` enforces it).
struct ObjectColumns {
    id: Option<String>,
    value: Option<String>,
}

impl ObjectColumns {
    /// Split a resolved [`FactObject`] into its two columns.
    fn of(object: &FactObject) -> Self {
        match object {
            FactObject::Entity(e) => Self {
                id: Some(e.id.to_string()),
                value: None,
            },
            FactObject::Value(v) => Self {
                id: None,
                value: Some(v.clone()),
            },
        }
    }
}

/// The id of the open head for a triple, if one exists.
///
/// "Open head" is the spec's idempotency and invalidation target: a row with
/// no `valid_to` that nothing supersedes, matching subject, predicate,
/// object, and scope exactly.
fn find_open_head(
    tx: &Transaction<'_>,
    subject_id: &str,
    predicate: &str,
    object: &ObjectColumns,
    scope: Option<&str>,
) -> Result<Option<FactId>> {
    let found: Option<String> = tx
        .query_row(
            "SELECT f.id FROM facts f \
             WHERE f.subject_id = ?1 AND f.predicate = ?2 \
               AND IFNULL(f.object_id, '') = IFNULL(?3, '') \
               AND IFNULL(f.object_value, '') = IFNULL(?4, '') \
               AND IFNULL(f.scope, '') = IFNULL(?5, '') \
               AND f.valid_to IS NULL \
               AND NOT EXISTS (SELECT 1 FROM facts g WHERE g.supersedes = f.id) \
             LIMIT 1",
            params![subject_id, predicate, object.id, object.value, scope],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| Error::Sqlite {
            context: "looking for an open fact head",
            source: e,
        })?;
    found
        .map(|s| s.parse::<FactId>())
        .transpose()
        .map_err(Into::into)
}

/// Insert one fully-formed revision. The only `INSERT` into `facts` in the
/// crate, so `add` and `invalidate` cannot drift apart.
fn insert_fact_row(tx: &Transaction<'_>, fact: &Fact) -> Result<()> {
    let object = ObjectColumns::of(&fact.object);
    tx.execute(
        "INSERT INTO facts \
         (id, subject_id, predicate, object_id, object_value, valid_from, valid_to, \
          confidence, source_item_id, scope, supersedes, recorded_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            fact.id.to_string(),
            fact.subject.id.to_string(),
            fact.predicate,
            object.id,
            object.value,
            fact.valid_from.map(to_sql),
            fact.valid_to.map(to_sql),
            f64::from(fact.confidence),
            fact.source_item_id.map(|i| i.to_string()),
            fact.scope,
            fact.supersedes.map(|i| i.to_string()),
            to_sql(fact.recorded_at),
        ],
    )
    .map_err(|e| Error::Sqlite {
        context: "inserting fact revision",
        source: e,
    })?;
    Ok(())
}

/// The id of an existing entity, without creating one. Entities are
/// store-global, so the lookup is by normalised name alone.
fn find_entity(tx: &Transaction<'_>, name: &str) -> Result<Option<EntityRef>> {
    let normalised = normalise::entity_name(name)?;
    let row: Option<(String, String)> = tx
        .query_row(
            "SELECT id, name FROM entities WHERE normalised_name = ?1",
            params![normalised],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| Error::Sqlite {
            context: "looking up entity",
            source: e,
        })?;
    match row {
        None => Ok(None),
        Some((id, display)) => Ok(Some(EntityRef {
            id: id.parse()?,
            name: display,
        })),
    }
}

/// A caller-supplied `kind`, trimmed, with a blank one treated as absent.
///
/// Storing `Some("")` for `"   "` would brick the entity: every later write
/// naming a real kind would fail the immutability check against `""`.
fn kind_or_none(kind: Option<&str>) -> Option<&str> {
    kind.map(str::trim).filter(|s| !s.is_empty())
}

/// Whether an existing open head already answers `request` exactly, or
/// diverges from it in a way `add` must not silently discard.
///
/// Only the validity window and confidence can differ here — subject,
/// predicate, object, and scope are what [`find_open_head`] matched on.
/// A request that omits `valid_from`/`valid_to` matches a head whose
/// column is `NULL`, and an omitted confidence is the default `1.0`.
fn head_matches_request(
    head: &Fact,
    request_window: (Option<Timestamp>, Option<Timestamp>),
    request_confidence: f32,
) -> bool {
    (head.valid_from, head.valid_to) == request_window
        && (head.confidence - request_confidence).abs() < f32::EPSILON
}

impl Store {
    /// Resolve or create an entity inside `tx`, returning its id and display
    /// name. Entities are store-global: identity is the normalised name
    /// alone. `kind` is set on first creation and on an entity that has
    /// none; a conflicting kind is a validation error (spec: kind is
    /// immutable once set).
    fn get_or_create_entity(
        &self,
        tx: &Transaction<'_>,
        now: Timestamp,
        name: &str,
        kind: Option<&str>,
    ) -> Result<EntityRef> {
        let normalised = normalise::entity_name(name)?;
        let existing: Option<(String, String, Option<String>)> = tx
            .query_row(
                "SELECT id, name, kind FROM entities WHERE normalised_name = ?1",
                params![normalised],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| Error::Sqlite {
                context: "looking up entity",
                source: e,
            })?;

        if let Some((id, display, existing_kind)) = existing {
            if let (Some(wanted), Some(held)) = (kind, existing_kind.as_deref()) {
                if wanted != held {
                    return Err(Error::Validation {
                        field: "kind",
                        reason: format!("entity {display:?} already has kind {held:?}"),
                    });
                }
            }
            if kind.is_some() && existing_kind.is_none() {
                tx.execute(
                    "UPDATE entities SET kind = ?1 WHERE id = ?2",
                    params![kind, id],
                )
                .map_err(|e| Error::Sqlite {
                    context: "setting entity kind",
                    source: e,
                })?;
            }
            return Ok(EntityRef {
                id: id.parse()?,
                name: display,
            });
        }

        let id = EntityId::from_ulid(mint_raw_ulid(self, now)?);
        let display = name.trim().to_string();
        tx.execute(
            "INSERT INTO entities (id, name, normalised_name, kind, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.to_string(), display, normalised, kind, to_sql(now)],
        )
        .map_err(|e| Error::Sqlite {
            context: "inserting entity",
            source: e,
        })?;
        Ok(EntityRef { id, name: display })
    }

    /// Validate, resolve entities, and append one fact revision inside `tx`.
    /// Shared by [`Store::add_fact`] and the second half of
    /// [`Store::supersede_fact`].
    ///
    /// Entities are resolved in the store-wide (unscoped) namespace: a
    /// fact's scope scopes the fact, not the things it talks about, so two
    /// facts in sibling scopes refer to the same `tantivy`.
    fn add_fact_in_tx(&self, tx: &Transaction<'_>, now: Timestamp, f: NewFact) -> Result<Fact> {
        let predicate = normalise::predicate(&f.predicate)?;
        if !(0.0..=1.0).contains(&f.confidence) {
            return Err(Error::Validation {
                field: "confidence",
                reason: format!("{} is outside [0.0, 1.0]", f.confidence),
            });
        }
        if let (Some(from), Some(to)) = (f.valid_from, f.valid_to) {
            if to < from {
                return Err(Error::Validation {
                    field: "valid_window",
                    reason: format!("valid_to {to} precedes valid_from {from}"),
                });
            }
        }
        let scope = f.scope.as_deref().map(scope::validate).transpose()?;
        if let Some(source) = f.source_item_id {
            let exists = tx
                .query_row(
                    "SELECT 1 FROM items WHERE id = ?1",
                    params![source.to_string()],
                    |r| r.get::<_, i64>(0),
                )
                .optional()
                .map_err(|e| Error::Sqlite {
                    context: "checking fact source item",
                    source: e,
                })?
                .is_some();
            if !exists {
                return Err(Error::Validation {
                    field: "source_item_id",
                    reason: format!("item {source} does not exist in this store"),
                });
            }
        }

        // A whitespace-only kind is no kind at all: trimming alone would
        // store `""`, which then collides with every later real kind and
        // leaves the entity permanently unusable.
        let subject_kind = kind_or_none(f.subject_kind.as_deref());
        let subject = self.get_or_create_entity(tx, now, &f.subject, subject_kind)?;
        let object = match f.object {
            NewObject::Entity { name, kind } => {
                let kind = kind_or_none(kind.as_deref());
                FactObject::Entity(self.get_or_create_entity(tx, now, &name, kind)?)
            }
            NewObject::Value(value) => {
                // Literal values are stored trimmed so the surrounding
                // whitespace of one write cannot fork the triple's identity
                // from another's — `invalidate` trims the same way.
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err(Error::Validation {
                        field: "object",
                        reason: "value must be non-empty".to_string(),
                    });
                }
                FactObject::Value(trimmed.to_string())
            }
        };

        let columns = ObjectColumns::of(&object);
        if let Some(open) = find_open_head(
            tx,
            &subject.id.to_string(),
            &predicate,
            &columns,
            scope.as_deref(),
        )? {
            let head = load_fact(tx, open)?.ok_or(Error::FactIdNotFound { id: open })?;
            if !head_matches_request(&head, (f.valid_from, f.valid_to), f.confidence) {
                // The triple is the same but the request carries a
                // different window or confidence. Returning the head would
                // silently drop what the caller asked for; appending a
                // second open head would fork the chain. Neither is `add`'s
                // job — say so instead.
                return Err(Error::Validation {
                    field: "fact",
                    reason: format!(
                        "an open fact <{} —{}→ {}> already exists with a different \
                         validity window or confidence; use supersede or invalidate",
                        head.subject.name,
                        head.predicate,
                        match &head.object {
                            FactObject::Entity(e) => e.name.clone(),
                            FactObject::Value(v) => v.clone(),
                        }
                    ),
                });
            }
            // Idempotent: the same open fact already stands, so return it
            // rather than appending a duplicate revision.
            return Ok(head);
        }

        let fact = Fact {
            id: FactId::from_ulid(mint_raw_ulid(self, now)?),
            subject,
            predicate,
            object,
            valid_from: f.valid_from,
            valid_to: f.valid_to,
            confidence: f.confidence,
            source_item_id: f.source_item_id,
            scope,
            supersedes: None,
            recorded_at: now,
        };
        insert_fact_row(tx, &fact)?;
        Ok(fact)
    }

    /// Close the open head for `triple` by appending a revision with
    /// `valid_to = at`. Shared by [`Store::invalidate_fact`] and the first
    /// half of [`Store::supersede_fact`].
    ///
    /// Entities are resolved without being created: an unknown subject or
    /// object means there is no fact to close, which is `FactNotFound`.
    fn invalidate_in_tx(
        &self,
        tx: &Transaction<'_>,
        now: Timestamp,
        at: Timestamp,
        triple: &TripleRef<'_>,
    ) -> Result<Fact> {
        let predicate = normalise::predicate(triple.predicate)?;
        let scope = triple.scope.map(scope::validate).transpose()?;
        let Some(subject) = find_entity(tx, triple.subject)? else {
            return Err(triple.not_found());
        };
        let columns = match triple.object {
            NewObject::Entity { name, .. } => match find_entity(tx, name)? {
                None => return Err(triple.not_found()),
                Some(e) => ObjectColumns {
                    id: Some(e.id.to_string()),
                    value: None,
                },
            },
            // Matched against the trimmed form `add_fact_in_tx` stored.
            NewObject::Value(value) => ObjectColumns {
                id: None,
                value: Some(value.trim().to_string()),
            },
        };

        let Some(head_id) = find_open_head(
            tx,
            &subject.id.to_string(),
            &predicate,
            &columns,
            scope.as_deref(),
        )?
        else {
            return Err(triple.not_found());
        };
        let head = load_fact(tx, head_id)?.ok_or(Error::FactIdNotFound { id: head_id })?;
        if head.valid_from.is_some_and(|from| at < from) {
            return Err(Error::Validation {
                field: "valid_window",
                reason: format!(
                    "cannot end a fact at {at}, before it became valid at {}",
                    head.valid_from.unwrap_or(at)
                ),
            });
        }

        let closed = Fact {
            id: FactId::from_ulid(mint_raw_ulid(self, now)?),
            valid_to: Some(at),
            supersedes: Some(head.id),
            recorded_at: now,
            ..head
        };
        insert_fact_row(tx, &closed)?;
        Ok(closed)
    }

    /// Record a fact, creating its entities on demand.
    ///
    /// Idempotent: if an identical open fact (same subject, predicate,
    /// object, and scope) already stands with the same validity window and
    /// confidence, it is returned unchanged and nothing is written. If the
    /// triple matches but the window or confidence differs, the request is
    /// rejected rather than quietly discarded — closing or replacing a
    /// standing fact is `invalidate_fact`/`supersede_fact`'s job.
    ///
    /// # Errors
    /// `Error::ReadOnly` on a read-only store; `Error::Validation` for a
    /// bad entity name, predicate, `confidence`, `valid_window`, `scope`,
    /// `kind`, unknown `source_item_id`, or (field `"fact"`) an open head
    /// that diverges from the request; `Error::Sqlite` on database error.
    /// On any error nothing is written.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    // The transaction borrows the guard, so the guard must outlive the
    // commit; there is no tighter scope to drop it in.
    #[allow(clippy::significant_drop_tightening)]
    pub fn add_fact(&self, f: NewFact) -> Result<Fact> {
        self.assert_writable("add_fact")?;
        let now = self.clock.now();
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(|e| Error::Sqlite {
            context: "starting add_fact transaction",
            source: e,
        })?;
        let fact = self.add_fact_in_tx(&tx, now, f)?;
        tx.commit().map_err(|e| Error::Sqlite {
            context: "committing add_fact transaction",
            source: e,
        })?;
        Ok(fact)
    }

    /// End the open fact `subject —predicate→ object` at `at` (default:
    /// now) by appending a closing revision. The original row is never
    /// modified.
    ///
    /// # Errors
    /// `Error::ReadOnly` on a read-only store; `Error::FactNotFound` if no
    /// open head matches; `Error::Validation { field: "valid_window" }` if
    /// `at` precedes the fact's `valid_from`; `Error::Sqlite` on database
    /// error. On any error nothing is written.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    #[allow(clippy::significant_drop_tightening)]
    pub fn invalidate_fact(
        &self,
        subject: &str,
        predicate: &str,
        object: &NewObject,
        scope: Option<&str>,
        at: Option<Timestamp>,
    ) -> Result<Fact> {
        self.assert_writable("invalidate_fact")?;
        let now = self.clock.now();
        let at = at.unwrap_or(now);
        let triple = TripleRef {
            subject,
            predicate,
            object,
            scope,
        };
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(|e| Error::Sqlite {
            context: "starting invalidate_fact transaction",
            source: e,
        })?;
        let closed = self.invalidate_in_tx(&tx, now, at, &triple)?;
        tx.commit().map_err(|e| Error::Sqlite {
            context: "committing invalidate_fact transaction",
            source: e,
        })?;
        Ok(closed)
    }

    /// Replace one fact with another in a single transaction: close the old
    /// fact at `at` (default: now) and open the new one from `at`.
    ///
    /// A missing old fact is tolerated — it is logged and reported as
    /// `Ok((None, new))` — but any other failure rolls the whole thing back,
    /// so the old fact is never left closed without its replacement.
    ///
    /// # Errors
    /// `Error::ReadOnly` on a read-only store; anything
    /// [`Store::add_fact`] can return for the new fact; `Error::Validation`
    /// or `Error::Sqlite` from closing the old one.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    #[allow(clippy::significant_drop_tightening)]
    pub fn supersede_fact(
        &self,
        subject: &str,
        predicate: &str,
        old: &NewObject,
        new: NewObject,
        scope: Option<&str>,
        at: Option<Timestamp>,
    ) -> Result<(Option<Fact>, Fact)> {
        self.assert_writable("supersede_fact")?;
        let now = self.clock.now();
        let at = at.unwrap_or(now);
        let triple = TripleRef {
            subject,
            predicate,
            object: old,
            scope,
        };

        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(|e| Error::Sqlite {
            context: "starting supersede_fact transaction",
            source: e,
        })?;

        let closed = match self.invalidate_in_tx(&tx, now, at, &triple) {
            Ok(fact) => Some(fact),
            Err(Error::FactNotFound { .. }) => {
                tracing::warn!(
                    subject, predicate,
                    object = %triple.object_text(),
                    "supersede found no open fact to close; recording the new fact only"
                );
                None
            }
            Err(other) => return Err(other),
        };

        let replacement = self
            .add_fact_in_tx(
                &tx,
                now,
                NewFact {
                    subject: subject.to_string(),
                    subject_kind: None,
                    predicate: predicate.to_string(),
                    object: new,
                    valid_from: Some(at),
                    valid_to: None,
                    confidence: 1.0,
                    source_item_id: None,
                    scope: scope.map(ToString::to_string),
                },
            )
            .map_err(|e| match e {
                // `add`'s divergence error tells the caller to supersede — which
                // is what they just did. Say what actually applies here.
                Error::Validation {
                    field: "fact",
                    reason,
                } => Error::Validation {
                    field: "fact",
                    reason: format!(
                        "{}; the replacement already stands, so supersede with the \
                     time point it opened at, or invalidate it first",
                        reason
                            .split("; use supersede or invalidate")
                            .next()
                            .unwrap_or(&reason)
                    ),
                },
                other => other,
            })?;

        tx.commit().map_err(|e| Error::Sqlite {
            context: "committing supersede_fact transaction",
            source: e,
        })?;
        Ok((closed, replacement))
    }
}
