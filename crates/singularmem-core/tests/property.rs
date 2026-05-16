//! Property tests using proptest. Each property covers an invariant the
//! library claims in the spec.

use proptest::prelude::*;
use singularmem_core::{NewItem, Store};
use tempfile::TempDir;

// Strategy: produce arbitrary `NewItem`s that satisfy the validation rules.
// (Invalid inputs are exercised by tests/validation.rs.)
fn valid_new_item() -> impl Strategy<Value = NewItem> {
    let content = "[a-zA-Z0-9 ]{1,200}".prop_filter("non-empty", |s| !s.is_empty());
    let tag = "[a-z][a-z0-9-]{0,30}";
    let tags = prop::collection::vec(tag, 0..5);
    let source = prop::option::of("[a-z][a-z0-9-]{0,50}");
    (content, tags, source).prop_map(|(c, t, s)| {
        let mut item = NewItem::text(c);
        item.tags = t;
        item.source = s;
        item
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// For any valid `NewItem`, `ingest(item)` followed by `get(id)` returns
    /// an `Item` whose payload-bearing fields equal the input.
    #[test]
    fn ingest_then_get_round_trip(input in valid_new_item()) {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path().join("store.db")).unwrap();
        let stored = store.ingest(input.clone()).expect("ingest");
        let fetched = store.get(stored.id).expect("get");
        prop_assert_eq!(fetched.content, input.content);
        prop_assert_eq!(fetched.source, input.source);

        // Tags should match the input as a set (after dedup + sort).
        let mut expected_tags = input.tags;
        expected_tags.sort();
        expected_tags.dedup();
        prop_assert_eq!(fetched.tags, expected_tags);
    }

    /// Tag dedup is silent; ingesting duplicates produces the same stored set.
    #[test]
    fn tag_dedup_idempotent(content in "[a-z]{1,50}", tag in "[a-z]{1,20}") {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path().join("store.db")).unwrap();
        let mut item = NewItem::text(content);
        item.tags = vec![tag.clone(), tag.clone(), tag.clone()];
        let stored = store.ingest(item).expect("ingest");
        prop_assert_eq!(stored.tags, vec![tag]);
    }

    /// Two ingests in the same store produce distinct IDs.
    #[test]
    fn distinct_ids(c1 in "[a-z]{1,20}", c2 in "[a-z]{1,20}") {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path().join("store.db")).unwrap();
        let a = store.ingest(NewItem::text(c1)).expect("a");
        let b = store.ingest(NewItem::text(c2)).expect("b");
        prop_assert_ne!(a.id, b.id);
    }
}
