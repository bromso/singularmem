//! Append-only temporal knowledge graph: entities, facts, and their revision
//! chains. Spec: `docs/superpowers/specs/2026-09-05-knowledge-graph-14-design.md`.
//!
//! [`types`] holds the public data types, [`normalise`] and [`time`] the
//! pure input helpers; the `read` and `write` modules add the operations to
//! [`crate::Store`] itself, so callers reach them as `store.add_fact(..)`
//! rather than through a separate handle.

pub mod normalise;
pub mod time;
pub mod types;

mod read;
mod write;

pub use types::*;
