//! `Index` — wraps a Tantivy index with the writer mutex.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tantivy::{Index as TantivyIndex, IndexReader, IndexWriter, ReloadPolicy};

use crate::error::{Error, Result};
use crate::schema::{build_schema, Fields};

/// Options controlling how an `Index` is opened.
#[derive(Debug, Clone, Copy)]
pub struct IndexOptions {
    /// Writer RAM budget in bytes. Tantivy default is 50 MB; we keep it.
    pub writer_memory_bytes: usize,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            writer_memory_bytes: 50 * 1024 * 1024,
        }
    }
}

/// Tantivy-backed lexical index. Owns the writer + a reusable reader.
#[allow(dead_code)]
pub struct Index {
    pub(crate) inner: TantivyIndex,
    pub(crate) writer: Mutex<IndexWriter>,
    pub(crate) reader: IndexReader,
    pub(crate) fields: Fields,
    pub(crate) path: PathBuf,
}

impl Index {
    /// Open (or create) a Tantivy index at the given directory.
    ///
    /// # Errors
    /// Returns `Error::Tantivy` if Tantivy fails to open or create the index
    /// (e.g. the directory exists but contains incompatible segment files).
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(dir, IndexOptions::default())
    }

    /// Open with explicit options.
    ///
    /// # Errors
    /// Same as `open`.
    pub fn open_with_options(dir: impl AsRef<Path>, options: IndexOptions) -> Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir).map_err(Error::Io)?;

        let (schema, fields) = build_schema();

        // Tantivy's `open_or_create` behaviour: open existing or build new from schema.
        let mmap_dir = tantivy::directory::MmapDirectory::open(dir).map_err(|e| {
            Error::IndexCorrupted {
                path: dir.to_path_buf(),
                reason: format!("could not open Tantivy directory: {e}"),
            }
        })?;
        let inner =
            TantivyIndex::open_or_create(mmap_dir, schema).map_err(|e| Error::Tantivy {
                context: "opening Tantivy index",
                source: e,
            })?;

        let writer = inner
            .writer(options.writer_memory_bytes)
            .map_err(|e| Error::Tantivy {
                context: "constructing Tantivy writer",
                source: e,
            })?;

        let reader = inner
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| Error::Tantivy {
                context: "constructing Tantivy reader",
                source: e,
            })?;

        Ok(Self {
            inner,
            writer: Mutex::new(writer),
            reader,
            fields,
            path: dir.to_path_buf(),
        })
    }

    /// Number of indexed documents (post-commit segments).
    ///
    /// # Errors
    /// Returns `Error::Tantivy` if the reader cannot be searched.
    pub fn doc_count(&self) -> Result<u64> {
        let searcher = self.reader.searcher();
        Ok(searcher.num_docs())
    }
}
