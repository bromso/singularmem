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

/// Escape the three characters that have special meaning inside XML text
/// content: `&`, `<`, `>`. `'` and `"` only matter inside attribute values,
/// which we never emit user content into (only the digit-only `index`).
///
/// Order matters: `&` MUST be replaced first, otherwise the `&amp;` from
/// other replacements would get double-escaped.
#[allow(dead_code)] // used by format() in Task 3
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_returns_claude() {
        assert_eq!(ClaudeAdapter.name(), "claude");
    }

    #[test]
    fn escape_xml_replaces_ampersand_first_then_angle_brackets() {
        // & must be replaced first; otherwise the &amp; from the < replacement
        // would get re-escaped to &amp;amp; (and similarly for >).
        let input = "a & b < c > d";
        let out = escape_xml(input);
        assert_eq!(out, "a &amp; b &lt; c &gt; d");
    }

    #[test]
    fn escape_xml_leaves_quotes_and_apostrophes_alone() {
        // We never put user content into attribute values, so quote
        // escaping isn't needed and would just add noise.
        let input = r#"alice's "quoted" text"#;
        let out = escape_xml(input);
        assert_eq!(out, r#"alice's "quoted" text"#);
    }

    #[test]
    fn escape_xml_handles_pre_escaped_input_correctly() {
        // Input that already looks like an XML entity gets escaped again,
        // which is the right behaviour: a literal ampersand in user content
        // must be preserved as &amp;, not silently passed through.
        let input = "&amp; literal";
        let out = escape_xml(input);
        assert_eq!(out, "&amp;amp; literal");
    }
}
