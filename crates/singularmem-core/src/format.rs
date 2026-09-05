//! On-disk format versioning constants and helpers.
//!
//! The canonical specification lives at `docs/formats/store-v4.md` in the
//! repository root. This module is the in-code anchor — the constant value
//! here MUST match the `singularmem_meta.format_version` row in any store this
//! binary writes.

/// Maximum on-disk format version this binary supports. A store at a higher
/// version causes `Store::open` to fail with `Error::UnsupportedFormatVersion`.
pub const FORMAT_VERSION: &str = "4";

/// Marker constant for the JSONL export schema (`_singularmem_format` field on
/// the meta line of an export). See `docs/formats/store-v4.md` § "Export —
/// `export-v2`".
pub const EXPORT_FORMAT: &str = "export-v2";
