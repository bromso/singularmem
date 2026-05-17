//! Singularmem retrieve — composes hybrid search + store reads into
//! prompt-ready memory blocks, and defines the typed `Adapter` contract
//! that per-provider formatter crates implement.
//!
//! See `docs/superpowers/specs/2026-05-18-provider-adapters-3a-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod error;
pub mod retriever;
pub mod testing;

pub use crate::error::{Error, Result};

pub use crate::retriever::{MemoryBlock, RetrieveOptions, RetrievedContext};

// Re-exports activated in subsequent tasks:
// pub use crate::adapter::{Adapter, PlainAdapter};
// pub use crate::retriever::Retriever;
// pub use crate::testing::MockAdapter;
