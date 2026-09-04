//! Cursor IDE chat source: reads the `state.vscdb` `SQLite` stores. Nothing
//! here is documented by Cursor; the key shapes were captured from a live
//! install and the parser tolerates missing fields.

use std::cell::{Cell, RefCell};
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
    /// (`immutable=1`), falling back to a temporary copy.
    fn open_db(path: &Path) -> Result<Connection> {
        let uri = format!("file:{}?immutable=1", path.display());
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        match Connection::open_with_flags(&uri, flags) {
            Ok(c) => Ok(c),
            Err(first) => {
                let tmp = std::env::temp_dir()
                    .join(format!("singularmem-cursor-{}.vscdb", std::process::id()));
                std::fs::copy(path, &tmp).map_err(|source| Error::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
                Connection::open_with_flags(
                    &tmp,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )
                .map_err(|e| Error::Io {
                    path: path.to_path_buf(),
                    source: std::io::Error::other(format!("{first}; copy fallback: {e}")),
                })
            }
        }
    }

    /// Enumerate workspaces → composers, using the workspace DBs for the
    /// composer list and the global DB for headers.
    fn composers(&self, global: &Connection) -> Result<Vec<Composer>> {
        let ws_root = self.user_dir.join("workspaceStorage");
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&ws_root) else {
            return Ok(out);
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
            let ws = Self::open_db(&db_path)?;
            let raw: Option<String> = ws
                .query_row(
                    "SELECT value FROM ItemTable WHERE key = 'composer.composerData'",
                    [],
                    |r| r.get(0),
                )
                .ok();
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
                    .query_row(
                        "SELECT value FROM cursorDiskKV WHERE key = ?1",
                        [format!("composerData:{id}")],
                        |r| r.get(0),
                    )
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
                    .and_then(serde_json::Value::as_i64)
                    .or_else(|| c.get("createdAt").and_then(|v| v.as_str()?.parse().ok()));
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
        Ok(out)
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
                .query_row(
                    "SELECT value FROM cursorDiskKV WHERE key = ?1",
                    [format!("bubbleId:{}:{bubble_id}", c.id)],
                    |r| r.get(0),
                )
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
fn read_workspace_folder(dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(dir.join("workspace.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let folder = v.get("folder")?.as_str()?;
    Some(
        folder
            .strip_prefix("file://")
            .map_or(folder, |p| p)
            .to_string(),
    )
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
        let composers = match self.composers(&global) {
            Ok(c) => c,
            Err(e) => return Box::new(std::iter::once(Err(e))),
        };
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
