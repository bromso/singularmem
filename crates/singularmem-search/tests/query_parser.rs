//! Tests for `Query::parse` and the `QueryBuilder` API.

use singularmem_search::{Field, Query, QueryBuilder};

#[test]
fn parse_bare_term() {
    let _q = Query::parse("decision").expect("parse single term");
}

#[test]
fn parse_required_plus_excluded() {
    let _q = Query::parse("+decision -draft").expect("parse +req -excl");
}

#[test]
fn parse_field_value() {
    let _q = Query::parse("tags:work").expect("parse field:value");
}

#[test]
fn parse_phrase() {
    let _q = Query::parse("\"deferred to v0.3\"").expect("parse phrase");
}

#[test]
fn parse_boolean() {
    let _q = Query::parse("(decision OR fix) AND -draft").expect("parse boolean");
}

#[test]
fn parse_malformed_errors() {
    let result = Query::parse("tags:");
    assert!(result.is_err(), "trailing colon should not parse");
}

#[test]
fn query_builder_constructs_single_term() {
    let _q = QueryBuilder::new()
        .term(Field::Content, "decision")
        .build();
}

#[test]
fn query_builder_combines_must_and_must_not() {
    let q = QueryBuilder::new()
        .must(QueryBuilder::new().term(Field::Content, "decision").build())
        .must_not(QueryBuilder::new().term(Field::Content, "draft").build())
        .build();
    let _ = q;
}
