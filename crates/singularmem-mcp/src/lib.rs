//! Library entry for `singularmem-mcp`. Exposes `serve()` so the binary
//! (`src/main.rs`) and the integration test can both launch the server
//! against the same code path.

#![forbid(unsafe_code)]

pub mod error;
pub mod server;

pub use crate::error::{Error, Result};
pub use crate::server::serve;
