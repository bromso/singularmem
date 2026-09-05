//! Principle III.b end-to-end test: ingest -> list -> export -> re-load all
//! work using ONLY the open singularmem-core crate plus stdlib + tempfile.
//!
//! If a future sub-project introduces a hidden dependency on a proprietary
//! component for any of {ingest, get, list, export, revision-walk}, this
//! test fails — either at compile time (missing import) or at assertion time.

use std::collections::HashSet;
use std::io::Cursor;

use singularmem_core::graph::{NewFact, NewObject};
use singularmem_core::{Item, NewItem, Store};
use tempfile::TempDir;

#[derive(serde::Deserialize)]
struct ItemLine {
    #[serde(rename = "_kind")]
    kind: String,
    #[serde(flatten)]
    item: Item,
}

/// Loader-facing shape of an `export-v2` entity line — deserialised the way
/// a third party would, straight off `docs/formats/store-v4.md`.
#[derive(serde::Deserialize)]
struct EntityLine {
    #[serde(rename = "_kind")]
    line_kind: String,
    id: String,
    name: String,
    normalised_name: String,
    #[serde(default, rename = "kind")]
    entity_kind: Option<String>,
    created_at: String,
}

#[derive(serde::Deserialize)]
struct SubjectRef {
    id: String,
    name: String,
}

/// Loader-facing shape of an `export-v2` fact line.
#[derive(serde::Deserialize)]
struct FactLine {
    #[serde(rename = "_kind")]
    line_kind: String,
    id: String,
    subject: SubjectRef,
    predicate: String,
    object: serde_json::Value,
    #[serde(default)]
    valid_from: Option<String>,
    #[serde(default)]
    valid_to: Option<String>,
    confidence: f32,
    #[serde(default)]
    source_item_id: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    supersedes: Option<String>,
    recorded_at: String,
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

    let mut keyed = NewItem::text("keyed note");
    keyed.external_id = Some("test:keyed".into());
    let _keyed = store.ingest(keyed).unwrap();

    let mut scoped = NewItem::text("scoped note");
    scoped.scope = Some("Rt/Scope".into());
    let _scoped = store.ingest(scoped).unwrap();

    let originals: Vec<Item> = store.list().unwrap().map(|r| r.unwrap()).collect();
    assert_eq!(originals.len(), 6);

    // Graph: one entity-object fact, one value-object fact carrying a
    // source item, then invalidate the entity-object fact — three fact
    // revisions in total, over two entities (singularmem, tantivy).
    let triple_fact = store
        .add_fact(NewFact::triple("singularmem", "uses", "tantivy"))
        .unwrap();
    let value_fact = store
        .add_fact(NewFact {
            subject: "singularmem".into(),
            subject_kind: None,
            predicate: "confidence_note".into(),
            object: NewObject::Value("battle-tested".into()),
            valid_from: None,
            valid_to: None,
            confidence: 0.9,
            source_item_id: Some(sourced.id),
            scope: None,
        })
        .unwrap();
    let invalidated = store
        .invalidate_fact(
            "singularmem",
            "uses",
            &NewObject::Entity {
                name: "tantivy".into(),
                kind: None,
            },
            None,
            None,
        )
        .unwrap();

    // Export to a buffer.
    let mut buf = Vec::new();
    store.export(&mut buf).expect("export");

    // Manually re-parse the JSONL: skip meta line, parse items.
    let text = String::from_utf8(buf.clone()).expect("utf8");
    let lines: Vec<&str> = text.lines().collect();
    // 1 meta + 6 items + 2 entities (singularmem, tantivy) + 3 fact revisions.
    assert_eq!(lines.len(), 12, "1 meta + 6 items + 2 entities + 3 facts");

    let meta: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(meta["_singularmem_format"], "export-v2");
    assert_eq!(meta["store_format_version"], "4");

