//! `Store::ingest` and `Store::ingest_many`.

use jiff::Timestamp;
use rusqlite::params;
use ulid::Ulid;

use crate::error::{Error, Result};
use crate::item::{validate, Item, ItemId, NewItem};
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
        let normalised_tags = validate(&item)?;

        // Generate ID + timestamp using injected clock+rng.
        let now = self.clock.now();
        let id = mint_ulid(self, now)?;

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

        // Serialise metadata once.
        let metadata_text = serde_json::to_string(&item.metadata).map_err(|e| Error::Json {
            context: "serialising item metadata",
            source: e,
        })?;
        let created_at_text = now.to_string();
        let id_text = id.to_string();

        tx.execute(
            "INSERT INTO items (id, content, created_at, supersedes, source, metadata, external_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id_text,
                item.content,
                created_at_text,
                item.supersedes.map(|i| i.to_string()),
                item.source,
                metadata_text,
                item.external_id,
            ],
        )
        .map_err(|e| map_insert_err(e, item.external_id.as_deref(), "inserting item row"))?;

        for tag in &normalised_tags {
            tx.execute(
                "INSERT INTO item_tags (item_id, tag) VALUES (?1, ?2)",
                params![id_text, tag],
            )
            .map_err(|e| Error::Sqlite {
                context: "inserting item tag",
                source: e,
            })?;
        }

        tx.commit().map_err(|e| Error::Sqlite {
            context: "committing ingest transaction",
            source: e,
        })?;

        // Invoke the IndexHook if one is attached. Per Principle VII,
        // hook failures DO NOT roll back the SQLite write — the item is
        // durably stored, and the hook implementation is expected to log
        // a warning naming the item ID so the user can recover via
        // `singularmem reindex`.
        if let Some(hook) = self
            .hook
            .lock()
            .expect("store hook mutex poisoned")
            .as_ref()
        {
            let item_for_hook = Item {
                id,
                content: item.content.clone(),
                created_at: now,
                supersedes: item.supersedes,
                tags: normalised_tags.clone(),
                source: item.source.clone(),
                metadata: item.metadata.clone(),
                external_id: item.external_id.clone(),
            };
            if let Err(e) = hook.on_ingest(&item_for_hook) {
                tracing::warn!(
                    item_id = %id,
                    error = %e,
                    "IndexHook::on_ingest failed; item is durably stored in SQLite but un-searchable. Run `singularmem reindex` to recover."
                );
            } else if let Err(e) = hook.commit() {
                tracing::warn!(
                    item_id = %id,
                    error = %e,
                    "IndexHook::commit failed after on_ingest; item may or may not be searchable until next commit succeeds. Run `singularmem reindex` to be sure."
                );
            }
        }

        Ok(Item {
            id,
            content: item.content,
            created_at: now,
            supersedes: item.supersedes,
            tags: normalised_tags,
            source: item.source,
            metadata: item.metadata,
            external_id: item.external_id,
        })
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
        let mut normalised_tag_lists = Vec::with_capacity(items.len());
        for item in &items {
            normalised_tag_lists.push(validate(item)?);
        }

        let now = self.clock.now();
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction().map_err(|e| Error::Sqlite {
            context: "starting bulk ingest transaction",
            source: e,
        })?;

        let mut out = Vec::with_capacity(items.len());

        for (item, normalised_tags) in items.into_iter().zip(normalised_tag_lists) {
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
            let id = mint_ulid(self, now)?;
            let id_text = id.to_string();
            let metadata_text = serde_json::to_string(&item.metadata).map_err(|e| Error::Json {
                context: "serialising metadata in bulk",
                source: e,
            })?;
            let created_at_text = now.to_string();

            tx.execute(
                "INSERT INTO items (id, content, created_at, supersedes, source, metadata, external_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id_text,
                    item.content,
                    created_at_text,
                    item.supersedes.map(|i| i.to_string()),
                    item.source,
                    metadata_text,
                    item.external_id,
                ],
            )
            .map_err(|e| map_insert_err(e, item.external_id.as_deref(), "inserting bulk item row"))?;

            for tag in &normalised_tags {
                tx.execute(
                    "INSERT INTO item_tags (item_id, tag) VALUES (?1, ?2)",
                    params![id_text, tag],
                )
                .map_err(|e| Error::Sqlite {
                    context: "inserting bulk item tag",
                    source: e,
                })?;
            }

            out.push(Item {
                id,
                content: item.content,
                created_at: now,
                supersedes: item.supersedes,
                tags: normalised_tags,
                source: item.source,
                metadata: item.metadata,
                external_id: item.external_id,
            });
        }

        tx.commit().map_err(|e| Error::Sqlite {
            context: "committing bulk ingest transaction",
            source: e,
        })?;

        // Hook integration: per-item on_ingest, then ONE commit at the end.
        if let Some(hook) = self
            .hook
            .lock()
            .expect("store hook mutex poisoned")
            .as_ref()
        {
            for item in &out {
                if let Err(e) = hook.on_ingest(item) {
                    tracing::warn!(
                        item_id = %item.id,
                        error = %e,
                        "IndexHook::on_ingest failed during bulk ingest; item is durably stored but un-searchable. Run `singularmem reindex` to recover."
                    );
                }
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
/// inside a loop without re-entering the `impl Store` borrow.
fn mint_ulid(store: &Store, now: Timestamp) -> Result<ItemId> {
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
    Ok(ItemId::from_ulid(Ulid::from_parts(ms, random)))
}
