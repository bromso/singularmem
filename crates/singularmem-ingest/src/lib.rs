//! Bulk, idempotent ingestion sources for Singularmem.
//!
//! A [`Source`] yields `NewItem`s; a later ingest driver writes them to a
//! `Store`, skipping anything whose `external_id` is already present.
//! Spec: `docs/superpowers/specs/2026-09-03-transcript-ingestion-11-design.md`.

#![forbid(unsafe_code)]

pub mod chunk;
pub mod error;

pub use chunk::{chunk_text, DEFAULT_CHUNK_BYTES};
pub use error::{Error, Result};

use singularmem_core::NewItem;

/// Something that can be turned into memory items.
pub trait Source {
    /// Human-readable label for progress output (usually a path).
    fn name(&self) -> String;

    /// Yield items in source order. Per-item errors are yielded as `Err`
    /// and the iterator continues.
    fn items(&self) -> Box<dyn Iterator<Item = Result<NewItem>> + '_>;

    /// Number of inputs the source deliberately skipped (not errors), valid
    /// after the iterator from [`Source::items`] has been exhausted.
    fn filtered_count(&self) -> usize {
        0
    }
}
