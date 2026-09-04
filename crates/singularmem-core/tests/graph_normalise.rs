//! Entity/predicate normalisation and time-point parsing tests.
//! Spec: `docs/superpowers/specs/2026-09-05-knowledge-graph-14-design.md`
//! § "Data model" → "Normalisation".

use singularmem_core::graph::normalise::{entity_name, predicate};
use singularmem_core::graph::time::parse_point;
use singularmem_core::Error;

#[test]
fn entity_names_normalise() {
    assert_eq!(entity_name("  Singular Mem ").unwrap(), "singular_mem");
    assert_eq!(entity_name("Jonas's  Laptop").unwrap(), "jonass_laptop");
    assert_eq!(entity_name("Tantivy").unwrap(), "tantivy");
    assert_eq!(entity_name("café").unwrap(), "café"); // NFC, lowercase, non-ASCII allowed
    assert!(matches!(
        entity_name("   "),
        Err(Error::Validation {
            field: "entity",
            ..
        })
    ));
    assert!(matches!(
        entity_name(&"x".repeat(257)),
        Err(Error::Validation {
            field: "entity",
            ..
        })
    ));
}

#[test]
fn predicates_normalise_and_restrict() {
    assert_eq!(predicate("Uses").unwrap(), "uses");
    assert_eq!(predicate("Works At").unwrap(), "works_at");
    assert!(matches!(
        predicate("uses-db"),
        Err(Error::Validation {
            field: "predicate",
            ..
        })
    ));
    assert!(matches!(
        predicate("café"),
        Err(Error::Validation {
            field: "predicate",
            ..
        })
    ));
    assert!(matches!(
        predicate(&"p".repeat(65)),
        Err(Error::Validation {
            field: "predicate",
            ..
        })
    ));
}

#[test]
fn time_points_accept_dates_and_timestamps() {
    assert_eq!(
        parse_point("2026-05-16").unwrap().to_string(),
        "2026-05-16T00:00:00Z"
    );
    assert_eq!(
        parse_point("2026-05-16T10:20:30Z").unwrap().to_string(),
        "2026-05-16T10:20:30Z"
    );
    assert!(matches!(
        parse_point("yesterday"),
        Err(Error::Validation {
            field: "timestamp",
            ..
        })
    ));
}
