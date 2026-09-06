//! `Store::ingest` and `Store::ingest_many`.

use jiff::Timestamp;
use rusqlite::params;
use ulid::Ulid;

use crate::error::{Error, Result};
use crate::item::{validate, Item, ItemId, NewItem, Validated};
use crate::store::Store;

impl Store {
    /// Validate and persist a new memory item. Assigns ID + `created_at`.
    /// Returns the persisted `Item`.
    ///
    /// # Errors
    ///
    /// Returns `Error::Validation` if the item fails any rule (empty or
    /// oversized content, oversized source, non-object metadata, oversized or
    /// NUL-bearing tags, invalid `external_id`); `Error::SupersedesNotFound`
    /// if `supersedes` is set to an unknown ID; `Error::ExternalIdConflict`
    /// if `external_id` collides with an existing item's; `Error::Sqlite` on
    /// database error; `Error::ReadOnly` if the store was opened read-only.
    ///
    /// # Panics
    ///
    /// Panics if the internal connection `Mutex` is poisoned (i.e. another
    /// thread panicked while holding the lock).
    #[allow(clippy::significant_drop_tightening, clippy::too_many_lines)]
    pub fn ingest(&self, item: NewItem) -> Result<Item> {
        self.assert_writable("ingest")?;

        // Validate up front (no SQL touched if invalid).
        let Validated {
            tags: normalised_tags,
            scope,
        } = validate(&item)?;

        // Generate ID + timestamp using injected clock+rng.
        let now = self.clock.now();
        let id = ItemId::from_ulid(mint_raw_ulid(self, now)?);

        // Write under a single transaction.
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(|e| Error::Sqlite {
            context: "starting ingest transaction",
            source: e,
        })?;

        // Verify supersedes target exists, if any.
        if let Some(target) = item.supersedes {
            let exists: i64 = tx
                .query_row(
                    "SELECT 1 FROM items WHERE id = ?1",
                    params![target.to_string()],
                    |r| r.get(0),
                )
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(0),
                    other => Err(Error::Sqlite {
                        context: "checking supersedes target existence",
                        source: other,
                    }),
                })?;
            if exists == 0 {
                tx.rollback().map_err(|e| Error::Sqlite {
                    context: "rolling back after SupersedesNotFound",
                    source: e,
                })?;
                return Err(Error::SupersedesNotFound { id: target });
            }
        }

        insert_item_row(&tx, id, now, &item, &normalised_tags, scope.as_deref())?;

        tx.commit().map_err(|e| Error::Sqlite {
            context: "committing ingest transaction",
            source: e,
        })?;
        drop(conn);

        let stored = Item {
            id,
            content: item.content,
            created_at: now,
            supersedes: item.supersedes,
            tags: normalised_tags,
            source: item.source,
            metadata: item.metadata,
            external_id: item.external_id,
            scope,
        };
        self.fire_hook(&stored);
        Ok(stored)
    }

    /// Bulk variant of `ingest`. All items persist or none do.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Store::ingest`]. On any failure mid-batch,
    /// the entire transaction is rolled back; no items from this call persist.
    ///
    /// # Panics
    ///
    /// Panics if the internal connection `Mutex` is poisoned (i.e. another
    /// thread panicked while holding the lock).
    #[allow(clippy::significant_drop_tightening, clippy::too_many_lines)]
    pub fn ingest_many<I: IntoIterator<Item = NewItem>>(&self, items: I) -> Result<Vec<Item>> {
        self.assert_writable("ingest_many")?;

        // Materialise + validate up front so we can fail before touching SQL.
        let items: Vec<NewItem> = items.into_iter().collect();
        let mut validated = Vec::with_capacity(items.len());
        for item in &items {
            validated.push(validate(item)?);
        }

        let now = self.clock.now();
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(|e| Error::Sqlite {
            context: "starting bulk ingest transaction",
            source: e,
        })?;

        let mut out = Vec::with_capacity(items.len());

        for (
            item,
            Validated {
                tags: normalised_tags,
                scope,
            },
        ) in items.into_iter().zip(validated)
        {
            // Verify supersedes target inside the same tx (so concurrent ingests
            // can be referenced by later items in the batch).
            if let Some(target) = item.supersedes {
                let exists: i64 = tx
                    .query_row(
                        "SELECT 1 FROM items WHERE id = ?1",
                        params![target.to_string()],
                        |r| r.get(0),
                    )
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(0),
                        other => Err(Error::Sqlite {
                            context: "checking supersedes target in bulk",
                            source: other,
                        }),
                    })?;
                if exists == 0 {
                    return Err(Error::SupersedesNotFound { id: target });
                }
            }

            // Generate a new ULID per item; all share the wall-clock instant
            // captured at the start of the batch but differ in random bytes.
            let id = ItemId::from_ulid(mint_raw_ulid(self, now)?);
            insert_item_row(&tx, id, now, &item, &normalised_tags, scope.as_deref())?;

            out.push(Item {
                id,
                content: item.content,
                created_at: now,
                supersedes: item.supersedes,
                tags: normalised_tags,
                source: item.source,
                metadata: item.metadata,
                external_id: item.external_id,
                scope,
            });
        }

        tx.commit().map_err(|e| Error::Sqlite {
            context: "committing bulk ingest transaction",
            source: e,
        })?;

        // Hook integration: one on_ingest_batch call, then ONE commit at the end.
        if let Some(hook) = self
            .hook
            .lock()
            .expect("store hook mutex poisoned")
            .as_ref()
        {
            if let Err(e) = hook.on_ingest_batch(&out) {
                tracing::warn!(
                    items = out.len(),
                    error = %e,
                    "IndexHook::on_ingest_batch failed during bulk ingest; items are durably stored but some may be un-searchable. Run `singularmem reindex` to recover."
                );
            }
            if let Err(e) = hook.commit() {
                tracing::warn!(
                    error = %e,
                    "IndexHook::commit failed after bulk ingest; items may or may not be searchable until next commit succeeds. Run `singularmem reindex` to be sure."
                );
            }
        }

        Ok(out)
    }

    /// Ingest `item` as the successor of `replaces`, transferring
    /// `item.external_id` from the old item to the new one in the same
    /// transaction. This is the only in-place mutation the store performs
    /// (`items.external_id` on the old row is set to NULL); see
    /// `docs/formats/store-v2.md`.
    ///
    /// `item.supersedes` is overwritten with `replaces`.
    ///
    /// The replaced row's `external_id` is cleared unconditionally — even
    /// when `item` carries none — so an id never survives on a superseded
    /// row.
    ///
    /// # Errors
    /// `Error::ReadOnly`, `Error::Validation`, `Error::SupersedesNotFound`
    /// if `replaces` is unknown, `Error::ExternalIdConflict` if the id is
    /// held by a third item, `Error::Sqlite` otherwise. On any error
    /// nothing changes.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    #[allow(clippy::significant_drop_tightening)]
    pub fn ingest_replacing(&self, mut item: NewItem, replaces: ItemId) -> Result<Item> {
        self.assert_writable("ingest_replacing")?;
        item.supersedes = Some(replaces);
        let Validated {
            tags: normalised_tags,
            scope,
        } = validate(&item)?;
        let now = self.clock.now();
        let id = ItemId::from_ulid(mint_raw_ulid(self, now)?);

        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(|e| Error::Sqlite {
            context: "starting ingest_replacing transaction",
            source: e,
        })?;

        let freed = tx
            .execute(
                "UPDATE items SET external_id = NULL WHERE id = ?1",
                params![replaces.to_string()],
            )
            .map_err(|e| Error::Sqlite {
                context: "clearing external_id on replaced item",
                source: e,
            })?;
        if freed == 0 {
            return Err(Error::SupersedesNotFound { id: replaces });
        }

        insert_item_row(&tx, id, now, &item, &normalised_tags, scope.as_deref())?;

        tx.commit().map_err(|e| Error::Sqlite {
            context: "committing ingest_replacing transaction",
            source: e,
        })?;
        drop(conn);

        let stored = Item {
            id,
            content: item.content,
            created_at: now,
            supersedes: Some(replaces),
            tags: normalised_tags,
            source: item.source,
            metadata: item.metadata,
            external_id: item.external_id,
            scope,
        };
        self.fire_hook(&stored);
        Ok(stored)
    }

    /// Move an item to `scope` (or clear it with `None`) without creating a
    /// revision. This is the store's second sanctioned in-place mutation
    /// (`docs/formats/store-v4.md`). Index hooks are NOT notified: the Tantivy
    /// document keeps its old scope until `singularmem reindex`, which the
    /// CLI `scope move` verb documents.
    ///
    /// # Errors
    /// `Error::ReadOnly`, `Error::Validation { field: "scope" }`,
    /// `Error::NotFound`, `Error::Sqlite`.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn set_scope(&self, id: ItemId, scope: Option<&str>) -> Result<Item> {
        self.assert_writable("set_scope")?;
        let normalised = scope.map(crate::scope::validate).transpose()?;
        let conn = self.conn.lock().expect("store mutex poisoned");
        let changed = conn
            .execute(
                "UPDATE items SET scope = ?1 WHERE id = ?2",
                params![normalised, id.to_string()],
            )
            .map_err(|e| Error::Sqlite {
                context: "updating scope",
                source: e,
            })?;
        drop(conn);
        if changed == 0 {
            return Err(Error::NotFound { id });
        }
        self.get(id)
    }

    /// Run `on_ingest` + `commit` on the attached hook, warning on failure
    /// (the `SQLite` write is already durable; Principle VII).
    fn fire_hook(&self, item: &Item) {
        if let Some(hook) = self
            .hook
            .lock()
            .expect("store hook mutex poisoned")
            .as_ref()
        {
            if let Err(e) = hook.on_ingest(item) {
                tracing::warn!(item_id = %item.id, error = %e,
                    "IndexHook::on_ingest failed; item is durably stored in SQLite but un-searchable. Run `singularmem reindex` to recover.");
            } else if let Err(e) = hook.commit() {
                tracing::warn!(item_id = %item.id, error = %e,
                    "IndexHook::commit failed after on_ingest; item may or may not be searchable until next commit succeeds. Run `singularmem reindex` to be sure.");
            }
        }
    }
}

