//! SQL DDL for `format_version = 2` and the migration runner.

use crate::error::{Error, Result};
use crate::format::FORMAT_VERSION;

/// The full v2 DDL for a fresh store: v1 plus the nullable, unique
/// `external_id` column. Spec: `docs/formats/store-v2.md`.
const DDL_V2: &str = "
CREATE TABLE singularmem_meta (
    key    TEXT PRIMARY KEY NOT NULL,
    value  TEXT NOT NULL
) STRICT;

CREATE TABLE items (
    id           TEXT PRIMARY KEY NOT NULL,
    content      TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    supersedes   TEXT,
    source       TEXT,
    metadata     TEXT NOT NULL DEFAULT '{}',
    external_id  TEXT,
    FOREIGN KEY (supersedes) REFERENCES items(id) DEFERRABLE INITIALLY DEFERRED,
    CHECK (length(content) > 0),
    CHECK (length(content) <= 1048576),
    CHECK (json_valid(metadata) AND json_type(metadata) = 'object')
) STRICT;

CREATE TABLE item_tags (
    item_id  TEXT NOT NULL,
    tag      TEXT NOT NULL,
    PRIMARY KEY (item_id, tag),
    FOREIGN KEY (item_id) REFERENCES items(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_items_created_at ON items(created_at);
CREATE INDEX idx_items_supersedes ON items(supersedes) WHERE supersedes IS NOT NULL;
CREATE INDEX idx_item_tags_tag ON item_tags(tag);
CREATE UNIQUE INDEX idx_items_external_id ON items(external_id) WHERE external_id IS NOT NULL;
";

/// DDL applied by the 1 → 2 migration (excluding the `BEGIN`/`COMMIT`
/// bracketing, which is handled in Rust so the transaction can be inspected
/// and conditionally rolled back — see `migrate_1_to_2`).
const MIGRATE_1_TO_2_DDL: &str = "
ALTER TABLE items ADD COLUMN external_id TEXT;
CREATE UNIQUE INDEX idx_items_external_id ON items(external_id) WHERE external_id IS NOT NULL;
";

/// Apply the 1 → 2 migration.
///
/// Uses `BEGIN IMMEDIATE` to take the write lock up front (avoiding a
/// deferred-transaction upgrade race against another writer), then re-reads
/// `format_version` inside the transaction: if another process already won
/// the race and migrated the store to 2 while we were waiting for the lock,
/// this is a no-op (the transaction is rolled back — nothing to commit — and
/// `Ok(())` is returned). Otherwise the DDL and meta update are applied and
/// committed together.
///
/// # Errors
///
/// Returns `Error::Migration` if any statement fails; the transaction is
/// rolled back and the store is left at format version 1.
pub fn migrate_1_to_2(conn: &mut rusqlite::Connection) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| migration_err(e.to_string()))?;

    let current = read_format_version(&tx).map_err(|e| migration_err(e.to_string()))?;
    if current.as_deref() == Some(FORMAT_VERSION) {
        // Another process already completed the migration before we
        // acquired the write lock; nothing left to do.
        tx.rollback().map_err(|e| migration_err(e.to_string()))?;
        return Ok(());
    }

    if let Err(e) = tx.execute_batch(MIGRATE_1_TO_2_DDL) {
        let _ = tx.rollback();
        return Err(migration_err(e.to_string()));
    }

    if let Err(e) = tx.execute(
        "UPDATE singularmem_meta SET value = ?1 WHERE key = 'format_version'",
        rusqlite::params![FORMAT_VERSION],
    ) {
        let _ = tx.rollback();
        return Err(migration_err(e.to_string()));
    }

    tx.commit().map_err(|e| migration_err(e.to_string()))
}

/// Build an `Error::Migration` from `1` to the current `FORMAT_VERSION` with
/// the given reason.
fn migration_err(reason: String) -> Error {
    Error::Migration {
        from: "1".to_string(),
        to: FORMAT_VERSION,
        reason,
    }
}

/// Apply the current (v2) schema and write `format_version = '2'` to the meta
/// table. Used by `Store::open` on a fresh store. Idempotent only in the
/// sense that `CREATE TABLE` will fail loudly if the schema already exists —
/// callers must check the meta table first.
pub fn apply_current(conn: &rusqlite::Connection, created_at: &str) -> Result<()> {
    conn.execute_batch(DDL_V2).map_err(|e| Error::Sqlite {
        context: "applying v2 schema",
        source: e,
    })?;

    conn.execute(
        "INSERT INTO singularmem_meta (key, value) VALUES ('format_version', ?1)",
        rusqlite::params![FORMAT_VERSION],
    )
    .map_err(|e| Error::Sqlite {
        context: "writing format_version meta row",
        source: e,
    })?;

    conn.execute(
        "INSERT INTO singularmem_meta (key, value) VALUES ('created_at', ?1)",
        rusqlite::params![created_at],
    )
    .map_err(|e| Error::Sqlite {
        context: "writing created_at meta row",
        source: e,
    })?;

    Ok(())
}

/// Read the `format_version` meta row. Returns `None` if the row does not
/// exist (i.e. this is not a Singularmem store, or the meta table is empty),
/// or if the `singularmem_meta` table itself does not yet exist (fresh DB).
pub fn read_format_version(conn: &rusqlite::Connection) -> Result<Option<String>> {
    let mut stmt =
        match conn.prepare("SELECT value FROM singularmem_meta WHERE key = 'format_version'") {
            Ok(s) => s,
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.extended_code == rusqlite::ffi::SQLITE_ERROR =>
            {
                // "no such table" — fresh database with no schema yet.
                return Ok(None);
            }
            Err(e) => {
                return Err(Error::Sqlite {
                    context: "preparing format_version query",
                    source: e,
                });
            }
        };
    stmt.query_row([], |row| row.get::<_, String>(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(Error::Sqlite {
                context: "reading format_version meta row",
                source: other,
            }),
        })
}
