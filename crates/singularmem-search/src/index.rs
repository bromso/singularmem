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
        let mmap_dir =
            tantivy::directory::MmapDirectory::open(dir).map_err(|e| Error::IndexCorrupted {
                path: dir.to_path_buf(),
                reason: format!("could not open Tantivy directory: {e}"),
            })?;
        let inner = TantivyIndex::open_or_create(mmap_dir, schema).map_err(|e| Error::Tantivy {
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

    /// Execute a query and return ranked hits.
    ///
    /// # Errors
    /// Returns `Error::Tantivy` on index-read failure.
    pub fn search(
        &self,
        query: &crate::query::Query,
        options: crate::result::SearchOptions,
    ) -> Result<crate::result::SearchResults> {
        use std::str::FromStr;
        use std::time::Instant;
        use tantivy::collector::{Count, TopDocs};
        use tantivy::schema::OwnedValue;
        use tantivy::snippet::SnippetGenerator;
        use tantivy::TantivyDocument;

        let start = Instant::now();
        let searcher = self.reader.searcher();

        let collector = TopDocs::with_limit(options.limit + options.offset);
        let (top_docs, total) = searcher
            .search(&*query.inner, &(collector, Count))
            .map_err(|e| Error::Tantivy {
                context: "executing search",
                source: e,
            })?;

        // Snippet generator (only build if requested).
        let snippet_gen = if options.include_snippets {
            Some(
                SnippetGenerator::create(&searcher, &*query.inner, self.fields.content).map_err(
                    |e| Error::Tantivy {
                        context: "building snippet generator",
                        source: e,
                    },
                )?,
            )
        } else {
            None::<SnippetGenerator>
        };

        let hits: Vec<crate::result::Hit> = top_docs
            .into_iter()
            .skip(options.offset)
            .map(|(score, doc_address)| -> Result<crate::result::Hit> {
                let doc: TantivyDocument =
                    searcher.doc(doc_address).map_err(|e| Error::Tantivy {
                        context: "fetching stored document",
                        source: e,
                    })?;
                let id_str = doc
                    .get_first(self.fields.id)
                    .and_then(|v| {
                        if let OwnedValue::Str(s) = v {
                            Some(s.as_str())
                        } else {
                            None
                        }
                    })
                    .ok_or_else(|| Error::IndexCorrupted {
                        path: self.path.clone(),
                        reason: "document has no id field".to_string(),
                    })?
                    .to_string();
                let id = singularmem_core::ItemId::from_str(&id_str).map_err(|e| {
                    Error::IndexCorrupted {
                        path: self.path.clone(),
                        reason: format!("invalid ULID stored: {e}"),
                    }
                })?;

                let snippet = snippet_gen.as_ref().map(|gen| {
                    let snip = gen.snippet_from_doc(&doc);
                    snip.to_html()
                });

                Ok(crate::result::Hit { id, score, snippet })
            })
            .take(options.limit)
            .collect::<Result<Vec<crate::result::Hit>>>()?;

        Ok(crate::result::SearchResults {
            hits,
            total_matched: total as u64,
            elapsed: start.elapsed(),
        })
    }

    /// Rebuild this index from an iterator of `Item`s (typically `store.list()`).
    /// Deletes all existing documents, writes new ones from the iterator, and
    /// commits once at the end. Calls `on_progress(items_so_far)` every 1000
    /// items.
    ///
    /// Note: `wait_merging_threads` is intentionally skipped — Tantivy 0.22's
    /// API consumes the writer when called, which would invalidate the writer
    /// mutex. Background segment merges may continue after `reindex_from`
    /// returns. The caller's exit-success signal is "all docs indexed and
    /// committed", not "all merges settled".
    ///
    /// # Errors
    /// Returns `Error::Tantivy` on writer failure.
    ///
    /// # Panics
    /// Panics if the internal writer mutex is poisoned (i.e. another thread
    /// panicked while holding the writer lock).
    pub fn reindex_from<I, F>(&self, items: I, mut on_progress: F) -> Result<u64>
    where
        I: IntoIterator<Item = singularmem_core::Item>,
        F: FnMut(u64),
    {
        use singularmem_core::IndexHook;

        // Delete all documents in the current index.
        {
            let writer = self.writer.lock().expect("writer mutex poisoned");
            writer.delete_all_documents().map_err(|e| Error::Tantivy {
                context: "delete_all_documents during reindex",
                source: e,
            })?;
        }

        let mut count: u64 = 0;
        for item in items {
            // on_reindex returns singularmem_core::Result; surface as Tantivy error.
            self.on_reindex(&item).map_err(|core_err| Error::Tantivy {
                context: "on_reindex call during reindex_from",
                source: tantivy::TantivyError::SystemError(core_err.to_string()),
            })?;
            count += 1;
            if count % 1000 == 0 {
                on_progress(count);
            }
        }

        // Single commit at the end of the batch.
        self.commit().map_err(|core_err| Error::Tantivy {
            context: "commit during reindex_from",
            source: tantivy::TantivyError::SystemError(core_err.to_string()),
        })?;

        Ok(count)
    }
}
