//! SQL DDL for `format_version = 4` and the migration runner.

use crate::error::{Error, Result};
use crate::format::FORMAT_VERSION;

/// DDL applied by the 1 → 2 migration (excluding the `BEGIN`/`COMMIT`
/// bracketing, which is handled in Rust so the transaction can be inspected
/// and conditionally rolled back — see `run_migration`).
const MIGRATE_1_TO_2_DDL: &str = "
ALTER TABLE items ADD COLUMN external_id TEXT;
CREATE UNIQUE INDEX idx_items_external_id ON items(external_id) WHERE external_id IS NOT NULL;
";

/// DDL applied by the 2 → 3 migration (same transactional shape as 1 → 2;
/// see `run_migration`).
const MIGRATE_2_TO_3_DDL: &str = "
ALTER TABLE items ADD COLUMN scope TEXT;
CREATE INDEX idx_items_scope ON items(scope) WHERE scope IS NOT NULL;
";

/// DDL applied by the 3 → 4 migration: the `entities` and `facts` tables
/// backing the knowledge graph (same transactional shape as 1 → 2 and
/// 2 → 3; see `run_migration`). Spec:
/// `docs/superpowers/specs/2026-09-05-knowledge-graph-14-design.md`
/// § "Store format v4".
///
/// Entities are store-global: identity is the normalised name alone, so
/// `tantivy` is one node everywhere. Only `facts` carry a `scope`, which
/// is what scoped queries narrow on.
const MIGRATE_3_TO_4_DDL: &str = "
CREATE TABLE entities (
    id               TEXT PRIMARY KEY NOT NULL,
    name             TEXT NOT NULL,
    normalised_name  TEXT NOT NULL,
    kind             TEXT,
    created_at       TEXT NOT NULL,
    CHECK (length(name) > 0)
) STRICT;
CREATE UNIQUE INDEX idx_entities_identity ON entities(normalised_name);

CREATE TABLE facts (
    id              TEXT PRIMARY KEY NOT NULL,
    subject_id      TEXT NOT NULL,
    predicate       TEXT NOT NULL,
    object_id       TEXT,
    object_value    TEXT,
    valid_from      TEXT,
    valid_to        TEXT,
    confidence      REAL NOT NULL DEFAULT 1.0,
    source_item_id  TEXT,
    scope           TEXT,
    supersedes      TEXT,
    recorded_at     TEXT NOT NULL,
    FOREIGN KEY (subject_id) REFERENCES entities(id),
    FOREIGN KEY (object_id) REFERENCES entities(id),
    FOREIGN KEY (source_item_id) REFERENCES items(id) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (supersedes) REFERENCES facts(id) DEFERRABLE INITIALLY DEFERRED,
    CHECK ((object_id IS NULL) <> (object_value IS NULL)),
    CHECK (confidence >= 0.0 AND confidence <= 1.0),
    CHECK (valid_to IS NULL OR valid_from IS NULL OR valid_to >= valid_from)
) STRICT;
CREATE INDEX idx_facts_subject   ON facts(subject_id);
CREATE INDEX idx_facts_object    ON facts(object_id) WHERE object_id IS NOT NULL;
CREATE INDEX idx_facts_predicate ON facts(predicate);
CREATE INDEX idx_facts_supersedes ON facts(supersedes) WHERE supersedes IS NOT NULL;
CREATE INDEX idx_facts_scope     ON facts(scope) WHERE scope IS NOT NULL;
";

/// The full v4 DDL for a fresh store: the v3 schema plus
/// [`MIGRATE_3_TO_4_DDL`]'s `entities`/`facts` tables, written as a single
/// self-contained literal so a fresh store's schema reads in one place.
const DDL_V4: &str = "
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
    scope        TEXT,
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
CREATE INDEX idx_items_scope ON items(scope) WHERE scope IS NOT NULL;

CREATE TABLE entities (
    id               TEXT PRIMARY KEY NOT NULL,
    name             TEXT NOT NULL,
    normalised_name  TEXT NOT NULL,
    kind             TEXT,
    created_at       TEXT NOT NULL,
    CHECK (length(name) > 0)
) STRICT;
CREATE UNIQUE INDEX idx_entities_identity ON entities(normalised_name);

CREATE TABLE facts (
    id              TEXT PRIMARY KEY NOT NULL,
    subject_id      TEXT NOT NULL,
    predicate       TEXT NOT NULL,
    object_id       TEXT,
    object_value    TEXT,
    valid_from      TEXT,
    valid_to        TEXT,
    confidence      REAL NOT NULL DEFAULT 1.0,
    source_item_id  TEXT,
    scope           TEXT,
    supersedes      TEXT,
    recorded_at     TEXT NOT NULL,
    FOREIGN KEY (subject_id) REFERENCES entities(id),
    FOREIGN KEY (object_id) REFERENCES entities(id),
    FOREIGN KEY (source_item_id) REFERENCES items(id) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (supersedes) REFERENCES facts(id) DEFERRABLE INITIALLY DEFERRED,
    CHECK ((object_id IS NULL) <> (object_value IS NULL)),
    CHECK (confidence >= 0.0 AND confidence <= 1.0),
    CHECK (valid_to IS NULL OR valid_from IS NULL OR valid_to >= valid_from)
) STRICT;
CREATE INDEX idx_facts_subject   ON facts(subject_id);
CREATE INDEX idx_facts_object    ON facts(object_id) WHERE object_id IS NOT NULL;
CREATE INDEX idx_facts_predicate ON facts(predicate);
CREATE INDEX idx_facts_supersedes ON facts(supersedes) WHERE supersedes IS NOT NULL;
CREATE INDEX idx_facts_scope     ON facts(scope) WHERE scope IS NOT NULL;
";

