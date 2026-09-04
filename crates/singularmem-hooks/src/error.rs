//! Error type for the hooks crate. Every I/O-adjacent variant names the
//! path involved.

use std::path::PathBuf;

/// Result alias used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// All errors this crate surfaces.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// A config file exists but is not valid JSON. Never overwritten.
    #[error("invalid JSON in {path}: {source}")]
    InvalidJson {
        /// The unparsable config file.
        path: PathBuf,
        /// Underlying parse error.
        #[source]
        source: serde_json::Error,
    },

    /// Reading or writing a config file failed.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// The path involved.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },

    /// Neither `HOME` nor `USERPROFILE` is set, and no `--project`
    /// override was given, so the default config path cannot be derived.
    #[error("could not determine home directory: neither HOME nor USERPROFILE is set")]
    NoHome,
}
