//! Native Node.js bindings for Singularmem via napi-rs.
//!
//! This crate exposes the read-side of `singularmem-core` to JavaScript
//! and TypeScript consumers. See `package.json` for the npm-side wiring.

#![allow(clippy::needless_pass_by_value)]
// napi_derive macros will be used in subsequent tasks; allow the re-export now.
#![allow(unused_imports)]

#[macro_use]
extern crate napi_derive;
