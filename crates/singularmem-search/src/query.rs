//! Query construction: text parsing (Tantivy `QueryParser`) and programmatic builder.

use tantivy::query::{
    BooleanQuery, ConstScoreQuery, Occur, Query as TantivyQuery, QueryParser, QueryParserError,
    TermQuery,
};
use tantivy::schema::IndexRecordOption;
use tantivy::Term;

use singularmem_core::ScopeFilter;

use crate::error::{Error, Result};
use crate::schema::{build_schema, Fields};

/// The `Must` clause that restricts a query to `filter`.
///
/// An exact filter matches the item's own `scope`; a descendant-inclusive one
/// matches `scope_ancestors`, which carries one value per prefix of the
/// item's scope (so the subtree test is a single term lookup).
///
/// Wrapped in a [`ConstScoreQuery`] scored at `0.0` so this clause narrows
/// the document set without perturbing the BM25 score of the query it is
/// combined with — a scoped and an unscoped search for the same terms must
/// rank identically among the documents both return.
pub(crate) fn scope_clause(fields: Fields, filter: &ScopeFilter) -> Box<dyn TantivyQuery> {
    let field = if filter.exact {
        fields.scope
    } else {
        fields.scope_ancestors
    };
    let term = Term::from_field_text(field, &filter.path);
    let term_query = TermQuery::new(term, IndexRecordOption::Basic);
    Box::new(ConstScoreQuery::new(Box::new(term_query), 0.0))
}

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

/// True when `name` could plausibly be a field the caller meant to address:
/// an ASCII identifier (letter or `_` first, then letters, digits, `_`).
/// Anything else — `10` from `10:30`, `2` from `2:1` — is prose.
fn looks_like_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Query {
    /// Parse a Tantivy QueryParser-style query string. Default search fields are
    /// `content` and `source` (bare terms match either); `tags` requires the
    /// explicit `tags:` prefix to avoid accidental matches.
    ///
    /// Natural-language input is accepted: when the string is not valid
    /// query syntax (a stray `:`, `-`, `(` or quote — common in questions),
    /// the parser falls back to Tantivy's lenient mode, which keeps every
    /// clause it could read and drops the malformed ones — not only stray
    /// operators, but anything the strict parser couldn't make sense of.
    /// The fallback applies to genuine syntax errors and to `field:` prefixes
    /// whose "field name" cannot be an identifier — `10:30`, `2:1` — which
    /// only occur in prose. A `field:` prefix that does look like an
    /// identifier but names a field the schema doesn't have (`titel:foo`)
    /// is returned as an error rather than silently reinterpreted.
    ///
    /// # Errors
    /// Returns `Error::QueryParse` when the strict parse fails because of an
    /// unknown or unindexed identifier-like field, or when the input is a
    /// syntax error but not a single searchable term survives lenient
    /// parsing — for example an input made of operators alone.
    pub fn parse(query_str: &str) -> Result<Self> {
        let (schema, fields) = build_schema();
        // Construct a throwaway in-RAM index tied to the schema. The actual Index
        // construction reuses the same schema, so semantics match.
        let temp_index = tantivy::Index::create_in_ram(schema);
        let parser = QueryParser::for_index(&temp_index, vec![fields.content, fields.source]);
        let strict = parser.parse_query(query_str);
        let inner = match strict {
            Ok(q) => q,
            Err(strict_err)
                if matches!(strict_err, QueryParserError::SyntaxError(_))
                    || matches!(&strict_err, QueryParserError::FieldDoesNotExist(name)
                        if !looks_like_identifier(name)) =>
            {
                let (lenient, dropped) = parser.parse_query_lenient(query_str);
                let mut terms = 0usize;
                lenient.query_terms(&mut |_, _| terms += 1);
                if terms == 0 {
                    return Err(Error::QueryParse(format!("{strict_err}")));
                }
                if !dropped.is_empty() {
                    tracing::warn!(
                        query = query_str,
                        ?dropped,
                        "lenient query parsing dropped malformed clauses"
                    );
                }
                lenient
            }
            Err(strict_err) => return Err(Error::QueryParse(format!("{strict_err}"))),
        };
        Ok(Self { inner })
    }

    /// Wrap this query so only documents matching `filter` are returned.
    ///
    /// `Index::search` applies `SearchOptions::scope` itself, so this is for
    /// SDK callers composing a query by hand.
    #[must_use]
    pub fn scoped(self, filter: &ScopeFilter) -> Self {
        let (_schema, fields) = build_schema();
        Self {
            inner: Box::new(BooleanQuery::new(vec![
                (Occur::Must, self.inner),
                (Occur::Must, scope_clause(fields, filter)),
            ])),
        }
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
