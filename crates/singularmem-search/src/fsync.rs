//! `fsync` helpers shared by the vector sidecar's two writers,
//! [`crate::vector_index`] (compaction's temp-file-plus-rename) and
//! [`crate::vector_journal`] (the journal's lazy creation).
//!
//! A file's own `fsync` only makes its *contents* durable. The directory
//! entry that names it — a `create` or a `rename` — needs the *directory*
//! fsynced as well, or a power cut can leave the name missing while the
//! bytes are safely on the platter.

use std::fs::File;
use std::path::Path;

use crate::error::{Error, Result};

/// `fsync` the file at `path`, so a rename over the original cannot expose a
/// name pointing at unflushed contents.
pub fn sync_file(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(Error::Io)
}

/// `fsync` the directory at `path` so the creations and renames within it are
/// durable — this is what makes the "either old pair or new pair, never a
/// mix" ordering hold across a power cut, so a failure is reported rather
/// than swallowed.
///
/// Windows has no directory handle to sync; there the entries' ordering is
/// the filesystem's business and the failure to open is not ours to report.
pub fn sync_dir(path: &Path) -> Result<()> {
    match File::open(path).and_then(|d| d.sync_all()) {
        Ok(()) => Ok(()),
        Err(_) if cfg!(windows) => Ok(()),
        Err(e) => Err(Error::Io(e)),
    }
}
