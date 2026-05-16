//! Principle III.b end-to-end test: ingest -> list -> export -> re-load all
//! work using ONLY the open singularmem-core crate plus stdlib + tempfile.
//!
//! If a future sub-project introduces a hidden dependency on a proprietary
//! component for any of {ingest, get, list, export, revision-walk}, this
//! test fails — either at compile time (missing import) or at assertion time.

use std::collections::HashSet;
use std::io::Cursor;

use singularmem_core::{Item, NewItem, Store};
use tempfile::TempDir;

#[derive(serde::Deserialize)]
struct ItemLine {
    #[serde(rename = "_kind")]
    kind: String,
    #[serde(flatten)]
    item: Item,
}

#[test]
fn open_core_only_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.db");
    let store = Store::open(&path).expect("open fresh");

    // Ingest a varied sample: plain, tagged, sourced, with metadata, with supersedes.
    let plain = store.ingest(NewItem::text("plain note")).unwrap();

    let mut tagged = NewItem::text("with tags");
    tagged.tags = vec!["work".into(), "decision".into()];
    let tagged = store.ingest(tagged).unwrap();

    let mut sourced = NewItem::text("from a source");
    sourced.source = Some("conversation:abc-123".into());
    sourced.metadata = serde_json::json!({"project": "alpha", "priority": 2});
    let sourced = store.ingest(sourced).unwrap();

    let mut correction = NewItem::text("corrected note");
    correction.supersedes = Some(plain.id);
    let correction = store.ingest(correction).unwrap();

    let originals: Vec<Item> = store.list().unwrap().map(|r| r.unwrap()).collect();
    assert_eq!(originals.len(), 4);

    // Export to a buffer.
    let mut buf = Vec::new();
    store.export(&mut buf).expect("export");

    // Manually re-parse the JSONL: skip meta line, parse items.
    let text = String::from_utf8(buf.clone()).expect("utf8");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 5, "1 meta + 4 items");

    let meta: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(meta["_singularmem_format"], "export-v1");
    assert_eq!(meta["store_format_version"], "1");

    // Parse each item line as a serde-deserialised Item to prove the wire
    // shape is round-trip-compatible with the type itself.
    let parsed_items: Vec<Item> = lines[1..]
        .iter()
        .map(|line| {
            let parsed: ItemLine =
                serde_json::from_str(line).unwrap_or_else(|e| panic!("parse {line:?}: {e}"));
            assert_eq!(parsed.kind, "item");
            parsed.item
        })
        .collect();

    // Assert exact equality with the original list.
    assert_eq!(parsed_items, originals);

    // Cross-check: the supersedes pointer survived.
    let correction_via_export = parsed_items
        .iter()
        .find(|i| i.id == correction.id)
        .expect("correction in export");
    assert_eq!(correction_via_export.supersedes, Some(plain.id));

    // Cross-check: the JSON metadata survived.
    let sourced_via_export = parsed_items
        .iter()
        .find(|i| i.id == sourced.id)
        .expect("sourced in export");
    assert_eq!(
        sourced_via_export.metadata,
        serde_json::json!({"project": "alpha", "priority": 2})
    );
    assert_eq!(
        sourced_via_export.source.as_deref(),
        Some("conversation:abc-123")
    );

    // Cross-check: tag set survived (sorted-deduped).
    let tagged_via_export = parsed_items
        .iter()
        .find(|i| i.id == tagged.id)
        .expect("tagged in export");
    let tag_set: HashSet<&str> = tagged_via_export.tags.iter().map(String::as_str).collect();
    assert_eq!(tag_set, ["work", "decision"].into_iter().collect());

    // Last sanity check: the export is deterministic byte-for-byte across
    // two runs of the same store. (Cannot include exported_at in this
    // assertion because it changes on each run.)
    let mut buf2 = Vec::new();
    store.export(&mut Cursor::new(&mut buf2)).expect("export 2");
    // Strip the meta lines (they contain timestamps); compare the rest.
    let lines1: Vec<&str> = std::str::from_utf8(&buf).unwrap().lines().collect();
    let lines2: Vec<&str> = std::str::from_utf8(&buf2).unwrap().lines().collect();
    assert_eq!(&lines1[1..], &lines2[1..]);
}
