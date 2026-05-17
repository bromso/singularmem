//! `Adapter` trait + the default `PlainAdapter` formatter.

use crate::retriever::RetrievedContext;

/// Provider adapter contract per Constitution Principle II.
///
/// An adapter is a pure formatting strategy: it takes a [`RetrievedContext`]
/// and renders it as a single prompt-ready string in whatever format the
/// underlying LLM provider prefers (XML for Claude, Markdown for `OpenAI`,
/// plain text for local models).
///
/// **Contract:** implementations MUST be pure functions — no I/O, no
/// network calls, deterministic for identical input. This is enforced
/// by convention (not the type system) and is what makes the trait
/// trivially testable and composable.
pub trait Adapter: Send + Sync {
    /// Stable identifier used in CLI flags and logs.
    /// Lowercase, hyphen-separated. Examples: `"plain"`, `"claude"`, `"openai"`.
    fn name(&self) -> &str;

    /// Render a [`RetrievedContext`] as a single prompt-ready string.
    ///
    /// MUST be a pure function: no I/O, deterministic given the same input.
    fn format(&self, ctx: &RetrievedContext) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One-line concrete adapter used purely to verify the trait compiles
    /// and is object-safe (`Box<dyn Adapter>` works).
    struct NoopAdapter;
    impl Adapter for NoopAdapter {
        fn name(&self) -> &'static str {
            "noop"
        }
        fn format(&self, _ctx: &RetrievedContext) -> String {
            String::new()
        }
    }

    #[test]
    fn adapter_trait_is_object_safe() {
        let a: Box<dyn Adapter> = Box::new(NoopAdapter);
        assert_eq!(a.name(), "noop");
    }
}
