//! `impl IndexHook for Index` — bridges singularmem-core's hook trait to the
//! Tantivy index.

use singularmem_core::{Error as CoreError, IndexHook, Item, Result as CoreResult};
use tantivy::TantivyDocument;

use crate::index::Index;

impl IndexHook for Index {
    fn on_ingest(&self, item: &Item) -> CoreResult<()> {
        index_item(self, item).map_err(|e| to_core_error(&e))
    }

    fn on_reindex(&self, item: &Item) -> CoreResult<()> {
        // Same logic; future versions may differ (e.g., skip duplicate detection).
        index_item(self, item).map_err(|e| to_core_error(&e))
    }

    fn commit(&self) -> CoreResult<()> {
        // Scope the writer lock so it is released before we call reader.reload().
        {
            let mut writer = self.writer.lock().expect("Tantivy writer mutex poisoned");
            writer.commit().map_err(|e| {
                to_core_error(&crate::Error::Tantivy {
                    context: "committing Tantivy writer",
                    source: e,
                })
            })?;
        }
        // Force the reader to reflect the new commit immediately.
        // ReloadPolicy::OnCommitWithDelay is asynchronous (10s of ms);
        // calling reload() here makes the reader consistent right after commit
        // so callers don't need to sleep or poll.
        self.reader.reload().map_err(|e| {
            to_core_error(&crate::Error::Tantivy {
                context: "reloading reader after commit",
                source: e,
            })
        })?;
        Ok(())
    }
}

fn index_item(index: &Index, item: &Item) -> crate::Result<()> {
    let nanos: i128 = item.created_at.as_nanosecond();
    let odt = tantivy::time::OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .unwrap_or(tantivy::time::OffsetDateTime::UNIX_EPOCH);
    let datetime = tantivy::DateTime::from_utc(odt);

    let mut doc = TantivyDocument::default();
    doc.add_text(index.fields.id, item.id.to_string());
    doc.add_text(index.fields.content, &item.content);
    if let Some(src) = &item.source {
        doc.add_text(index.fields.source, src);
    }
    if let Some(sup) = &item.supersedes {
        doc.add_text(index.fields.supersedes, sup.to_string());
    }
    for tag in &item.tags {
        doc.add_text(index.fields.tags, tag);
    }
    doc.add_date(index.fields.created_at, datetime);

    // Scope the writer lock so it is released immediately after add_document.
    {
        let writer = index.writer.lock().expect("Tantivy writer mutex poisoned");
        writer
            .add_document(doc)
            .map_err(|e| crate::Error::Tantivy {
                context: "adding document to Tantivy writer",
                source: e,
            })?;
    }

    Ok(())
}

fn to_core_error(e: &crate::Error) -> CoreError {
    // Wrap any singularmem-search error as a core Error::Io with the message,
    // so the hook contract (Result is singularmem_core::Result) is satisfied
    // without core needing to depend on search.
    CoreError::Io(std::io::Error::other(e.to_string()))
}
