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
fn query_builder_constructs_single_term() {
    let _q = QueryBuilder::new().term(Field::Content, "decision").build();
}

#[test]
fn query_builder_combines_must_and_must_not() {
    let q = QueryBuilder::new()
        .must(QueryBuilder::new().term(Field::Content, "decision").build())
        .must_not(QueryBuilder::new().term(Field::Content, "draft").build())
        .build();
    let _ = q;
}

#[test]
fn natural_language_question_with_operator_characters_parses() {
    // Real LongMemEval questions that the strict parser rejected: a colon
    // after a word looks like a field prefix, a lone dash like a negation,
    // and unbalanced quotes like an unterminated phrase.
    for q in [
        "What is the order of the three events: 'I signed up', 'I used a coupon', and 'I redeemed'?",
        "I was going through our previous conversation - what did Borges say about the center?",
        "How many weeks in total: reading 'The Nightingale' and listening to 'Sapiens'?",
        "Which three events happened (in order) from first to last:",
    ] {
        Query::parse(q).unwrap_or_else(|e| panic!("{q:?} should parse leniently: {e}"));
    }
}

#[test]
fn operators_alone_still_fail_to_parse() {
    for q in ["tags:", ":::", "- -", "()"] {
        assert!(Query::parse(q).is_err(), "{q:?} has no searchable term");
    }
}

#[test]
fn unknown_field_is_still_an_error() {
    // Unlike a syntax error, an unknown field is a well-formed query the
    // schema simply can't satisfy — it must not be papered over by the
    // lenient fallback, which would silently reinterpret `titel:foo` as an
    // unqualified term against the default fields and return results.
    for q in [
        "decision titel:foo",
        "+decision +titel:foo",
        "sqlite -titel:foo",
    ] {
        assert!(Query::parse(q).is_err(), "{q:?} names an unknown field");
    }
}

#[test]
fn clock_times_and_ratios_are_prose_not_fields() {
    // Tantivy reads `10:30` as field `10` with value `30`; that "field" is
    // not an identifier, so it is natural language and parses leniently.
    for q in [
        "What time did I say the meeting was, 10:30 or 11:00?",
        "I have a meeting at 3:00pm about the 2:1 ratio",
    ] {
        Query::parse(q).unwrap_or_else(|e| panic!("{q:?} should parse leniently: {e}"));
    }
}
