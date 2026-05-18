//! Native Node.js bindings for Singularmem via napi-rs.
//!
//! This crate exposes the read-side of `singularmem-core` to JavaScript
//! and TypeScript consumers. See `package.json` for the npm-side wiring.

#![allow(clippy::needless_pass_by_value)]

#[macro_use]
extern crate napi_derive;

/// Returns the crate version. Used as a smoke-test export.
#[napi]
#[allow(clippy::must_use_candidate)]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