/// Apply the 1 → 2 migration.
///
/// # Errors
///
/// Returns `Error::Migration` if any statement fails; the transaction is
/// rolled back and the store is left at format version 1.
pub fn migrate_1_to_2(conn: &mut rusqlite::Connection) -> Result<()> {
    run_migration(conn, "1", "2", MIGRATE_1_TO_2_DDL)
}

/// Apply the 2 → 3 migration.
///
/// # Errors
///
/// Returns `Error::Migration { from: "2", to: "3", .. }` if any statement
/// fails; the transaction is rolled back and the store is left at format
/// version 2.
pub fn migrate_2_to_3(conn: &mut rusqlite::Connection) -> Result<()> {
    run_migration(conn, "2", "3", MIGRATE_2_TO_3_DDL)
}

/// Apply the 3 → 4 migration: adds the `entities` and `facts` tables.
///
/// # Errors
///
/// Returns `Error::Migration { from: "3", to: "4", .. }` if any statement
/// fails; the transaction is rolled back and the store is left at format
/// version 3.
pub fn migrate_3_to_4(conn: &mut rusqlite::Connection) -> Result<()> {
    run_migration(conn, "3", "4", MIGRATE_3_TO_4_DDL)
}

/// Run a single-step format migration from `from` to `to`, applying `ddl`.
///
/// Uses `BEGIN IMMEDIATE` to take the write lock up front (avoiding a
/// deferred-transaction upgrade race against another writer), then re-reads
/// `format_version` inside the transaction: if it no longer equals `from`
/// (another process already migrated the store while we were waiting for
/// the lock), this is a no-op — the transaction is rolled back and `Ok(())`
/// is returned. Otherwise `ddl` and the meta update are applied and
/// committed together.
///
/// # Errors
///
/// Returns `Error::Migration { from, to, .. }` if any statement fails; the
/// transaction is rolled back and the store is left at `from`.
fn run_migration(
    conn: &mut rusqlite::Connection,
    from: &'static str,
    to: &'static str,
    ddl: &str,
) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| migration_err(from, to, e.to_string()))?;

    let current = read_format_version(&tx).map_err(|e| migration_err(from, to, e.to_string()))?;
    if current.as_deref() != Some(from) {
        // Another process already completed this migration (or moved the
        // store past it) before we acquired the write lock; nothing left
        // to do.
        tx.rollback()
            .map_err(|e| migration_err(from, to, e.to_string()))?;
        return Ok(());
    }

    if let Err(e) = tx.execute_batch(ddl) {
        let _ = tx.rollback();
        return Err(migration_err(from, to, e.to_string()));
    }

    if let Err(e) = tx.execute(
        "UPDATE singularmem_meta SET value = ?1 WHERE key = 'format_version'",
        rusqlite::params![to],
    ) {
        let _ = tx.rollback();
        return Err(migration_err(from, to, e.to_string()));
    }

    tx.commit()
        .map_err(|e| migration_err(from, to, e.to_string()))
}

/// Build an `Error::Migration` from `from` to `to` with the given reason.
fn migration_err(from: &str, to: &'static str, reason: String) -> Error {
    Error::Migration {
        from: from.to_string(),
        to,
        reason,
    }
}

/// Apply the current (v4) schema and write `format_version = '4'` to the
/// meta table. Used by `Store::open` on a fresh store.
///
/// Takes the write lock up front with `BEGIN IMMEDIATE` and re-reads
/// `format_version` inside the transaction, exactly as `run_migration`
/// does: if a version row now exists, another process bootstrapped the
/// store while we were waiting for the lock, so this rolls back and
/// returns `Ok(())` rather than failing with "table … already exists".
/// The DDL and both meta rows are otherwise applied and committed
/// together, so a concurrent opener never observes a half-built schema.
///
/// # Errors
///
/// `Error::Sqlite` if the transaction, the DDL, or either meta insert
/// fails; the transaction is rolled back, leaving the file as it was.
pub fn apply_current(conn: &mut rusqlite::Connection, created_at: &str) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| Error::Sqlite {
            context: "beginning fresh-store bootstrap transaction",
            source: e,
        })?;

    if read_format_version(&tx)?.is_some() {
        // Another process finished the bootstrap before we took the lock.
        tx.rollback().map_err(|e| Error::Sqlite {
            context: "rolling back a bootstrap another process already did",
            source: e,
        })?;
        return Ok(());
    }

    let applied = tx
        .execute_batch(DDL_V4)
        .map_err(|e| Error::Sqlite {
            context: "applying v4 schema",
            source: e,
        })
        .and_then(|()| {
            tx.execute(
                "INSERT INTO singularmem_meta (key, value) VALUES ('format_version', ?1)",
                rusqlite::params![FORMAT_VERSION],
            )
            .map_err(|e| Error::Sqlite {
                context: "writing format_version meta row",
                source: e,
            })
        })
        .and_then(|_| {
            tx.execute(
                "INSERT INTO singularmem_meta (key, value) VALUES ('created_at', ?1)",
                rusqlite::params![created_at],
            )
            .map_err(|e| Error::Sqlite {
                context: "writing created_at meta row",
                source: e,
            })
        });

    if let Err(e) = applied {
        let _ = tx.rollback();
        return Err(e);
    }

    tx.commit().map_err(|e| Error::Sqlite {
        context: "committing fresh-store bootstrap",
        source: e,
    })
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
