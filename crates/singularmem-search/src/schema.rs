//! Tantivy schema definition. The schema is at v0.3.0; schema changes are a
//! breaking sidecar change — `Index::open` reports `IndexSchemaMismatch` and
//! the sidecar is rebuilt from `SQLite` by `singularmem reindex`.

use tantivy::schema::{Field, Schema, SchemaBuilder, FAST, INDEXED, STORED, STRING, TEXT};

/// Field handles for the v0.3.0 schema. Carried alongside the `Schema` so
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
    pub scope: Field,
    pub scope_ancestors: Field,
}

/// Construct the v0.3.0 schema and field handles.
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

    // STRING (no tokenization) → exact-match on the item's own scope path.
    let scope = b.add_text_field("scope", STRING | STORED);

    // STRING, multi-valued: one value per prefix of the item's scope, so a
    // descendant-inclusive filter is a single term lookup. Not stored — the
    // ancestors are derivable from `scope`.
    let scope_ancestors = b.add_text_field("scope_ancestors", STRING);

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
            scope,
            scope_ancestors,
        },
    )
}
