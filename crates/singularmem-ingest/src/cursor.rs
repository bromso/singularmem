//! Cursor IDE chat source: reads the `state.vscdb` `SQLite` stores. Nothing
//! here is documented by Cursor; the key shapes were captured from a live
//! install and the parser tolerates missing fields.

use std::cell::{Cell, RefCell};
use std::ops::Deref;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use singularmem_core::NewItem;

use crate::chunk::{chunk_text, DEFAULT_CHUNK_BYTES};
use crate::error::{Error, Result};
use crate::project_filter::{derive_scope, ProjectFilter};
use crate::Source;

/// Cursor's per-user chat store.
#[derive(Debug)]
pub struct CursorChats {
    /// The `…/Cursor/User` directory.
    pub user_dir: PathBuf,
    /// Keep only conversations whose workspace folder names this directory.
    pub project_filter: Option<PathBuf>,
    /// Keep only this composer (conversation) id.
    pub conversation_filter: Option<String>,
    /// Explicit scope override; wins over `cursor/<workspace basename>`.
    pub scope_override: Option<String>,
    /// Chunk cap in bytes.
    pub chunk_bytes: usize,
    filtered: Cell<usize>,
    derived_memo: RefCell<Option<(String, Option<String>)>>,
}

/// One conversation as resolved from the two databases.
struct Composer {
    id: String,
    title: Option<String>,
    created_at: Option<String>,
    workspace: String,
    bubbles: Vec<(String, u8)>, // (bubbleId, type)
}

/// A `SQLite` connection, optionally backed by a temporary copy of the
/// original file. The temp file (if any) lives exactly as long as this
/// value and is deleted on drop.
struct Db {
    conn: Connection,
    _keep: Option<tempfile::NamedTempFile>,
}

impl Deref for Db {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.conn
    }
}

impl CursorChats {
    /// Open the store rooted at `user_dir`.
    ///
    /// # Errors
    /// `Error::NotFound` if `<user_dir>/globalStorage/state.vscdb` is absent.
    pub fn open(user_dir: impl AsRef<Path>) -> Result<Self> {
        let user_dir = user_dir.as_ref().to_path_buf();
        let global = user_dir.join("globalStorage").join("state.vscdb");
        if !global.is_file() {
            return Err(Error::NotFound { path: global });
        }
        Ok(Self {
            user_dir,
            project_filter: None,
            conversation_filter: None,
            scope_override: None,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            filtered: Cell::new(0),
            derived_memo: RefCell::new(None),
        })
    }

