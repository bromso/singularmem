//! Singularmem search — Tantivy-backed lexical index for memory stores.
//!
//! See `docs/formats/store-v1.md` § "Tantivy sidecar index" for the on-disk
//! format and `docs/superpowers/specs/2026-05-16-search-v0-lexical-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

pub mod embedder;
pub mod error;
pub mod index;
pub mod model;
pub mod query;
pub mod result;
pub mod semantic_query;
pub mod testing;
pub mod vector_index;

mod hook;
mod reindex;
mod schema;

pub use crate::embedder::{Embedder, FastembedEmbedder};
pub use crate::error::{Error, Result};
pub use crate::index::{Index, IndexOptions};
pub use crate::model::EmbeddingModel;
pub use crate::query::{Field, Query, QueryBuilder};
pub use crate::result::{Hit, SearchOptions, SearchResults};
pub use crate::semantic_query::{SemanticHit, SemanticSearchOptions, SemanticSearchResults};
pub use crate::vector_index::{EmbedderIndex, VectorHit, VectorIndex, VectorIndexMeta, VectorIndexOptions};
