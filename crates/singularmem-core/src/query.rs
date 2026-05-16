//! `Store` read methods. The full implementation lands in Tasks 9, 10, 11.
//! This file currently contains only `Store::get` — enough to make the
//! ingest round-trip test compile and pass.

use rusqlite::params;

use crate::error::{Error, Result};
use crate::item::{Item, ItemId};
use crate::store::Store;

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
