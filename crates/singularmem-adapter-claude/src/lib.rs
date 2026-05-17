//! Singularmem retrieval adapter for Anthropic Claude.
//!
//! Pure formatter implementing [`singularmem_retrieve::Adapter`]. Emits
//! the element-heavy XML shape Anthropic's prompt-engineering docs
//! recommend: a `<documents>` wrapper around 1-indexed `<document>`
//! elements with optional `<source>`/`<tags>` sub-elements and an
//! XML-escaped `<document_content>` body.
//!
//! See `docs/superpowers/specs/2026-05-18-provider-adapter-claude-3b-design.md`
//! for the design rationale.

#![forbid(unsafe_code)]

use singularmem_retrieve::{Adapter, RetrievedContext};

/// Provider adapter for Anthropic Claude. Stateless unit struct.
pub struct ClaudeAdapter;

impl Adapter for ClaudeAdapter {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn format(&self, _ctx: &RetrievedContext) -> String {
        // Task 3 implements this.
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_returns_claude() {
        assert_eq!(ClaudeAdapter.name(), "claude");
    }
}
