//! Shared `--project` filter used by every JSONL transcript source.

use std::path::{Path, PathBuf};

/// A resolved `--project` filter: the raw path, its canonical form when it
/// resolves, and a one-entry memo so a transcript whose thousands of lines
/// share one `cwd` costs a single `canonicalize` call.
#[derive(Debug)]
pub struct ProjectFilter {
    raw: PathBuf,
    canonical: Option<PathBuf>,
    memo: Option<(String, bool)>,
}

impl ProjectFilter {
    pub fn new(raw: &Path) -> Self {
        Self {
            raw: raw.to_path_buf(),
            canonical: raw.canonicalize().ok(),
            memo: None,
        }
    }

    /// True when `cwd` names the same directory as the filter: equal as raw
    /// paths, or equal once both sides canonicalize successfully. A `cwd`
    /// that no longer exists on this machine can still match by raw path.
    pub fn matches(&mut self, cwd: Option<&str>) -> bool {
        let Some(cwd) = cwd else { return false };
        if let Some((seen, verdict)) = &self.memo {
            if seen == cwd {
                return *verdict;
            }
        }
        let verdict = self.raw.as_path() == Path::new(cwd)
            || match (&self.canonical, Path::new(cwd).canonicalize().ok()) {
                (Some(a), Some(b)) => *a == b,
                _ => false,
            };
        self.memo = Some((cwd.to_string(), verdict));
        verdict
    }
}

/// `<prefix>/<basename of dir>` validated as a scope, warning on failure.
pub fn derive_scope(prefix: &str, dir: &str) -> Option<String> {
    let Some(base) = Path::new(dir).file_name() else {
        tracing::warn!(dir, "directory has no basename; item left unscoped");
        return None;
    };
    let Some(base) = base.to_str() else {
        tracing::warn!(dir, "basename is not valid UTF-8; item left unscoped");
        return None;
    };
    match singularmem_core::scope::validate(&format!("{prefix}/{base}")) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!(dir, error = %e, "basename is not a valid scope segment; item left unscoped");
            None
        }
    }
}
