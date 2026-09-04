//! `Store` — the entire domain surface in one type.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OpenFlags};

use crate::clock::{Clock, SystemClock};
use crate::error::{Error, Result};
use crate::format::FORMAT_VERSION;
use crate::hook::IndexHook;
use crate::rng::{OsRng, Rng};
use crate::schema;

/// Options controlling how a store is opened.
#[derive(Debug, Clone, Copy, Default)]
pub struct StoreOptions {
    /// Open the store in read-only mode. Writes return `Error::ReadOnly`.
    /// In read-only mode, the store path MUST already exist.
    pub read_only: bool,
}

/// The Singularmem memory store.
///
/// Backed by a single `SQLite` file (default WAL journaling). `Store` is
/// `Send + Sync`; the underlying connection is wrapped in a `Mutex`.
///
/// # Lifetime
///
/// `Store` owns its connection. Drop closes the file. WAL sidecar files are
/// reclaimed automatically by `SQLite` at clean shutdown.
pub struct Store {
    pub(crate) conn: Mutex<Connection>,
    // clock and rng are used by ingest (Phase D) and later phases.
    #[allow(dead_code)]
    pub(crate) clock: Box<dyn Clock>,
    #[allow(dead_code)]
    pub(crate) rng: Mutex<Box<dyn Rng>>,
    pub(crate) read_only: bool,
    pub(crate) hook: Mutex<Option<Box<dyn IndexHook>>>,
}

