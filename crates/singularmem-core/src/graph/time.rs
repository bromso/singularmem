//! Graph time helpers: parse user-supplied time points (`YYYY-MM-DD`, or
//! RFC 3339) and render a [`Timestamp`] in the one shape the graph tables
//! store.

use jiff::Timestamp;

use crate::error::{Error, Result};

/// `strftime` pattern behind [`to_sql`]: UTC, fixed nine-digit fraction.
const SQL_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.9fZ";

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

/// Render `ts` in the fixed-width form the graph tables store:
/// `YYYY-MM-DDTHH:MM:SS.fffffffffZ` — always UTC, always exactly nine
/// fractional digits, always 30 characters.
///
/// [`Timestamp`]'s own `Display` trims trailing zeros, so a clock-minted
/// instant prints as `2026-09-05T03:12:00.788067Z` while a user-supplied
/// one prints as `2026-09-05T03:12:00Z`. `SQLite` compares those as text,
/// and `'.'` (0x2E) sorts before `'Z'` (0x5A), so the sub-second value
/// would compare *less* than the whole-second one it actually follows.
/// Padding every stored and every bound value to nine digits makes string
/// order and chronological order the same thing again.
///
/// Reads stay liberal: parse with [`str::parse::<Timestamp>`], which
/// accepts any RFC 3339 precision, including rows written before this.
#[must_use]
pub fn to_sql(ts: Timestamp) -> String {
    ts.strftime(SQL_FORMAT).to_string()
}

#[cfg(test)]
mod tests {
    use super::{parse_point, to_sql};

    #[test]
    fn to_sql_is_fixed_width_utc_with_nine_fractional_digits() {
        for raw in [
            "2026-09-05T03:12:00.788067Z",
            "2026-09-05T03:12:00Z",
            "2026-05-16",
            "2026-09-05T05:12:00+02:00",
        ] {
            let s = to_sql(parse_point(raw).unwrap());
            assert_eq!(s.len(), 30, "{raw} -> {s}");
            assert!(s.ends_with('Z'), "{raw} -> {s}");
            assert_eq!(&s[19..20], ".", "{raw} -> {s}");
        }
        assert_eq!(
            to_sql(parse_point("2026-05-16").unwrap()),
            "2026-05-16T00:00:00.000000000Z"
        );
        assert_eq!(
            to_sql(parse_point("2026-09-05T05:12:00+02:00").unwrap()),
            "2026-09-05T03:12:00.000000000Z",
            "always normalised to UTC"
        );
    }

    /// The bug this format exists to prevent: `Display` order is not time
    /// order within a second, `to_sql` order is.
    #[test]
    fn fixed_width_restores_string_order_as_time_order() {
        let earlier = parse_point("2026-09-05T03:12:00Z").unwrap();
        let later = parse_point("2026-09-05T03:12:00.788Z").unwrap();
        assert!(earlier < later);
        assert!(
            later.to_string() < earlier.to_string(),
            "Display order is broken, which is why to_sql exists"
        );
        assert!(to_sql(earlier) < to_sql(later));
    }
}
