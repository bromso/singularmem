//! Query construction: text parsing (Tantivy `QueryParser`) and programmatic builder.

use tantivy::query::{BooleanQuery, Occur, Query as TantivyQuery, QueryParser, TermQuery};
use tantivy::schema::IndexRecordOption;
use tantivy::Term;

use crate::error::{Error, Result};
use crate::schema::build_schema;

/// Schema field for `QueryBuilder::term`.
#[derive(Copy, Clone, Debug)]
pub enum Field {
    /// The main textual body of an item (`content` field in the index).
    Content,
    /// Free-form tag labels attached to an item (`tags` field in the index).
    Tags,
    /// Optional provenance string for an item (`source` field in the index).
    Source,
}

/// A parsed (or programmatically constructed) search query. Opaque wrapper around
/// a Tantivy `Box<dyn Query>` so callers don't need to depend on `tantivy::query`.
pub struct Query {
    pub(crate) inner: Box<dyn TantivyQuery>,
}

impl Query {
    /// Parse a Tantivy QueryParser-style query string. Default search fields are
    /// `content` and `source` (bare terms match either); `tags` requires the
    /// explicit `tags:` prefix to avoid accidental matches.
    ///
    /// # Errors
    /// Returns `Error::QueryParse` for malformed syntax.
    pub fn parse(query_str: &str) -> Result<Self> {
        let (schema, fields) = build_schema();
        // Construct a throwaway in-RAM index tied to the schema. The actual Index
        // construction reuses the same schema, so semantics match.
        let temp_index = tantivy::Index::create_in_ram(schema);
        let parser = QueryParser::for_index(&temp_index, vec![fields.content, fields.source]);
        let inner = parser
            .parse_query(query_str)
            .map_err(|e| Error::QueryParse(format!("{e}")))?;
        Ok(Self { inner })
    }
}

/// Programmatic query builder for SDK consumers who don't want to construct
/// query strings.
#[derive(Default)]
pub struct QueryBuilder {
    must: Vec<Box<dyn TantivyQuery>>,
    must_not: Vec<Box<dyn TantivyQuery>>,
    should: Vec<Box<dyn TantivyQuery>>,
}

impl QueryBuilder {
    /// Create a new empty `QueryBuilder`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a single-term query against the named field.
    #[must_use]
    pub fn term(mut self, field: Field, value: impl Into<String>) -> Self {
        let (_schema, fields) = build_schema();
        let tantivy_field = match field {
            Field::Content => fields.content,
            Field::Tags => fields.tags,
            Field::Source => fields.source,
        };
        let term = Term::from_field_text(tantivy_field, &value.into());
        let query = TermQuery::new(term, IndexRecordOption::WithFreqsAndPositions);
        self.must.push(Box::new(query));
        self
    }

    /// Compose with an existing Query as required (must match).
    #[must_use]
    pub fn must(mut self, q: Query) -> Self {
        self.must.push(q.inner);
        self
    }

    /// Compose with an existing Query as excluded (must not match).
    #[must_use]
    pub fn must_not(mut self, q: Query) -> Self {
        self.must_not.push(q.inner);
        self
    }

    /// Compose with an existing Query as optional (boosts score; doesn't filter).
    #[must_use]
    pub fn should(mut self, q: Query) -> Self {
        self.should.push(q.inner);
        self
    }

    /// Build the final Query.
    #[must_use]
    pub fn build(self) -> Query {
        let mut clauses: Vec<(Occur, Box<dyn TantivyQuery>)> = Vec::new();
        for q in self.must {
            clauses.push((Occur::Must, q));
        }
        for q in self.must_not {
            clauses.push((Occur::MustNot, q));
        }
        for q in self.should {
            clauses.push((Occur::Should, q));
        }
        let boolean = BooleanQuery::new(clauses);
        Query {
            inner: Box::new(boolean),
        }
    }
}
