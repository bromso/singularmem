//! Test fixtures. `MockAdapter` is unconditionally available so cross-crate
//! tests don't need `--features testing` toggling — same pattern as
//! `singularmem-search::testing::MockEmbedder`.

use crate::adapter::Adapter;
use crate::retriever::RetrievedContext;

/// Deterministic, easily-asserted-against `Adapter` for downstream tests.
///
/// Output shape:
///
/// ```text
/// MOCK[query="<query>" blocks=N ids=[id1,id2,...]]
/// ```
///
/// Used by sub-projects 3b/3c/3d to test their adapters' integration with
/// `Retriever` without committing to `PlainAdapter`'s specific output.
pub struct MockAdapter;

impl Adapter for MockAdapter {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn format(&self, ctx: &RetrievedContext) -> String {
        let ids: Vec<String> = ctx.blocks.iter().map(|b| b.id.to_string()).collect();
        format!(
            "MOCK[query={:?} blocks={} ids=[{}]]\n",
            ctx.query,
            ctx.blocks.len(),
            ids.join(",")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retriever::MemoryBlock;
    use jiff::Timestamp;
    use singularmem_core::ItemId;
    use singularmem_search::ScoreKind;
    use std::str::FromStr;
    use std::time::Duration;

    #[test]
    fn mock_adapter_format_includes_ids() {
        let block = MemoryBlock {
            id: ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap(),
            content: "anything".to_string(),
            score: 0.5,
            score_kind: ScoreKind::Rrf,
            source: None,
            tags: vec![],
            created_at: Timestamp::from_str("2026-05-12T14:30:00Z").unwrap(),
        };
        let ctx = RetrievedContext {
            blocks: vec![block],
            query: "test".to_string(),
            elapsed: Duration::from_millis(1),
            total_considered: 1,
        };
        let out = MockAdapter.format(&ctx);
        assert!(out.contains("MOCK["), "missing prefix: {out}");
        assert!(out.contains("query=\"test\""), "missing query: {out}");
        assert!(out.contains("blocks=1"), "missing block count: {out}");
        assert!(
            out.contains("ids=[01ARZ3NDEKTSV4RRFFQ69G5FAV]"),
            "missing id list: {out}"
        );
    }
}