/// Put the connection's journal mode into WAL, tolerating a concurrent
/// opener of the same fresh store.
///
/// `PRAGMA journal_mode = WAL` needs a brief exclusive lock and — unlike an
/// ordinary statement — reports `SQLITE_BUSY` immediately instead of going
/// through the busy handler, so `busy_timeout` does not cover it. Two
/// processes opening the same store at the same moment (a burst of editor
/// hooks on a brand-new machine) therefore race. Retry a bounded number of
/// times, and treat "already `wal`" as success: the journal mode is a
/// persistent property of the file, so whoever won the race set it for
/// everyone.
fn set_wal_journal_mode(conn: &Connection) -> Result<()> {
    const ATTEMPTS: u32 = 10;

    let mut last = None;
    for attempt in 0..ATTEMPTS {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => return Ok(()),
            Err(e) => {
                let already_wal = conn
                    .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
                    .is_ok_and(|mode| mode.eq_ignore_ascii_case("wal"));
                if already_wal {
                    return Ok(());
                }
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(5 * u64::from(attempt + 1)));
            }
        }
    }
    Err(Error::Sqlite {
        context: "setting WAL journal mode",
        source: last.unwrap_or_else(|| unreachable!("at least one attempt always runs")),
    })
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Open or create a store at the given path. Uses `SystemClock` and `OsRng`.
    /// Creates parent directories if missing.
    ///
    /// # Errors
    ///
    /// Returns `Error::Sqlite` on database open failure, `Error::Io` on
    /// directory creation failure, `Error::UnsupportedFormatVersion` if the
    /// existing file has a format version this binary cannot read, and
    /// `Error::Migration` if an in-place format migration is needed but
    /// fails (or is refused because the store was opened read-only).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(path, StoreOptions::default())
    }

    /// Open with explicit clock and rng injection — for tests and deterministic
    /// replay.
    ///
    /// # Errors
    ///
    /// Same as `open`.
    pub fn open_with(
        path: impl AsRef<Path>,
        clock: Box<dyn Clock>,
        rng: Box<dyn Rng>,
    ) -> Result<Self> {
        Self::open_inner(path.as_ref(), StoreOptions::default(), clock, rng)
    }

    /// Open with non-default options. Uses `SystemClock` and `OsRng`.
    ///
    /// # Errors
    ///
    /// Same as `open`. If `options.read_only` is true and the path does not
    /// exist, returns an error rather than creating an empty store.
    pub fn open_with_options(path: impl AsRef<Path>, options: StoreOptions) -> Result<Self> {
        Self::open_inner(
            path.as_ref(),
            options,
            Box::new(SystemClock),
            Box::new(OsRng),
        )
    }

    fn open_inner(
        path: &Path,
        options: StoreOptions,
        clock: Box<dyn Clock>,
        rng: Box<dyn Rng>,
    ) -> Result<Self> {
        if options.read_only {
            // Read-only: must not create the file.
            if !path.exists() {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!(
                        "store path {} does not exist; refusing to create in read-only mode",
                        path.display()
                    ),
                )));
            }
        } else {
            // Write mode: create parent directories as needed.
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
        }

        let flags = if options.read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
        };

        let mut conn = Connection::open_with_flags(path, flags).map_err(|e| Error::Sqlite {
            context: "opening database file",
            source: e,
        })?;

        // Pragmas — must run before schema work. `busy_timeout` comes first
        // so every later statement waits for a competing writer instead of
        // returning SQLITE_BUSY at once.
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| Error::Sqlite {
                context: "setting busy_timeout",
                source: e,
            })?;

        // Changing journal_mode writes the database header, which a read-only
        // connection cannot do; skip it there and simply read with whatever
        // journal mode is on disk.
        if !options.read_only {
            set_wal_journal_mode(&conn)?;
        }
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| Error::Sqlite {
                context: "enabling foreign_keys pragma",
                source: e,
            })?;

        // Bootstrap schema (write) or verify format version (read-only).
        if options.read_only {
            // Read-only: must already be at a supported version.
            let version =
                schema::read_format_version(&conn)?.ok_or(Error::UnsupportedFormatVersion {
                    found: "<missing>".to_string(),
                    max_supported: FORMAT_VERSION,
                })?;
            if version == "1" || version == "2" {
                return Err(Error::Migration {
                    from: version,
                    to: FORMAT_VERSION,
                    reason: "store is opened read-only; open it writable once to migrate"
                        .to_string(),
                });
            }
            if version != FORMAT_VERSION {
                return Err(Error::UnsupportedFormatVersion {
                    found: version,
                    max_supported: FORMAT_VERSION,
                });
            }
        } else {
            match schema::read_format_version(&conn)? {
                None => {
                    let now = clock.now().to_string();
                    // Transactional and race-safe: a concurrent first open
                    // of the same fresh store rolls back and returns Ok
                    // rather than failing with "table already exists".
                    schema::apply_current(&mut conn, &now)?;
                }
                Some(v) if v == FORMAT_VERSION => { /* already current */ }
                Some(v) if v == "1" => {
                    schema::migrate_1_to_2(&mut conn)?;
                    schema::migrate_2_to_3(&mut conn)?;
                }
                Some(v) if v == "2" => schema::migrate_2_to_3(&mut conn)?,
                Some(other) => {
                    return Err(Error::UnsupportedFormatVersion {
                        found: other,
                        max_supported: FORMAT_VERSION,
                    });
                }
            }
        }

        Ok(Self {
            conn: Mutex::new(conn),
            clock,
            rng: Mutex::new(rng),
            read_only: options.read_only,
            hook: Mutex::new(None),
        })
    }

    /// Read the on-disk format version from the `singularmem_meta` table.
    ///
    /// # Errors
    ///
    /// Returns `Error::Sqlite` on read failure.
    ///
    /// # Panics
    ///
    /// Panics if the internal `Mutex` is poisoned (i.e. another thread panicked
    /// while holding the lock).
    pub fn format_version(&self) -> Result<String> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        schema::read_format_version(&conn)?.ok_or(Error::UnsupportedFormatVersion {
            found: "<missing>".to_string(),
            max_supported: FORMAT_VERSION,
        })
    }

    /// Internal helper for write methods to refuse if read-only.
    // Used by ingest, query, and export phases.
    #[allow(dead_code)]
    pub(crate) const fn assert_writable(&self, op: &'static str) -> Result<()> {
        if self.read_only {
            Err(Error::ReadOnly { operation: op })
        } else {
            Ok(())
        }
    }

    /// Open with an `IndexHook` attached. Equivalent to `Store::open` for the
    /// `SQLite` layer.
    ///
    /// # Errors
    /// Same as `Store::open`.
    pub fn open_with_hook(path: impl AsRef<Path>, hook: Box<dyn IndexHook>) -> Result<Self> {
        let mut store = Self::open(path)?;
        store.set_hook(Some(hook));
        Ok(store)
    }

    /// Open with multiple `IndexHook`s. Equivalent to constructing a
    /// `MultiHook` from the list and calling `open_with_hook`.
    ///
    /// # Errors
    /// Same as `Store::open`.
    pub fn open_with_hooks(path: impl AsRef<Path>, hooks: Vec<Box<dyn IndexHook>>) -> Result<Self> {
        Self::open_with_hook(path, Box::new(crate::hook::MultiHook::new(hooks)))
    }

    /// Replace the `IndexHook` on an already-open store. Pass `None` to detach.
    ///
    /// # Panics
    ///
    /// Panics if the internal hook `Mutex` is poisoned (another thread panicked
    /// while holding the lock).
    pub fn set_hook(&mut self, hook: Option<Box<dyn IndexHook>>) {
        *self.hook.lock().expect("store hook mutex poisoned") = hook;
    }
}
