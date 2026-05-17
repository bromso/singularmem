//! Property tests for search invariants.

use proptest::prelude::*;
use singularmem_core::{NewItem, Store};
use singularmem_search::{Index, Query, SearchOptions};
use tempfile::TempDir;

/// Open a store with an attached index hook and return the dir + store + index
/// path. The hook's `Index` is moved into the store; after this call only the
/// store holds a writer on `index_path`.
fn setup() -> (TempDir, Store, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let index_path = dir.path().join("idx");
    let hook_index = Index::open(&index_path).unwrap();
    let store = Store::open_with_hook(dir.path().join("store.db"), Box::new(hook_index)).unwrap();
    (dir, store, index_path)
}

fn alpha_word() -> impl Strategy<Value = String> {
    "[a-z]{3,12}"
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, .. ProptestConfig::default() })]

    /// For any item ingested with content containing `word`, searching for
    /// `word` returns that item.
    #[test]
    fn ingested_words_are_findable(word in alpha_word()) {
        let (_dir, store, index_path) = setup();
        let content = format!("a sentence containing the word {word} naturally");
        let item = store.ingest(NewItem::text(content)).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(100));

        // Drop the store to release the hook's writer lock before opening a
        // second Index on the same path (Tantivy permits only one writer per
        // directory at a time; see constraints section in the plan).
        drop(store);

        let index2 = Index::open(&index_path).unwrap();
        let query = Query::parse(&word).unwrap();
        let results = index2.search(&query, SearchOptions::default()).unwrap();

        prop_assert!(
            results.hits.iter().any(|h| h.id == item.id),
            "search for {word:?} should find the ingested item"
        );
    }

    /// Search for a word that wasn't ingested returns zero matches.
    #[test]
    fn unmatched_words_return_empty(content_word in alpha_word(), other_word in alpha_word()) {
        prop_assume!(content_word != other_word);

        let (_dir, store, index_path) = setup();
        let content = format!("a sentence containing the word {content_word} naturally");
        store.ingest(NewItem::text(content)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Drop store to release writer lock before re-opening for search.
        drop(store);

        let index2 = Index::open(&index_path).unwrap();
        let query = Query::parse(&other_word).unwrap();
        let results = index2.search(&query, SearchOptions::default()).unwrap();
        prop_assert_eq!(results.total_matched, 0);
    }
}
