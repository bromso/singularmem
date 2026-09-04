//! Scope paths: validation, normalisation, ancestor expansion, and the
//! filter value threaded through every read surface.
//!
//! Spec: `docs/formats/store-v3.md` § "Scope".

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Maximum number of `/`-separated segments.
pub const MAX_SEGMENTS: usize = 8;
/// Maximum bytes per segment.
pub const MAX_SEGMENT_BYTES: usize = 64;
/// Maximum bytes for the whole normalised path.
pub const MAX_SCOPE_BYTES: usize = 512;

/// Validate and normalise a scope path.
///
/// Lowercases, strips leading/trailing slashes, and rejects empty, `.`,
/// `..`, or non-`[a-z0-9._-]` segments, more than [`MAX_SEGMENTS`]
/// segments, segments over [`MAX_SEGMENT_BYTES`], or a total over
/// [`MAX_SCOPE_BYTES`].
///
/// # Errors
/// `Error::Validation { field: "scope", .. }` describing the first rule broken.
pub fn validate(raw: &str) -> Result<String> {
    let reject = |reason: String| Error::Validation {
        field: "scope",
        reason,
    };
    // Cheap length guard before any allocation or per-segment work below.
    // `+ 2` allows for a leading and trailing slash, which trimming below
    // removes before the real `MAX_SCOPE_BYTES` check on the joined path.
    if raw.len() > MAX_SCOPE_BYTES + 2 {
        return Err(reject(format!(
            "exceeds {MAX_SCOPE_BYTES} bytes (got {} bytes before normalisation)",
            raw.len()
        )));
    }
    let trimmed = raw.trim_matches('/');
    if trimmed.is_empty() {
        return Err(reject("must contain at least one segment".into()));
    }
    let mut segments = Vec::new();
    for seg in trimmed.split('/') {
        if seg.is_empty() {
            return Err(reject("empty segment (double slash)".into()));
        }
        if seg == "." || seg == ".." {
            return Err(reject(format!("segment {seg:?} is not allowed")));
        }
        if seg.len() > MAX_SEGMENT_BYTES {
            return Err(reject(format!("segment exceeds {MAX_SEGMENT_BYTES} bytes")));
        }
        let lower = seg.to_ascii_lowercase();
        let is_valid_byte =
            |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-');
        if !lower.bytes().all(is_valid_byte) {
            return Err(reject(format!(
                "segment {seg:?} contains characters outside [A-Za-z0-9._-]"
            )));
        }
        segments.push(lower);
    }
    if segments.len() > MAX_SEGMENTS {
        return Err(reject(format!("more than {MAX_SEGMENTS} segments")));
    }
    let joined = segments.join("/");
    if joined.len() > MAX_SCOPE_BYTES {
        return Err(reject(format!("exceeds {MAX_SCOPE_BYTES} bytes")));
    }
    Ok(joined)
}

/// Every prefix of a normalised scope, shortest first, ending with the
/// scope itself. Input is assumed normalised (call [`validate`] first).
#[must_use]
pub fn ancestors(scope: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut acc = String::new();
    for seg in scope.split('/') {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(seg);
        out.push(acc.clone());
    }
    out
}

/// A scope filter: `path` plus whether descendants are included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeFilter {
    /// Normalised scope path.
    pub path: String,
    /// `true` → only items whose scope equals `path`; `false` → `path` and
    /// everything beneath it.
    pub exact: bool,
}

impl ScopeFilter {
    /// Descendant-inclusive filter; validates and normalises `path`.
    ///
    /// # Errors
    /// Same as [`validate`].
    pub fn descendants(path: &str) -> Result<Self> {
        Ok(Self {
            path: validate(path)?,
            exact: false,
        })
    }

    /// Exact-match filter; validates and normalises `path`.
    ///
    /// # Errors
    /// Same as [`validate`].
    pub fn exact(path: &str) -> Result<Self> {
        Ok(Self {
            path: validate(path)?,
            exact: true,
        })
    }

    /// Does a stored scope satisfy this filter?
    #[must_use]
    pub fn matches(&self, scope: Option<&str>) -> bool {
        match scope {
            None => false,
            Some(s) if self.exact => s == self.path,
            Some(s) => {
                s == self.path
                    || s.strip_prefix(self.path.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            }
        }
    }

    /// SQL fragment and bound parameters for a `WHERE` clause on `items.scope`.
    /// Uses `ESCAPE '\'` because `_` is a LIKE wildcard and a legal scope byte.
    #[must_use]
    pub(crate) fn sql_clause(&self) -> (&'static str, Vec<String>) {
        if self.exact {
            ("scope = ?", vec![self.path.clone()])
        } else {
            let escaped = self
                .path
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            (
                "(scope = ? OR scope LIKE ? ESCAPE '\\')",
                vec![self.path.clone(), format!("{escaped}/%")],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{validate, ScopeFilter};

    /// A ~10 KB input is rejected by the early length guard before any
    /// per-segment allocation. Any rejection reason is acceptable — this
    /// only proves the guard fires, not which message it produces.
    #[test]
    fn oversized_input_is_rejected() {
        let huge = "a/".repeat(5000);
        assert!(huge.len() > 9000);
        assert!(validate(&huge).is_err());
    }

    /// `sql_clause` is `pub(crate)`, so this test lives here rather than in
    /// the integration test file. `%` and `\` aren't valid scope bytes, so
    /// the filter is built directly rather than through `descendants`
    /// (which validates and would reject them).
    #[test]
    fn sql_clause_escapes_like_wildcards() {
        let filter = ScopeFilter {
            path: "p_q%r\\s".to_string(),
            exact: false,
        };
        let (clause, binds) = filter.sql_clause();
        assert_eq!(clause, "(scope = ? OR scope LIKE ? ESCAPE '\\')");
        assert_eq!(binds.len(), 2);
        assert_eq!(binds[0], filter.path);
        let escaped = &binds[1];
        assert!(escaped.ends_with("/%"), "{escaped:?}");
        assert!(escaped.contains("\\_"), "{escaped:?}");
        assert!(escaped.contains("\\%"), "{escaped:?}");
        assert!(escaped.contains("\\\\"), "{escaped:?}");
    }
}
