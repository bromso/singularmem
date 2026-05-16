//! Memory item types: `ItemId`, `Item`, `NewItem`.
//!
//! `Item` is the persisted form (immutable, has an assigned ID and timestamp).
//! `NewItem` is the to-be-ingested form — the type system prevents callers
//! from setting an ID that the store does not control.
//!
//! Full implementation arrives in Task 4 (Phase B).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Stable, opaque identifier for a memory item (stub — Task 4 fills this in).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ItemId(Ulid);

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for ItemId {
    type Err = ulid::DecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ulid::from_string(s).map(Self)
    }
}

/// A persisted memory item (stub — Task 4 fills this in).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item;

/// The "to be ingested" form of an item (stub — Task 4 fills this in).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewItem;
