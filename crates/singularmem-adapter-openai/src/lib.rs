//! Singularmem retrieval adapter for OpenAI/Codex.
//!
//! Pure formatter implementing [`singularmem_retrieve::Adapter`]. Emits
//! bracketed citation markers (`[N]`) with a leading directive line
//! that primes GPT-family models to cite back by index. Same-line
//! `source:` in the `[N]` header when present; own-line `tags:` below;
//! blank line; full content emitted verbatim (no escaping).
//!
//! See `docs/superpowers/specs/2026-05-18-provider-adapter-openai-3c-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

use singularmem_retrieve::{Adapter, RetrievedContext};

/// Provider adapter for `OpenAI` / `OpenAI` Codex. Stateless unit struct.
pub struct OpenAiAdapter;

impl Adapter for OpenAiAdapter {
    fn name(&self) -> &'static str {
        "openai"
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
    fn name_returns_openai() {
        assert_eq!(OpenAiAdapter.name(), "openai");
    }
}
