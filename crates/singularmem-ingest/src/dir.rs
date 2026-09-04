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

/// Walks a directory and yields one item per readable UTF-8 text file.
#[derive(Debug)]
pub struct DirectoryWalker {
    /// Canonicalised root.
    pub root: PathBuf,
    /// Files larger than this are skipped (counted as filtered).
    pub max_file_bytes: u64,
    /// Chunk cap in bytes.
    pub chunk_bytes: usize,
    filtered: Cell<usize>,
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
        })
    }

    fn file_to_items(&self, abs: &Path) -> Result<Vec<NewItem>> {
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
        let ext = abs.extension().map(|e| e.to_string_lossy().to_lowercase());
        let chunk_count = chunks.len();
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
                        "size_bytes": meta.len(),
                        "chunk_index": i,
                        "chunk_count": chunk_count,
                    }),
                    external_id: Some(external_id),
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
        let walker = ignore::WalkBuilder::new(&self.root)
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .require_git(false)
            .sort_by_file_path(std::cmp::Ord::cmp)
            .build();
        Box::new(walker.flat_map(move |entry| match entry {
            Err(e) => vec![Err(Error::Io {
                path: self.root.clone(),
                source: std::io::Error::other(e.to_string()),
            })],
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
