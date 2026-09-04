//! Parse user-supplied time points: `YYYY-MM-DD` (midnight UTC) or RFC 3339.

use jiff::Timestamp;

use crate::error::{Error, Result};

/// Parse a time point given as either a bare date (`YYYY-MM-DD`, expanded to
/// midnight UTC) or a full RFC 3339 timestamp.
///
/// # Errors
/// `Validation { field: "timestamp" }` when neither form parses.
pub fn parse_point(raw: &str) -> Result<Timestamp> {
    let s = raw.trim();
    if let Ok(t) = s.parse::<Timestamp>() {
        return Ok(t);
    }
    if let Ok(d) = s.parse::<jiff::civil::Date>() {
        return d
            .to_zoned(jiff::tz::TimeZone::UTC)
            .map(|z| z.timestamp())
            .map_err(|e| Error::Validation {
                field: "timestamp",
                reason: e.to_string(),
            });
    }
    Err(Error::Validation {
        field: "timestamp",
        reason: format!("{s:?} is neither YYYY-MM-DD nor RFC 3339"),
    })
}
