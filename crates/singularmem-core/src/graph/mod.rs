//! Append-only temporal knowledge graph: entities, facts, and their revision
//! chains. Spec: `docs/superpowers/specs/2026-09-05-knowledge-graph-14-design.md`.
//!
//! This task (ids, normalisation, time parsing, store format v4) lands the
//! public types and the pure helpers; the read/write operations arrive in
//! later tasks.

pub mod normalise;
pub mod time;
pub mod types;

pub use types::*;
