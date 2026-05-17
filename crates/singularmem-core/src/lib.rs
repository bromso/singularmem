//! Singularmem memory store — local-first, SQLite-backed, immutable text items
//! with supersedes-chained revisions.
//!
//! See `docs/formats/store-v1.md` in the repository root for the on-disk format
//! specification and `docs/superpowers/specs/2026-05-16-memory-store-v0-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

pub mod clock;
pub mod error;
pub mod format;
pub mod hook;
pub mod item;
pub mod rng;
pub mod store;

mod export;
mod ingest;
mod query;
mod schema;

pub use crate::clock::{Clock, SystemClock};
pub use crate::error::{Error, Result};
pub use crate::format::FORMAT_VERSION;
pub use crate::hook::IndexHook;
pub use crate::item::{Item, ItemId, NewItem};
pub use crate::query::ItemIter;
pub use crate::rng::{OsRng, Rng};
pub use crate::store::{Store, StoreOptions};
