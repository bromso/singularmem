//! `Store` read methods: `get`, `get_optional`, `list`, `list_by_tags`,
//! `revision_history`, `latest_revision`.

use std::collections::VecDeque;

use rusqlite::params;

use crate::error::{Error, Result};
use crate::item::{Item, ItemId};
use crate::store::Store;

/// Iterator over `Item`s, returned by `Store::list` and `Store::list_by_tags`.
///
/// IDs are fetched eagerly under a single lock acquisition; `Item` payloads
/// are fetched lazily on each `next()` call so callers iterating over a large
/// store don't materialise everything in memory at once.
pub struct ItemIter<'store> {
    store: &'store Store,
    pending_ids: VecDeque<ItemId>,
}

impl Iterator for ItemIter<'_> {
    type Item = Result<Item>;
    fn next(&mut self) -> Option<Self::Item> {
        let id = self.pending_ids.pop_front()?;
        Some(self.store.get(id))
    }
}

impl Store {
    /// Fetch a single item by ID.
    ///
    /// # Errors
    ///
    /// Returns `Error::NotFound` if no item with the given ID exists in the
    /// store; `Error::Sqlite` on database error.
    ///
    /// # Panics
    ///
    /// Panics if the internal connection `Mutex` is poisoned (i.e. another
    /// thread panicked while holding the lock).
    pub fn get(&self, id: ItemId) -> Result<Item> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        load_item(&conn, id)
    }

    /// Like `get`, but returns `Ok(None)` for a missing ID instead of
    /// `Err(Error::NotFound)`. Useful when the absence is not exceptional.
    ///
    /// # Errors
    ///
    /// Returns `Error::Sqlite` on database error. A missing item is `Ok(None)`.
    pub fn get_optional(&self, id: ItemId) -> Result<Option<Item>> {
        match self.get(id) {
            Ok(item) => Ok(Some(item)),
            Err(Error::NotFound { .. }) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// Iterate over every item in `created_at` ascending order.
    ///
    /// IDs are loaded eagerly; `Item` payloads load lazily as the iterator
    /// advances. Memory cost: O(IDs) — about 30 bytes per item — not O(items).
    ///
    /// # Errors
    ///
    /// Returns `Err` from the initial ID query if the database errors.
    /// Each iterator step may also return `Err` if a subsequent payload
    /// fetch fails.
    ///
    /// # Panics
    ///
    /// Panics if the internal connection `Mutex` is poisoned (i.e. another
    /// thread panicked while holding the lock).
    pub fn list(&self) -> Result<ItemIter<'_>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn
            .prepare("SELECT id FROM items ORDER BY created_at ASC")
            .map_err(|e| Error::Sqlite {
                context: "preparing list query",
                source: e,
            })?;
        let id_strings: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| Error::Sqlite {
                context: "executing list query",
                source: e,
            })?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| Error::Sqlite {
                context: "collecting list IDs",
                source: e,
            })?;
        drop(stmt);
        drop(conn);

        let pending_ids = id_strings
            .into_iter()
            .map(|s| s.parse::<ItemId>())
            .collect::<std::result::Result<VecDeque<_>, _>>()?;

        Ok(ItemIter {
            store: self,
            pending_ids,
        })
    }

    /// Iterate over items whose tag set contains every named tag (AND-semantics).
    /// An empty `tags` slice returns the same result as `list`.
    ///
    /// # Errors
    ///
    /// Same as `list`.
    ///
    /// # Panics
    ///
    /// Panics if the internal connection `Mutex` is poisoned (i.e. another
    /// thread panicked while holding the lock).
    pub fn list_by_tags(&self, tags: &[&str]) -> Result<ItemIter<'_>> {
        if tags.is_empty() {
            return self.list();
        }

        let conn = self.conn.lock().expect("store mutex poisoned");

        // Build IN-list placeholders for the tag values.
        let placeholders = tags
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let count_param = tags.len() + 1;
        let sql = format!(
            "SELECT i.id FROM items i \
             WHERE i.id IN ( \
                 SELECT item_id FROM item_tags \
                 WHERE tag IN ({placeholders}) \
                 GROUP BY item_id \
                 HAVING COUNT(DISTINCT tag) = ?{count_param} \
             ) \
             ORDER BY i.created_at ASC",
        );

        // Collect tag strings + count into a single params list.
        let tag_strings: Vec<String> = tags.iter().map(|t| (*t).to_string()).collect();
        let tag_count = i64::try_from(tags.len()).unwrap_or(i64::MAX);

        let mut stmt = conn.prepare(&sql).map_err(|e| Error::Sqlite {
            context: "preparing list_by_tags query",
            source: e,
        })?;
        let id_strings: Vec<String> = stmt
            .query_map(
                rusqlite::params_from_iter(
                    tag_strings.iter().map(|s| s as &dyn rusqlite::ToSql)
                        .chain(std::iter::once(&tag_count as &dyn rusqlite::ToSql)),
                ),
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| Error::Sqlite {
                context: "executing list_by_tags query",
                source: e,
            })?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| Error::Sqlite {
                context: "collecting list_by_tags IDs",
                source: e,
            })?;
        drop(stmt);
        drop(conn);

        let pending_ids = id_strings
            .into_iter()
            .map(|s| s.parse::<ItemId>())
            .collect::<std::result::Result<VecDeque<_>, _>>()?;

        Ok(ItemIter {
            store: self,
            pending_ids,
        })
    }
}

