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

    fn format(&self, ctx: &RetrievedContext) -> String {
        use std::fmt::Write;
        if ctx.blocks.is_empty() {
            return "<documents></documents>\n".to_string();
        }
        let mut out = String::new();
        let _ = writeln!(out, "<documents>");
        for (i, block) in ctx.blocks.iter().enumerate() {
            let _ = writeln!(out, "<document index=\"{}\">", i + 1);
            if let Some(source) = &block.source {
                let _ = writeln!(out, "<source>{}</source>", escape_xml(source));
            }
            if !block.tags.is_empty() {
                let joined = block.tags.join(", ");
                let _ = writeln!(out, "<tags>{}</tags>", escape_xml(&joined));
            }
            let _ = writeln!(out, "<document_content>");
            let _ = writeln!(out, "{}", escape_xml(&block.content));
            let _ = writeln!(out, "</document_content>");
            let _ = writeln!(out, "</document>");
        }
        let _ = writeln!(out, "</documents>");
        out
    }
}

/// Escape the three characters that have special meaning inside XML text
/// content: `&`, `<`, `>`. `'` and `"` only matter inside attribute values,
/// which we never emit user content into (only the digit-only `index`).
///
/// Order matters: `&` MUST be replaced first, otherwise the `&amp;` from
/// other replacements would get double-escaped.
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

    use jiff::Timestamp;
    use singularmem_core::ItemId;
    use singularmem_retrieve::MemoryBlock;
    use singularmem_search::ScoreKind;
    use std::str::FromStr;
    use std::time::Duration;

    fn sample_block(id_str: &str, source: Option<&str>, tags: Vec<&str>) -> MemoryBlock {
        MemoryBlock {
            id: ItemId::from_str(id_str).unwrap(),
            content: "the quick brown fox jumps over the lazy dog".to_string(),
            score: 0.5,
            score_kind: ScoreKind::Rrf,
            source: source.map(String::from),
            tags: tags.into_iter().map(String::from).collect(),
            created_at: Timestamp::from_str("2026-05-12T14:30:00Z").unwrap(),
        }
    }

    fn sample_context(blocks: Vec<MemoryBlock>, query: &str) -> RetrievedContext {
        RetrievedContext {
            blocks,
            query: query.to_string(),
            elapsed: Duration::from_millis(1),
            total_considered: 0,
        }
    }

    #[test]
    fn format_wraps_in_documents_element() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = ClaudeAdapter.format(&ctx);
        assert!(
            out.starts_with("<documents>\n"),
            "output should start with documents tag: {out}"
        );
        assert!(
            out.trim_end().ends_with("</documents>"),
            "output should end with closing documents tag: {out}"
        );
    }

    #[test]
    fn format_uses_one_indexed_document_indices() {
        let ctx = sample_context(
            vec![
                sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]),
                sample_block("01BX5ZZKBKACTAV9WEVGEMMVRZ", None, vec![]),
            ],
            "fox",
        );
        let out = ClaudeAdapter.format(&ctx);
        assert!(
            out.contains("<document index=\"1\">"),
            "missing index=1: {out}"
        );
        assert!(
            out.contains("<document index=\"2\">"),
            "missing index=2: {out}"
        );
        assert!(
            !out.contains("<document index=\"0\">"),
            "0-indexed slipped in: {out}"
        );
    }

    #[test]
    fn format_includes_source_when_present() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                Some("claude-conversation:abc-123"),
                vec![],
            )],
            "fox",
        );
        let out = ClaudeAdapter.format(&ctx);
        assert!(
            out.contains("<source>claude-conversation:abc-123</source>"),
            "missing source element: {out}"
        );
    }

    #[test]
    fn format_omits_source_when_none() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = ClaudeAdapter.format(&ctx);
        assert!(
            !out.contains("<source>"),
            "unexpected source element: {out}"
        );
    }

    #[test]
    fn format_includes_tags_when_non_empty() {
        let ctx = sample_context(
            vec![sample_block(
                "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                None,
                vec!["fox", "animals"],
            )],
            "fox",
        );
        let out = ClaudeAdapter.format(&ctx);
        assert!(
            out.contains("<tags>fox, animals</tags>"),
            "missing tags element: {out}"
        );
    }

    #[test]
    fn format_omits_tags_when_empty() {
        let ctx = sample_context(
            vec![sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![])],
            "fox",
        );
        let out = ClaudeAdapter.format(&ctx);
        assert!(!out.contains("<tags>"), "unexpected tags element: {out}");
    }

    #[test]
    fn format_escapes_xml_special_chars_in_content() {
        let mut block = sample_block("01ARZ3NDEKTSV4RRFFQ69G5FAV", None, vec![]);
        block.content = r#"<script>alert("xss")</script> with & ampersand"#.to_string();
        let ctx = sample_context(vec![block], "fox");
        let out = ClaudeAdapter.format(&ctx);
        // Special chars escaped:
        assert!(out.contains("&lt;script&gt;"), "< not escaped: {out}");
        assert!(out.contains("&lt;/script&gt;"), "</ not escaped: {out}");
        assert!(out.contains("&amp;"), "& not escaped: {out}");
        // Raw tags must NOT appear (they would break Claude's XML parser):
        assert!(
            !out.contains("<script>"),
            "raw <script> tag leaked through into output: {out}"
        );
    }

    #[test]
    fn format_empty_context_emits_empty_documents() {
        let ctx = sample_context(vec![], "nothing matches");
        let out = ClaudeAdapter.format(&ctx);
        assert_eq!(out, "<documents></documents>\n");
    }
}
