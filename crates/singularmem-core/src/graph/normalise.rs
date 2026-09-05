//! Entity and predicate name normalisation (spec § "Normalisation").

use unicode_normalization::UnicodeNormalization;

use crate::error::{Error, Result};

/// Maximum length in bytes of a normalised entity name.
pub const MAX_ENTITY_BYTES: usize = 256;
/// Maximum length in bytes of a normalised predicate.
pub const MAX_PREDICATE_BYTES: usize = 64;

/// NFC-normalise, trim, lowercase, strip apostrophes, and collapse internal
/// whitespace runs to a single `_`. Shared by [`entity_name`] and
/// [`predicate`]; each applies its own length/charset rules on top.
fn base(raw: &str) -> String {
    let nfc: String = raw.nfc().collect();
    let lowered = nfc.trim().to_lowercase().replace('\'', "");
    lowered.split_whitespace().collect::<Vec<_>>().join("_")
}

/// Normalise an entity name.
///
/// # Errors
/// `Validation { field: "entity" }` when empty or over [`MAX_ENTITY_BYTES`].
pub fn entity_name(raw: &str) -> Result<String> {
    let n = base(raw);
    if n.is_empty() {
        return Err(Error::Validation {
            field: "entity",
            reason: "must be non-empty".into(),
        });
    }
    if n.len() > MAX_ENTITY_BYTES {
        return Err(Error::Validation {
            field: "entity",
            reason: format!("exceeds {MAX_ENTITY_BYTES} bytes after normalisation"),
        });
    }
    Ok(n)
}

/// Normalise a predicate.
///
/// # Errors
/// `Validation { field: "predicate" }` when empty, over
/// [`MAX_PREDICATE_BYTES`], or outside `[a-z0-9_]`.
pub fn predicate(raw: &str) -> Result<String> {
    let n = base(raw);
    if n.is_empty() {
        return Err(Error::Validation {
            field: "predicate",
            reason: "must be non-empty".into(),
        });
    }
    if n.len() > MAX_PREDICATE_BYTES {
        return Err(Error::Validation {
            field: "predicate",
            reason: format!("exceeds {MAX_PREDICATE_BYTES} bytes"),
        });
    }
    if !n
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(Error::Validation {
            field: "predicate",
            reason: "must match [a-z0-9_]+ after normalisation".into(),
        });
    }
    Ok(n)
}
