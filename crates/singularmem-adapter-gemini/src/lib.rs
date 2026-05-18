//! Singularmem retrieval adapter for Google Gemini.
//!
//! Pure formatter implementing [`singularmem_retrieve::Adapter`]. Emits
//! em-dash-separated `Source N` headers with a leading directive line
//! that primes Gemini to ground its answer in the listed sources.
//! Per-block header is `Source N:` when both `source` and `tags` are
//! absent, otherwise `Source N — source: ..., tags: ...:` with the
//! metadata comma-joined. Content immediately follows on the next line.
//!
//! The em-dash separator is U+2014 (UTF-8); Rust source files are UTF-8
//! so the literal compiles fine. Project precedent: sub-project 2c's
//! `--show-ranks` output uses the same character.
//!
//! See `docs/superpowers/specs/2026-05-18-provider-adapter-gemini-3d-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

use singularmem_retrieve::{Adapter, RetrievedContext};

/// Provider adapter for Google Gemini. Stateless unit struct.
pub struct GeminiAdapter;

impl Adapter for GeminiAdapter {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn format(&self, _ctx: &RetrievedContext) -> String {
        // Task 2 implements this.
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_returns_gemini() {
        assert_eq!(GeminiAdapter.name(), "gemini");
    }
}
