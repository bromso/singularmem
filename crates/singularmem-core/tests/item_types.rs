//! Tests for the `Item`, `ItemId`, and `NewItem` types and their validation.

use singularmem_core::ItemId;
use std::str::FromStr;

#[test]
fn item_id_parse_and_display_round_trip() {
    let s = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let id = ItemId::from_str(s).expect("parse");
    assert_eq!(id.to_string(), s.to_uppercase());
}

#[test]
fn item_id_parse_lowercase_accepted() {
    let upper = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let lower = upper.to_lowercase();
    let id_upper = ItemId::from_str(upper).expect("upper parse");
    let id_lower = ItemId::from_str(&lower).expect("lower parse");
    assert_eq!(id_upper, id_lower);
    assert_eq!(id_upper.to_string(), upper); // display always uppercase
}

#[test]
fn item_id_parse_garbage_errors() {
    assert!(ItemId::from_str("not-a-ulid").is_err());
    assert!(ItemId::from_str("").is_err());
    assert!(ItemId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAVX").is_err()); // 27 chars
}
