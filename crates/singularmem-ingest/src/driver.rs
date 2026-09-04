//! `ingest_source`: write a `Source` into a `Store`, idempotently.

use std::collections::HashSet;

use singularmem_core::{Error as CoreError, NewItem, Store};

use crate::error::{Error, Result};
use crate::Source;

/// Items per `ingest_many` transaction.
pub const BATCH_SIZE: usize = 500;

/// Outcome counts for one `ingest_source` run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// Items written (or, in dry-run, that would have been written).
    pub ingested: usize,
    /// Items whose `external_id` was already present (in the store, or
    /// earlier in this run) and did not qualify for replacement.
    pub skipped_existing: usize,
    /// Inputs the source deliberately filtered (tool results, binaries, …).
    pub skipped_filtered: usize,
    /// Per-item source errors (malformed lines, unreadable files).
    pub failed: usize,
}

/// Ingest everything `source` yields that is not already in `store`.
///
/// Items carrying `metadata.sha256` whose stored counterpart has a
/// different hash are ingested via `Store::ingest_replacing`, superseding
/// the old item. Everything else new is written in batches of
/// [`BATCH_SIZE`]. With `dry_run`, nothing is written but counts are
/// computed as if it were.
///
/// A batch the store rejects with `Error::Validation` is retried item by
/// item so that one bad item (an over-long `external_id`, say) costs one
/// `Report::failed` rather than the whole batch.
///
/// # Errors
/// Returns `Err` only for store-level failures (read-only, `SQLite`). Source
/// errors and per-item validation rejects are counted in `Report::failed`.
pub fn ingest_source(store: &Store, source: &dyn Source, dry_run: bool) -> Result<Report> {
    let mut report = Report::default();
    let mut candidates: Vec<NewItem> = Vec::new();
    for r in source.items() {
        match r {
            Ok(mut item) => {
                if item.scope.is_none() {
                    item.scope = source.default_scope(&item);
                }
                candidates.push(item);
            }
            Err(e) => {
                tracing::warn!(source = %source.name(), error = %e, "skipping item");
                report.failed += 1;
            }
        }
    }
    report.skipped_filtered = source.filtered_count();

    let ids: Vec<&str> = candidates
        .iter()
        .filter_map(|i| i.external_id.as_deref())
        .collect();
    let existing: HashSet<String> = store.existing_external_ids(&ids)?;

    let mut fresh: Vec<NewItem> = Vec::new();
    let mut accepted_ids: HashSet<String> = HashSet::new();
    for item in candidates {
        let Some(key) = item.external_id.clone() else {
            fresh.push(item);
            continue;
        };
        if !accepted_ids.insert(key.clone()) {
            tracing::warn!(
                external_id = %key,
                source = %source.name(),
                "skipping duplicate external_id within this run"
            );
            report.skipped_existing += 1;
            continue;
        }
        if !existing.contains(&key) {
            fresh.push(item);
            continue;
        }
        // Present: replace only when both sides carry a hash and they differ.
        let new_hash = item
            .metadata
            .get("sha256")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let old = store.get_by_external_id(&key)?;
        let old_hash = old
            .as_ref()
            .and_then(|o| o.metadata.get("sha256").and_then(|v| v.as_str()))
            .map(str::to_owned);
        let differs = matches!((&new_hash, &old_hash), (Some(a), Some(b)) if a != b);
        match (differs, old) {
            (true, Some(o)) => {
                if !dry_run {
                    store.ingest_replacing(item, o.id)?;
                }
                report.ingested += 1;
            }
            _ => report.skipped_existing += 1,
        }
    }

    if dry_run {
        report.ingested += fresh.len();
        return Ok(report);
    }

    write_batches(store, source, fresh, &mut report)?;
    Ok(report)
}

/// Write `fresh` to `store` in [`BATCH_SIZE`] transactions, updating
/// `report`. Split out of [`ingest_source`] to keep each function readable.
fn write_batches(
    store: &Store,
    source: &dyn Source,
    mut fresh: Vec<NewItem>,
    report: &mut Report,
) -> Result<()> {
    while !fresh.is_empty() {
        let n = fresh.len().min(BATCH_SIZE);
        let batch: Vec<NewItem> = fresh.drain(..n).collect();
        match store.ingest_many(batch.clone()) {
            Ok(written) => report.ingested += written.len(),
            Err(CoreError::ExternalIdConflict { .. }) => {
                // Race with a concurrent writer: re-filter and retry once.
                let ids: Vec<&str> = batch
                    .iter()
                    .filter_map(|i| i.external_id.as_deref())
                    .collect();
                let now_existing = store.existing_external_ids(&ids)?;
                let retry: Vec<NewItem> = batch
                    .into_iter()
                    .filter(|i| {
                        !i.external_id
                            .as_deref()
                            .is_some_and(|k| now_existing.contains(k))
                    })
                    .collect();
                let skipped = n - retry.len();
                report.skipped_existing += skipped;
                let written = store.ingest_many(retry)?;
                report.ingested += written.len();
            }
            Err(CoreError::Validation { .. }) => {
                // One malformed item must not sink the whole batch: retry
                // the batch item by item so the rest still lands.
                for item in batch {
                    let key = item.external_id.clone().unwrap_or_default();
                    match store.ingest(item) {
                        Ok(_) => report.ingested += 1,
                        Err(e @ CoreError::Validation { .. }) => {
                            tracing::warn!(
                                external_id = %key,
                                source = %source.name(),
                                error = %e,
                                "store rejected item"
                            );
                            report.failed += 1;
                        }
                        Err(e) => return Err(Error::Core(e)),
                    }
                }
            }
            Err(e) => return Err(Error::Core(e)),
        }
    }
    Ok(())
}
