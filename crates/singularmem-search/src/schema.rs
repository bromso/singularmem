//! Tantivy schema definition. The schema is fixed in v0.2.0; future schema
//! changes get migrators that rebuild from `SQLite`.

use tantivy::schema::{Field, Schema, SchemaBuilder, FAST, INDEXED, STORED, STRING, TEXT};

/// Field handles for the v0.2.0 schema. Carried alongside the `Schema` so
/// callers don't have to look up fields by name on every operation.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct Fields {
    pub content: Field,
    pub tags: Field,
    pub source: Field,
    pub id: Field,
    pub created_at: Field,
    pub supersedes: Field,
}

/// Construct the v0.2.0 schema and field handles.
pub fn build_schema() -> (Schema, Fields) {
    let mut b = SchemaBuilder::new();

    // Searchable + stored — the primary search target.
    let content = b.add_text_field("content", TEXT | STORED);

    // STRING (no tokenization) → tag queries are exact-match.
    let tags = b.add_text_field("tags", STRING | STORED);

    // TEXT (tokenized) → partial-match search on source labels.
    let source = b.add_text_field("source", TEXT | STORED);

    // STORED only — used to reconstruct the Item from a hit.
    let id = b.add_text_field("id", STRING | STORED);

    // FAST + INDEXED so a later sub-project can do range filtering by date
    // without re-indexing.
    let created_at = b.add_date_field("created_at", INDEXED | STORED | FAST);

    // STORED only — pointer for revision-aware filtering (deferred).
    let supersedes = b.add_text_field("supersedes", STRING | STORED);

    let schema = b.build();
    (
        schema,
        Fields {
            content,
            tags,
            source,
            id,
            created_at,
            supersedes,
        },
    )
}