/// Insert one item row and its tags inside `tx`. Shared by `ingest`,
/// `ingest_many`, and `ingest_replacing`; behaviour (columns, error mapping)
/// is identical across all three call sites.
///
/// `scope` is the already-validated, normalised scope path (or `None`) —
/// callers pass the value produced by [`crate::item::validate`], not
/// `item.scope` directly.
///
/// # Errors
/// `Error::Json` if metadata serialisation fails; `Error::ExternalIdConflict`
/// if `item.external_id` collides with an existing row; `Error::Sqlite`
/// otherwise.
fn insert_item_row(
    tx: &rusqlite::Transaction<'_>,
    id: ItemId,
    now: Timestamp,
    item: &NewItem,
    tags: &[String],
    scope: Option<&str>,
) -> Result<()> {
    let metadata_text = serde_json::to_string(&item.metadata).map_err(|e| Error::Json {
        context: "serialising item metadata",
        source: e,
    })?;
    let id_text = id.to_string();
    tx.execute(
        "INSERT INTO items (id, content, created_at, supersedes, source, metadata, external_id, scope) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id_text,
            item.content,
            now.to_string(),
            item.supersedes.map(|i| i.to_string()),
            item.source,
            metadata_text,
            item.external_id,
            scope,
        ],
    )
    .map_err(|e| map_insert_err(e, item.external_id.as_deref(), "inserting item row"))?;

    for tag in tags {
        tx.execute(
            "INSERT INTO item_tags (item_id, tag) VALUES (?1, ?2)",
            params![id_text, tag],
        )
        .map_err(|e| Error::Sqlite {
            context: "inserting item tag",
            source: e,
        })?;
    }
    Ok(())
}