    // Line kinds appear in blocks, in order: meta, item x N, entity x M,
    // fact x K.
    let kinds: Vec<String> = lines
        .iter()
        .map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            v["_kind"].as_str().unwrap().to_string()
        })
        .collect();
    let mut expected_kinds = vec!["meta".to_string()];
    expected_kinds.extend(std::iter::repeat("item".to_string()).take(6));
    expected_kinds.extend(std::iter::repeat("entity".to_string()).take(2));
    expected_kinds.extend(std::iter::repeat("fact".to_string()).take(3));
    assert_eq!(kinds, expected_kinds);

    // Parse each item line as a serde-deserialised Item to prove the wire
    // shape is round-trip-compatible with the type itself.
    let parsed_items: Vec<Item> = lines[1..7]
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

    // Cross-check: the external_id survived.
    assert!(parsed_items
        .iter()
        .any(|i| i.external_id.as_deref() == Some("test:keyed")));

    // Cross-check: the scope survived, normalised.
    assert!(parsed_items
        .iter()
        .any(|i| i.scope.as_deref() == Some("rt/scope")));

    // Entity lines: singularmem and tantivy. Both are resolved inside the
    // same `add_fact` call and so share one `created_at`, in which case the
    // spec's tie-break is by id — not by which of the two was resolved
    // first — so only the *set* of names, not their order, is asserted
    // here; ordering itself is checked generically below.
    let entity_lines: Vec<EntityLine> = lines[7..9]
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for entity in &entity_lines {
        assert_eq!(entity.line_kind, "entity");
        assert!(entity.entity_kind.is_none());
        assert_eq!(entity.normalised_name, entity.name.to_lowercase());
    }
    let entity_names: HashSet<&str> = entity_lines.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        entity_names,
        ["singularmem", "tantivy"].into_iter().collect()
    );

    // Ordering invariant: (created_at, id) is non-decreasing across the
    // entity block.
    assert!(entity_lines[0].created_at <= entity_lines[1].created_at);
    if entity_lines[0].created_at == entity_lines[1].created_at {
        assert!(entity_lines[0].id <= entity_lines[1].id);
    }

    // Fact lines: parse each one and check the shapes the spec promises.
    let fact_lines: Vec<FactLine> = lines[9..12]
        .iter()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("parse {line:?}: {e}")))
        .collect();
    for fact in &fact_lines {
        assert_eq!(fact.line_kind, "fact");
        assert!(!fact.subject.id.is_empty());
        assert!(!fact.recorded_at.is_empty());
    }
    // Ordering invariant: (recorded_at, id) is non-decreasing across the
    // fact block.
    for pair in fact_lines.windows(2) {
        assert!(pair[0].recorded_at <= pair[1].recorded_at);
        if pair[0].recorded_at == pair[1].recorded_at {
            assert!(pair[0].id <= pair[1].id);
        }
    }
    let fact_ids: HashSet<&str> = fact_lines.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(fact_ids.len(), 3, "three distinct fact revisions");
    assert!(fact_ids.contains(triple_fact.id.to_string().as_str()));
    assert!(fact_ids.contains(value_fact.id.to_string().as_str()));
    assert!(fact_ids.contains(invalidated.id.to_string().as_str()));

    // The entity-object fact's `object` is `{"entity": {"id", "name"}}`.
    let triple_line = fact_lines
        .iter()
        .find(|f| f.id == triple_fact.id.to_string())
        .expect("triple fact line present");
    assert_eq!(triple_line.subject.name, "singularmem");
    assert_eq!(triple_line.predicate, "uses");
    assert_eq!(triple_line.object["entity"]["name"], "tantivy");
    assert!(triple_line.object.get("value").is_none());
    assert!(triple_line.source_item_id.is_none());
    assert!(triple_line.scope.is_none());
    assert!(triple_line.supersedes.is_none());

    // The value-object fact's `object` is `{"value": "…"}`, and it carries
    // its source item id.
    let value_line = fact_lines
        .iter()
        .find(|f| f.id == value_fact.id.to_string())
        .expect("value fact line present");
    assert_eq!(value_line.object["value"], "battle-tested");
    assert!(value_line.object.get("entity").is_none());
    assert_eq!(
        value_line.source_item_id.as_deref(),
        Some(sourced.id.to_string()).as_deref()
    );
    assert!((value_line.confidence - 0.9).abs() < f32::EPSILON);

    // The invalidating revision supersedes the original triple fact and
    // carries a `valid_to`.
    let invalidated_line = fact_lines
        .iter()
        .find(|f| f.id == invalidated.id.to_string())
        .expect("invalidated fact line present");
    assert_eq!(
        invalidated_line.supersedes.as_deref(),
        Some(triple_fact.id.to_string()).as_deref()
    );
    assert!(invalidated_line.valid_to.is_some());
    assert!(invalidated_line.valid_from.is_none());

    // A loader that only understands `meta`/`item` and ignores every other
    // `_kind` (per the export-v2 rule) still reads all N items.
    let v1_style_items: Vec<Item> = lines
        .iter()
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            if v["_kind"] == "item" {
                let parsed: ItemLine = serde_json::from_str(line).unwrap();
                Some(parsed.item)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(v1_style_items, originals);

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
