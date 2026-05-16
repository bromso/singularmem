//! Singularmem search — Tantivy-backed lexical index for memory stores.
//!
//! See `docs/formats/store-v1.md` § "Tantivy sidecar index" for the on-disk
//! format and `docs/superpowers/specs/2026-05-16-search-v0-lexical-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

pub mod error;
pub mod index;
pub mod query;
pub mod result;

mod hook;
mod reindex;
mod schema;

// pub use crate::error::{Error, Result};
// pub use crate::index::{Index, IndexOptions};
// pub use crate::query::{Field, Query, QueryBuilder};
// pub use crate::result::{Hit, SearchOptions, SearchResults};