/// Map an INSERT error: a unique violation on `external_id` becomes
/// `ExternalIdConflict`; anything else is `Sqlite`.
fn map_insert_err(e: rusqlite::Error, external_id: Option<&str>, context: &'static str) -> Error {
    if let rusqlite::Error::SqliteFailure(ffi, Some(ref msg)) = e {
        if ffi.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
            && msg.contains("items.external_id")
        {
            return Error::ExternalIdConflict {
                external_id: external_id.unwrap_or_default().to_string(),
            };
        }
    }
    Error::Sqlite { context, source: e }
}

/// Mint a fresh ULID using the store's injected rng and the given timestamp.
///
/// This is a free function (not a method) so that `ingest_many` can call it
/// inside a loop without re-entering the `impl Store` borrow. Returns the raw
/// `Ulid` rather than an `ItemId` so it can back any of the store's ULID
/// newtypes (`ItemId`, `EntityId`, `FactId`).
///
/// `pub` rather than `pub(crate)`: `ingest` is a private module, so the two
/// are equally crate-bounded here and clippy's `redundant_pub_crate` prefers
/// the plain form.
///
/// # Errors
/// `Error::Validation` if the current wall-clock predates 1970-01-01.
pub fn mint_raw_ulid(store: &Store, now: Timestamp) -> Result<Ulid> {
    // ulid::Ulid::from_parts takes (timestamp_ms_u64, random_u128).
    let ms = u64::try_from(now.as_millisecond()).map_err(|_| Error::Validation {
        field: "internal:timestamp",
        reason: "current wall-clock predates 1970-01-01".to_string(),
    })?;
    let mut random_bytes = [0u8; 16];
    {
        let mut rng = store.rng.lock().expect("rng mutex poisoned");
        rng.fill_bytes(&mut random_bytes);
    }
    let random = u128::from_be_bytes(random_bytes);
    Ok(Ulid::from_parts(ms, random))
}