    /// Open a `state.vscdb` read-only without taking `SQLite` locks
    /// (`immutable=1`), falling back to a unique temporary copy that is
    /// deleted once the returned `Db` is dropped.
    fn open_db(path: &Path) -> Result<Db> {
        let uri = format!("file:{}?immutable=1", path.display());
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        match Connection::open_with_flags(&uri, flags) {
            Ok(conn) => Ok(Db { conn, _keep: None }),
            Err(first) => {
                let tmp = tempfile::NamedTempFile::new().map_err(|source| Error::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
                std::fs::copy(path, tmp.path()).map_err(|source| Error::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
                let conn = Connection::open_with_flags(
                    tmp.path(),
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .map_err(|e| Error::Io {
                    path: path.to_path_buf(),
                    source: std::io::Error::other(format!("{first}; copy fallback: {e}")),
                })?;
                Ok(Db {
                    conn,
                    _keep: Some(tmp),
                })
            }
        }
    }

    /// Enumerate workspaces → composers, using the workspace DBs for the
    /// composer list and the global DB for headers. A workspace whose
    /// `state.vscdb` cannot be opened or read is skipped (warned, counted
    /// as filtered) rather than aborting the whole scan.
    #[allow(
        clippy::too_many_lines,
        reason = "linear per-field extraction across two DBs; splitting would obscure the single control flow"
    )]
    fn composers(&self, global: &Connection) -> Vec<Composer> {
        let ws_root = self.user_dir.join("workspaceStorage");
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&ws_root) else {
            return out;
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        let mut filter = self.project_filter.as_deref().map(ProjectFilter::new);
        for dir in dirs {
            let Some(folder) = read_workspace_folder(&dir) else {
                self.filtered.set(self.filtered.get() + 1);
                continue;
            };
            if let Some(f) = filter.as_mut() {
                if !f.matches(Some(&folder)) {
                    continue;
                }
            }
            let db_path = dir.join("state.vscdb");
            if !db_path.is_file() {
                continue;
            }
            let ws = match Self::open_db(&db_path) {
                Ok(ws) => ws,
                Err(error) => {
                    tracing::warn!(path = %db_path.display(), %error, "skipping workspace: could not open state.vscdb");
                    self.filtered.set(self.filtered.get() + 1);
                    continue;
                }
            };
            let raw: Option<String> = match ws.query_row(
                "SELECT value FROM ItemTable WHERE key = 'composer.composerData'",
                [],
                |r| r.get(0),
            ) {
                Ok(v) => Some(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(error) => {
                    tracing::warn!(path = %db_path.display(), %error, "skipping workspace: could not read state.vscdb");
                    self.filtered.set(self.filtered.get() + 1);
                    continue;
                }
            };
            let Some(raw) = raw else { continue };
            let data: serde_json::Value =
                serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
            for c in data
                .get("allComposers")
                .and_then(|a| a.as_array())
                .into_iter()
                .flatten()
            {
                let Some(id) = c.get("composerId").and_then(|v| v.as_str()) else {
                    continue;
                };
                if self
                    .conversation_filter
                    .as_deref()
                    .is_some_and(|want| want != id)
                {
                    continue;
                }
                let head: Option<String> = global
                    .prepare_cached("SELECT value FROM cursorDiskKV WHERE key = ?1")
                    .and_then(|mut stmt| {
                        stmt.query_row([format!("composerData:{id}")], |r| r.get(0))
                    })
                    .ok();
                let Some(head) = head else { continue };
                let h: serde_json::Value =
                    serde_json::from_str(&head).unwrap_or(serde_json::Value::Null);
                let bubbles = h
                    .get("fullConversationHeadersOnly")
                    .or_else(|| h.get("conversation"))
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|b| {
                                Some((
                                    b.get("bubbleId")?.as_str()?.to_string(),
                                    u8::try_from(b.get("type")?.as_u64()?).ok()?,
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let created_ms = h
                    .get("createdAt")
                    .and_then(ms_from)
                    .or_else(|| c.get("createdAt").and_then(ms_from));
                out.push(Composer {
                    id: id.to_string(),
                    title: h.get("name").and_then(|v| v.as_str()).map(str::to_string),
                    created_at: created_ms
                        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
                        .map(|t| t.to_string()),
                    workspace: folder.clone(),
                    bubbles,
                });
            }
        }
        out
    }

    fn bubble_items(&self, global: &Connection, c: &Composer) -> Vec<Result<NewItem>> {
        let mut out = Vec::new();
        for (index, (bubble_id, kind)) in c.bubbles.iter().enumerate() {
            let role = match kind {
                1 => "user",
                2 => "assistant",
                _ => {
                    self.filtered.set(self.filtered.get() + 1);
                    continue;
                }
            };
            let raw: Option<String> = global
                .prepare_cached("SELECT value FROM cursorDiskKV WHERE key = ?1")
                .and_then(|mut stmt| {
                    stmt.query_row([format!("bubbleId:{}:{bubble_id}", c.id)], |r| r.get(0))
                })
                .ok();
            let text = raw
                .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
                .and_then(|b| b.get("text").and_then(|t| t.as_str()).map(str::to_string))
                .unwrap_or_default();
            let chunks = chunk_text(&text, self.chunk_bytes);
            if chunks.is_empty() {
                self.filtered.set(self.filtered.get() + 1);
                continue;
            }
            let chunk_count = chunks.len();
            for (i, content) in chunks.into_iter().enumerate() {
                let mut tags = vec![
                    "cursor".to_string(),
                    format!("role:{role}"),
                    "transcript".to_string(),
                ];
                tags.sort();
                let external_id = if chunk_count == 1 {
                    format!("cursor:{}:{bubble_id}", c.id)
                } else {
                    format!("cursor:{}:{bubble_id}#{i}", c.id)
                };
                out.push(Ok(NewItem {
                    content,
                    supersedes: None,
                    tags,
                    source: Some(format!("cursor:{}", c.id)),
                    metadata: serde_json::json!({
                        "composer_id": c.id, "bubble_id": bubble_id, "index": index, "role": role,
                        "title": c.title, "workspace": c.workspace, "composer_created_at": c.created_at,
                        "chunk_index": i, "chunk_count": chunk_count,
                    }),
                    external_id: Some(external_id),
                    scope: None,
                }));
            }
        }
        out
    }
}

/// `folder` from `<workspace dir>/workspace.json`, as a filesystem path.
///
/// Cursor stores `folder` as a URI (`file:///Users/me/My%20Repo`, or on
/// Windows `file:///c%3A/Users/...`): strip the scheme, percent-decode, and
/// (Windows only) drop the leading `/` in front of a drive letter.
fn read_workspace_folder(dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(dir.join("workspace.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let folder = v.get("folder")?.as_str()?;
    let stripped = folder.strip_prefix("file://").map_or(folder, |p| p);
    let decoded = percent_decode(stripped);
    let bytes = decoded.as_bytes();
    if cfg!(windows)
        && bytes.first() == Some(&b'/')
        && bytes.get(1).is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(2) == Some(&b':')
    {
        return Some(decoded[1..].to_string());
    }
    Some(decoded)
}

/// Percent-decode `%XX` escape sequences in `s`. A `%` not followed by two
/// hex digits (an invalid escape, or a trailing `%`) is passed through
/// literally. Decoding happens byte-wise; the result is lossily converted
/// back to UTF-8.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(hex) = s.get(i + 1..i + 3) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `createdAt` as milliseconds since the epoch, accepting either a JSON
/// number or a numeric string (Cursor uses both shapes across versions).
fn ms_from(v: &serde_json::Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str()?.parse().ok())
}

impl Source for CursorChats {
    fn name(&self) -> String {
        self.user_dir.display().to_string()
    }

    fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_> {
        self.filtered.set(0);
        let global_path = self.user_dir.join("globalStorage").join("state.vscdb");
        let global = match Self::open_db(&global_path) {
            Ok(c) => c,
            Err(e) => return Box::new(std::iter::once(Err(e))),
        };
        let composers = self.composers(&global);
        let mut all = Vec::new();
        for c in &composers {
            all.extend(self.bubble_items(&global, c));
        }
        Box::new(all.into_iter())
    }

    fn filtered_count(&self) -> usize {
        self.filtered.get()
    }

    fn default_scope(&self, item: &NewItem) -> Option<String> {
        if let Some(o) = &self.scope_override {
            match singularmem_core::scope::validate(o) {
                Ok(s) => return Some(s),
                Err(e) => {
                    tracing::warn!(r#override = %o, error = %e, "ignoring invalid scope override; using derived scope");
                }
            }
        }
        let ws = item.metadata.get("workspace")?.as_str()?;
        if let Some((seen, result)) = self.derived_memo.borrow().as_ref() {
            if seen == ws {
                return result.clone();
            }
        }
        let result = derive_scope("cursor", ws);
        *self.derived_memo.borrow_mut() = Some((ws.to_string(), result.clone()));
        result
    }
}

/// Cursor's per-OS user directory.
#[must_use]
pub fn default_cursor_user_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support/Cursor/User"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Cursor").join("User"))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/Cursor/User"))
    }
}

/// Test-support: a bubble in a fixture conversation.
#[cfg(any(test, feature = "testing"))]
pub struct FixtureBubble {
    /// Bubble id.
    pub id: &'static str,
    /// Bubble type: `1` = user, `2` = assistant.
    pub kind: u8,
    /// Bubble text.
    pub text: &'static str,
}

/// Test-support: a fixture workspace.
#[cfg(any(test, feature = "testing"))]
pub struct FixtureWorkspace {
    /// Workspace storage dir name (a hash in real installs).
    pub hash: &'static str,
    /// `workspace.json`'s `folder`, without a `file://` fixture would add itself.
    pub folder: Option<&'static str>,
    /// `(composerId, title, createdAt ms, bubbles)`
    pub composers: Vec<(&'static str, &'static str, i64, Vec<FixtureBubble>)>,
}

/// Test-support: write a miniature Cursor user dir with the real key shapes.
///
/// # Panics
/// On any I/O or `SQLite` failure (test helper).
#[cfg(any(test, feature = "testing"))]
pub fn write_fixture(user_dir: &Path, workspaces: &[FixtureWorkspace]) {
    std::fs::create_dir_all(user_dir.join("globalStorage")).unwrap();
    let global = Connection::open(user_dir.join("globalStorage/state.vscdb")).unwrap();
    global
        .execute_batch(
            "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB); \
             CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value BLOB);",
        )
        .unwrap();
    for ws in workspaces {
        let dir = user_dir.join("workspaceStorage").join(ws.hash);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(folder) = ws.folder {
            std::fs::write(
                dir.join("workspace.json"),
                format!("{{\"folder\":\"file://{folder}\"}}"),
            )
            .unwrap();
        }
        let wsdb = Connection::open(dir.join("state.vscdb")).unwrap();
        wsdb.execute_batch(
            "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB); \
             CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value BLOB);",
        )
        .unwrap();
        let all: Vec<serde_json::Value> = ws
            .composers
            .iter()
            .map(|(id, _, created, _)| {
                serde_json::json!({"type":"head","composerId":id,"createdAt":created.to_string()})
            })
            .collect();
        wsdb.execute(
            "INSERT INTO ItemTable VALUES ('composer.composerData', ?1)",
            [serde_json::json!({"allComposers": all}).to_string()],
        )
        .unwrap();
        for (id, title, created, bubbles) in &ws.composers {
            let headers: Vec<serde_json::Value> = bubbles
                .iter()
                .map(|b| serde_json::json!({"bubbleId": b.id, "type": b.kind}))
                .collect();
            global
                .execute(
                    "INSERT INTO cursorDiskKV VALUES (?1, ?2)",
                    [
                        format!("composerData:{id}"),
                        serde_json::json!({
                            "composerId": id, "name": title, "createdAt": created,
                            "fullConversationHeadersOnly": headers,
                        })
                        .to_string(),
                    ],
                )
                .unwrap();
            for b in bubbles {
                global
                    .execute(
                        "INSERT INTO cursorDiskKV VALUES (?1, ?2)",
                        [
                            format!("bubbleId:{id}:{}", b.id),
                            serde_json::json!({"bubbleId": b.id, "type": b.kind, "text": b.text})
                                .to_string(),
                        ],
                    )
                    .unwrap();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_handles_escapes_and_literals() {
        assert_eq!(percent_decode("My%20Repo"), "My Repo");
        assert_eq!(percent_decode("c%3A"), "c:");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("abc%"), "abc%");
        assert_eq!(percent_decode("plain"), "plain");
    }

    /// A path containing `?` breaks the `file:...?immutable=1` URI form
    /// (`SQLite` parses everything after the first `?` as query parameters),
    /// but not a plain-path open. This proves `open_db` falls back to a
    /// temporary copy, and that the copy is cleaned up when the returned
    /// `Db` is dropped.
    #[test]
    fn open_db_falls_back_to_a_cleaned_up_temp_copy() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("state?.vscdb");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch("CREATE TABLE t (k TEXT PRIMARY KEY, v TEXT);")
                .unwrap();
            conn.execute("INSERT INTO t VALUES ('a', 'b')", []).unwrap();
        }
        let before: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();

        {
            let db = CursorChats::open_db(&db_path).unwrap();
            let v: String = db
                .query_row("SELECT v FROM t WHERE k = 'a'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(v, "b");
        }

        let after: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        assert_eq!(
            before.len(),
            after.len(),
            "temp copy must be removed once the Db is dropped: before={before:?} after={after:?}"
        );
    }
}
