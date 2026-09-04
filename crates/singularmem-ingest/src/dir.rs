//! Source-tree walker: one item per text file, `.gitignore`-aware.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use singularmem_core::NewItem;

use crate::chunk::{chunk_text, DEFAULT_CHUNK_BYTES};
use crate::error::{Error, Result};
use crate::Source;

/// Default per-file size cap (1 MiB).
pub const DEFAULT_MAX_FILE_BYTES: u64 = 1_048_576;

/// The store's `external_id` cap. Mirrors `singularmem_core`'s private
/// `MAX_EXTERNAL_ID_BYTES`; a longer id would be rejected at validation.
const MAX_EXTERNAL_ID_BYTES: usize = 512;

/// The store's per-tag cap. Mirrors `singularmem_core`'s private
/// `MAX_TAG_BYTES`.
const MAX_TAG_BYTES: usize = 64;

/// Walks a directory and yields one item per readable UTF-8 text file.
///
/// Only `.gitignore`/`.ignore` files found inside the walked root are
/// honoured; ignore files in parent directories, and global or per-repo
/// `.git/info/exclude` rules, are never consulted (Principle VI: a walk's
/// result must not depend on the machine it runs on).
#[derive(Debug)]
pub struct DirectoryWalker {
    /// Canonicalised root.
    pub root: PathBuf,
    /// Files larger than this are skipped (counted as filtered).
    pub max_file_bytes: u64,
    /// Chunk cap in bytes.
    pub chunk_bytes: usize,
    filtered: Cell<usize>,
    visited: Cell<usize>,
}

impl DirectoryWalker {
    /// Create a walker rooted at `root` (must exist).
    ///
    /// # Errors
    /// `Error::NotFound` if `root` is not a directory; `Error::Io` if it
    /// cannot be canonicalised.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        if !root.is_dir() {
            return Err(Error::NotFound {
                path: root.to_path_buf(),
            });
        }
        let root = root.canonicalize().map_err(|source| Error::Io {
            path: root.to_path_buf(),
            source,
        })?;
        Ok(Self {
            root,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            filtered: Cell::new(0),
            visited: Cell::new(0),
        })
    }

    /// Files handed to the content pipeline this run (items produced,
    /// filtered, or rejected). Walk-level errors on directories are not
    /// counted. Valid after the iterator from [`Source::items`] has been
    /// exhausted; the CLI reports it as the "across N files" figure.
    #[must_use]
    pub fn visited_files(&self) -> usize {
        self.visited.get()
    }

    fn file_to_items(&self, abs: &Path) -> Result<Vec<NewItem>> {
        self.visited.set(self.visited.get() + 1);
        let meta = std::fs::metadata(abs).map_err(|source| Error::Io {
            path: abs.to_path_buf(),
            source,
        })?;
        if meta.len() > self.max_file_bytes {
            self.filtered.set(self.filtered.get() + 1);
            return Ok(Vec::new());
        }
        let bytes = std::fs::read(abs).map_err(|source| Error::Io {
            path: abs.to_path_buf(),
            source,
        })?;
        let sniff = &bytes[..bytes.len().min(8192)];
        if sniff.contains(&0) {
            self.filtered.set(self.filtered.get() + 1);
            return Ok(Vec::new());
        }
        let size_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let Ok(text) = String::from_utf8(bytes) else {
            self.filtered.set(self.filtered.get() + 1);
            return Ok(Vec::new());
        };
        let chunks = chunk_text(&text, self.chunk_bytes);
        if chunks.is_empty() {
            self.filtered.set(self.filtered.get() + 1);
            return Ok(Vec::new());
        }
        let sha256 = format!("{:x}", Sha256::digest(text.as_bytes()));
        let rel = abs.strip_prefix(&self.root).unwrap_or(abs);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let abs_str = abs.display().to_string();
        let chunk_count = chunks.len();

        // Reject the whole file up front rather than letting the store fail
        // validation mid-batch: one over-long path must not sink the run.
        // The last chunk carries the longest `#<n>` suffix.
        let longest_id_len = if chunk_count == 1 {
            "file:".len() + abs_str.len()
        } else {
            "file:".len() + abs_str.len() + 1 + (chunk_count - 1).to_string().len()
        };
        if longest_id_len > MAX_EXTERNAL_ID_BYTES {
            return Err(Error::Unsupported {
                path: abs.to_path_buf(),
                reason: format!(
                    "path too long for external_id (max {MAX_EXTERNAL_ID_BYTES} bytes)"
                ),
            });
        }

        // An extension too long to fit a tag is dropped, not fatal.
        let ext = abs
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .filter(|e| "ext:".len() + e.len() <= MAX_TAG_BYTES);
        Ok(chunks
            .into_iter()
            .enumerate()
            .map(|(i, content)| {
                let mut tags = vec!["file".to_string()];
                if let Some(e) = &ext {
                    tags.push(format!("ext:{e}"));
                }
                tags.sort();
                let external_id = if chunk_count == 1 {
                    format!("file:{abs_str}")
                } else {
                    format!("file:{abs_str}#{i}")
                };
                NewItem {
                    content,
                    supersedes: None,
                    tags,
                    source: Some(format!("dir:{}", self.root.display())),
                    metadata: serde_json::json!({
                        "path": abs_str,
                        "rel_path": rel_str,
                        "sha256": sha256,
                        "size_bytes": size_bytes,
                        "chunk_index": i,
                        "chunk_count": chunk_count,
                    }),
                    external_id: Some(external_id),
                    scope: None,
                }
            })
            .collect())
    }
}

impl Source for DirectoryWalker {
    fn name(&self) -> String {
        self.root.display().to_string()
    }

    fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_> {
        self.filtered.set(0);
        self.visited.set(0);
        let walker = ignore::WalkBuilder::new(&self.root)
            .hidden(true)
            .git_ignore(true)
            // Global and per-repo-exclude ignore files live outside the
            // walked tree; honouring them would make results depend on the
            // machine (Principle VI).
            .git_global(false)
            .git_exclude(false)
            .require_git(false)
            // Only `.gitignore`/`.ignore` files inside the walked root
            // apply; do not climb into parent directories for more rules
            // (also Principle VI — a walk of a subdirectory must not depend
            // on files outside it).
            .parents(false)
            .sort_by_file_path(std::cmp::Ord::cmp)
            .build();
        Box::new(walker.flat_map(move |entry| match entry {
            Err(e) => {
                let path = walk_error_path(&e, &self.root);
                vec![Err(Error::Io {
                    path,
                    source: std::io::Error::other(e.to_string()),
                })]
            }
            Ok(entry) => {
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    return Vec::new();
                }
                match self.file_to_items(entry.path()) {
                    Ok(items) => items.into_iter().map(Ok).collect(),
                    Err(e) => vec![Err(e)],
                }
            }
        }))
    }

    fn filtered_count(&self) -> usize {
        self.filtered.get()
    }
}

/// Extracts the path a walker error is associated with, falling back to
/// `root` when the error carries none.
pub(crate) fn walk_error_path(e: &ignore::Error, root: &Path) -> PathBuf {
    match e {
        ignore::Error::WithPath { path, .. } => path.clone(),
        _ => root.to_path_buf(),
    }
}
