//! `Store` read methods: `get`, `get_optional`, `list`, `list_by_tags`,
//! `revision_history`, `latest_revision`.

use std::collections::{HashSet, VecDeque};
use std::fmt::Write as _;

use rusqlite::params;

use crate::error::{Error, Result};
use crate::item::{Item, ItemId};
use crate::scope::ScopeFilter;
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

    /// Fetch the item carrying `external_id`, if any.
    ///
    /// # Errors
    /// Returns `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    #[allow(clippy::significant_drop_tightening)]
    pub fn get_by_external_id(&self, external_id: &str) -> Result<Option<Item>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let id_text: Option<String> = conn
            .query_row(
                "SELECT id FROM items WHERE external_id = ?1",
                params![external_id],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(Error::Sqlite {
                    context: "looking up external_id",
                    source: other,
                }),
            })?;
        match id_text {
            None => Ok(None),
            Some(t) => load_item(&conn, t.parse::<ItemId>()?).map(Some),
        }
    }

    /// Return the subset of `ids` that already exist as `external_id` values.
    /// One indexed point query per id.
    ///
    /// # Errors
    /// Returns `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    #[allow(clippy::significant_drop_tightening)]
    pub fn existing_external_ids(&self, ids: &[&str]) -> Result<HashSet<String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn
            .prepare_cached("SELECT 1 FROM items WHERE external_id = ?1")
            .map_err(|e| Error::Sqlite {
                context: "preparing external_id existence query",
                source: e,
            })?;
        let mut out = HashSet::with_capacity(ids.len());
        for id in ids {
            let hit = stmt.exists(params![id]).map_err(|e| Error::Sqlite {
                context: "checking external_id existence",
                source: e,
            })?;
            if hit {
                out.insert((*id).to_string());
            }
        }
        Ok(out)
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
    /// Delegates to [`Store::list_by_tags_scoped`], which panics if the
    /// connection `Mutex` is poisoned.
    pub fn list(&self) -> Result<ItemIter<'_>> {
        self.list_by_tags_scoped(&[], None)
    }

    /// Iterate over items whose tag set contains every named tag (AND-semantics).
    /// An empty `tags` slice returns the same result as `list`.
    ///
    /// # Errors
    ///
    /// Same as `list`.
    ///
    /// # Panics
    /// Delegates to [`Store::list_by_tags_scoped`], which panics if the
    /// connection `Mutex` is poisoned.
    pub fn list_by_tags(&self, tags: &[&str]) -> Result<ItemIter<'_>> {
        self.list_by_tags_scoped(tags, None)
    }

    /// Iterate items in `created_at` order, restricted to `filter` when given.
    ///
    /// # Errors
    /// Same as [`Store::list`].
    ///
    /// # Panics
    /// Panics if the internal connection `Mutex` is poisoned (i.e. another
    /// thread panicked while holding the lock).
    pub fn list_scoped(&self, filter: Option<&ScopeFilter>) -> Result<ItemIter<'_>> {
        self.list_by_tags_scoped(&[], filter)
    }

    /// Items carrying every tag in `tags` (AND) and matching `filter` when given.
    ///
    /// # Errors
    /// Same as [`Store::list`].
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn list_by_tags_scoped(
        &self,
        tags: &[&str],
        filter: Option<&ScopeFilter>,
    ) -> Result<ItemIter<'_>> {
        let mut sql = String::from("SELECT i.id FROM items i WHERE 1=1");
        let mut params: Vec<String> = Vec::new();
        if !tags.is_empty() {
            let placeholders = vec!["?"; tags.len()].join(", ");
            // `tags.len()` is a `usize` we computed, not user input, so it is
            // safe to inline as a literal; binding it as a `String` parameter
            // instead would compare `COUNT(DISTINCT tag)` (INTEGER) against a
            // TEXT value, which SQLite's storage-class rules never equate.
            let count = tags.len();
            let _ = write!(
                sql,
                " AND i.id IN (SELECT item_id FROM item_tags WHERE tag IN ({placeholders}) \
                  GROUP BY item_id HAVING COUNT(DISTINCT tag) = {count})"
            );
            params.extend(tags.iter().map(|t| (*t).to_string()));
        }
        if let Some(f) = filter {
            let (clause, binds) = f.sql_clause();
            sql.push_str(" AND ");
            sql.push_str(&clause.replace("scope", "i.scope"));
            params.extend(binds);
        }
        sql.push_str(" ORDER BY i.created_at ASC");

        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&sql).map_err(|e| Error::Sqlite {
            context: "preparing scoped list query",
            source: e,
        })?;
        let id_strings: Vec<String> = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| Error::Sqlite {
                context: "executing scoped list query",
                source: e,
            })?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| Error::Sqlite {
                context: "collecting scoped list IDs",
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

    /// Distinct scopes with item counts, sorted by path.
    ///
    /// # Errors
    /// `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn scopes(&self) -> Result<Vec<(String, usize)>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn
            .prepare(
                "SELECT scope, COUNT(*) FROM items WHERE scope IS NOT NULL \
                 GROUP BY scope ORDER BY scope ASC",
            )
            .map_err(|e| Error::Sqlite {
                context: "preparing scopes query",
                source: e,
            })?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| Error::Sqlite {
                context: "executing scopes query",
                source: e,
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| Error::Sqlite {
                context: "collecting scopes",
                source: e,
            })?;
        drop(stmt);
        drop(conn);
        Ok(rows
            .into_iter()
            .map(|(s, n)| (s, usize::try_from(n).unwrap_or(0)))
            .collect())
    }

    /// The newest `limit` items, optionally restricted to `filter`. Newest
    /// first (callers wanting chronological order reverse the vector).
    ///
    /// # Errors
    /// `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn recent(&self, filter: Option<&ScopeFilter>, limit: usize) -> Result<Vec<Item>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut sql = String::from("SELECT id FROM items WHERE 1=1");
        let mut params: Vec<String> = Vec::new();
        if let Some(f) = filter {
            let (clause, binds) = f.sql_clause();
            sql.push_str(" AND ");
            sql.push_str(clause);
            params.extend(binds);
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        let bind_params = params
            .iter()
            .map(|p| Box::new(p.clone()) as Box<dyn rusqlite::ToSql>)
            .chain(std::iter::once(
                Box::new(limit_i64) as Box<dyn rusqlite::ToSql>
            ))
            .collect::<Vec<_>>();

        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&sql).map_err(|e| Error::Sqlite {
            context: "preparing recent query",
            source: e,
        })?;
        let id_strings: Vec<String> = stmt
            .query_map(
                rusqlite::params_from_iter(bind_params.iter().map(std::convert::AsRef::as_ref)),
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| Error::Sqlite {
                context: "executing recent query",
                source: e,
            })?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| Error::Sqlite {
                context: "collecting recent IDs",
                source: e,
            })?;
        drop(stmt);
        drop(conn);
        let ids = id_strings
            .into_iter()
            .map(|s| s.parse::<ItemId>())
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter().map(|id| self.get(id)).collect()
    }

    /// Number of items matching `filter` (all items when `None`).
    ///
    /// # Errors
    /// `Error::Sqlite` on database error.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn count_scoped(&self, filter: Option<&ScopeFilter>) -> Result<usize> {
        let mut sql = String::from("SELECT COUNT(*) FROM items WHERE 1=1");
        let mut params: Vec<String> = Vec::new();
        if let Some(f) = filter {
            let (clause, binds) = f.sql_clause();
            sql.push_str(" AND ");
            sql.push_str(clause);
            params.extend(binds);
        }
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n: i64 = conn
            .query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| {
                r.get(0)
            })
            .map_err(|e| Error::Sqlite {
                context: "counting scoped items",
                source: e,
            })?;
        drop(conn);
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// The scope of one item (cheap point read, no payload).
    ///
    /// # Errors
    /// `Error::NotFound` if the id is unknown; `Error::Sqlite` otherwise.
    ///
    /// # Panics
    /// Panics if the connection `Mutex` is poisoned.
    pub fn scope_of(&self, id: ItemId) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.query_row(
            "SELECT scope FROM items WHERE id = ?1",
            params![id.to_string()],
            |r| r.get::<_, Option<String>>(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Error::NotFound { id },
            other => Error::Sqlite {
                context: "reading scope",
                source: other,
            },
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
                    current_id = next_ids.into_iter().next().expect("len == 1").parse()?;
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
            "SELECT content, created_at, supersedes, source, metadata, external_id, scope \
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
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Error::NotFound { id },
            other => Error::Sqlite {
                context: "fetching item row",
                source: other,
            },
        })?;
    let (content, created_at_text, supersedes_text, source, metadata_text, external_id, scope) =
        row;
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
        external_id,
        scope,
    })
}