impl Store {
    /// Walk the supersedes chain from a starting item back to the original.
    /// Items returned newest-first; the starting item is included as
    /// `result[0]`.
    ///
    /// # Errors
    ///
    /// Returns `Error::NotFound` if the starting ID is not in the store.
    /// Returns `Error::Sqlite` on database errors.
    ///
    /// # Panics
    ///
    /// Panics if the internal connection `Mutex` is poisoned.
    pub fn revision_history(&self, id: ItemId) -> Result<Vec<Item>> {
        let mut history = Vec::new();
        let mut cursor = self.get(id)?;
        history.push(cursor.clone());
        while let Some(prev_id) = cursor.supersedes {
            let prev = self.get(prev_id)?;
            history.push(prev.clone());
            cursor = prev;
        }
        Ok(history)
    }

    /// Find the latest revision reachable forward from `id`.
    /// An item is "latest" iff no other item has it in its `supersedes` field.
    ///
    /// # Errors
    ///
    /// Returns `Error::NotFound` if the starting ID is not in the store.
    /// Returns `Error::AmbiguousLatest` if the chain forks (multiple items
    /// supersede the same head). Per Principle VII, the library refuses to
    /// guess which fork wins; callers must resolve.
    /// Returns `Error::Sqlite` on database errors.
    ///
    /// # Panics
    ///
    /// Panics if the internal connection `Mutex` is poisoned.
    pub fn latest_revision(&self, id: ItemId) -> Result<Item> {
        // Confirm the starting item exists.
        let _ = self.get(id)?;

        // Walk forward: from `current`, find items where supersedes = current.id.
        // If exactly one, advance. If zero, current is the head. If many, ambiguous.
        let mut current_id = id;
        loop {
            let conn = self.conn.lock().expect("store mutex poisoned");
            let mut stmt = conn
                .prepare("SELECT id FROM items WHERE supersedes = ?1")
                .map_err(|e| Error::Sqlite {
                    context: "preparing latest_revision walk",
                    source: e,
                })?;
            let next_ids: Vec<String> = stmt
                .query_map(params![current_id.to_string()], |r| r.get(0))
                .map_err(|e| Error::Sqlite {
                    context: "executing latest_revision walk",
                    source: e,
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| Error::Sqlite {
                    context: "collecting latest_revision walk IDs",
                    source: e,
                })?;
            drop(stmt);
            drop(conn);

            match next_ids.len() {
                0 => return self.get(current_id),
                1 => {
                    current_id = next_ids
                        .into_iter()
                        .next()
                        .expect("len == 1")
                        .parse()?;
                }
                _ => {
                    let candidates = next_ids
                        .into_iter()
                        .map(|s| s.parse::<ItemId>())
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    return Err(Error::AmbiguousLatest { candidates });
                }
            }
        }
    }
}

fn load_item(conn: &rusqlite::Connection, id: ItemId) -> Result<Item> {
    let id_text = id.to_string();
    let mut stmt = conn
        .prepare(
            "SELECT content, created_at, supersedes, source, metadata \
             FROM items WHERE id = ?1",
        )
        .map_err(|e| Error::Sqlite {
            context: "preparing get statement",
            source: e,
        })?;
    let row = stmt
        .query_row(params![id_text], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Error::NotFound { id },
            other => Error::Sqlite {
                context: "fetching item row",
                source: other,
            },
        })?;
    let (content, created_at_text, supersedes_text, source, metadata_text) = row;
    let created_at: jiff::Timestamp = created_at_text.parse().map_err(|_| Error::Sqlite {
        context: "parsing stored created_at",
        source: rusqlite::Error::InvalidColumnType(
            1,
            "created_at".into(),
            rusqlite::types::Type::Text,
        ),
    })?;
    let supersedes = supersedes_text
        .as_deref()
        .map(str::parse::<ItemId>)
        .transpose()?;
    let metadata: serde_json::Value =
        serde_json::from_str(&metadata_text).map_err(|e| Error::Json {
            context: "parsing stored metadata JSON",
            source: e,
        })?;

    let mut tag_stmt = conn
        .prepare("SELECT tag FROM item_tags WHERE item_id = ?1 ORDER BY tag ASC")
        .map_err(|e| Error::Sqlite {
            context: "preparing tag query",
            source: e,
        })?;
    let tags: Vec<String> = tag_stmt
        .query_map(params![id_text], |r| r.get(0))
        .map_err(|e| Error::Sqlite {
            context: "querying item tags",
            source: e,
        })?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(|e| Error::Sqlite {
            context: "reading item tag rows",
            source: e,
        })?;

    Ok(Item {
        id,
        content,
        created_at,
        supersedes,
        tags,
        source,
        metadata,
    })
}
