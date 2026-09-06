//! Append-only journal of vectors added since the last compaction of the
//! `USearch` index. Format documented in `docs/formats/vectors-v2.md`.
//!
//! Wired into `VectorIndex::commit`/`open` in sub-project 17 Task 4; until
//! then this module's `pub(crate)` surface is only reachable from its own
//! tests, hence the blanket `dead_code` allow below.
#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use singularmem_core::ItemId;

use crate::error::{Error, Result};

/// Magic bytes identifying a `journal.bin` file.
pub const JOURNAL_MAGIC: &[u8; 4] = b"SMVJ";
/// Current on-disk format version of the journal header.
pub const JOURNAL_VERSION: u16 = 1;
/// Size in bytes of a record's ULID prefix (big-endian, per the `ulid` crate).
const ID_BYTES: usize = 16;
/// Bytes of fixed-size header fields before the variable-length model id:
/// magic (4) + version (2) + dim (4) + model-id length (2).
const HEADER_PREFIX_LEN: usize = 4 + 2 + 4 + 2;

/// Decoded `journal.bin` header: the embedding dimension and model id the
/// journal's records were written under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalHeader {
    pub dim: u32,
    pub model_id: String,
}

impl JournalHeader {
    fn encoded(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_PREFIX_LEN + self.model_id.len());
        out.extend_from_slice(JOURNAL_MAGIC);
        out.extend_from_slice(&JOURNAL_VERSION.to_le_bytes());
        out.extend_from_slice(&self.dim.to_le_bytes());
        let id_len = u16::try_from(self.model_id.len()).expect("model id fits in a u16 length");
        out.extend_from_slice(&id_len.to_le_bytes());
        out.extend_from_slice(self.model_id.as_bytes());
        out
    }

    /// Length in bytes of this header once encoded, i.e. the offset of the
    /// first record in the file.
    fn byte_len(&self) -> u64 {
        u64::try_from(self.encoded().len()).expect("header length fits in a u64")
    }

    fn decode(bytes: &[u8], path: &Path) -> Result<Self> {
        let corrupt = |reason: String| Error::IndexCorrupted {
            path: path.to_path_buf(),
            reason,
        };
        if bytes.len() < HEADER_PREFIX_LEN || &bytes[..4] != JOURNAL_MAGIC {
            return Err(corrupt(
                "journal header magic missing or truncated".to_string(),
            ));
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != JOURNAL_VERSION {
            return Err(corrupt(format!("unsupported journal version {version}")));
        }
        let dim = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
        let id_len = usize::from(u16::from_le_bytes([bytes[10], bytes[11]]));
        let id_bytes = bytes
            .get(HEADER_PREFIX_LEN..HEADER_PREFIX_LEN + id_len)
            .ok_or_else(|| corrupt("journal header model id truncated".to_string()))?;
        let model_id = String::from_utf8(id_bytes.to_vec())
            .map_err(|_| corrupt("journal header model id is not valid UTF-8".to_string()))?;
        Ok(Self { dim, model_id })
    }
}

/// Handle to an open `journal.bin`. Cheap to hold; every operation opens the
/// underlying file fresh so concurrent-safety is left to the caller's lock
/// (see the design doc's "Concurrency" section).
#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    header: JournalHeader,
}

impl Journal {
    /// Open or create `journal.bin` at `path`. Creates the header (and an
    /// otherwise-empty file) when the file is absent.
    ///
    /// # Errors
    /// - `Error::IndexCorrupted` if an existing file's header is unreadable.
    /// - `Error::DimMismatch` / `Error::ModelMismatch` if an existing
    ///   header disagrees with `dim` / `model_id`.
    /// - `Error::Io` on a filesystem failure.
    pub fn open(path: &Path, dim: usize, model_id: &str) -> Result<Self> {
        let dim_u32 = u32::try_from(dim).map_err(|_| Error::DimMismatch {
            expected: dim,
            got: usize::MAX,
        })?;
        let wanted = JournalHeader {
            dim: dim_u32,
            model_id: model_id.to_string(),
        };

        if path.exists() {
            let mut buf = vec![0_u8; HEADER_PREFIX_LEN + 512];
            let mut file = File::open(path).map_err(Error::Io)?;
            let n = file.read(&mut buf).map_err(Error::Io)?;
            let found = JournalHeader::decode(&buf[..n], path)?;
            if found.model_id != wanted.model_id {
                return Err(Error::ModelMismatch {
                    path: path.to_path_buf(),
                    found_model: found.model_id,
                    expected_model: wanted.model_id,
                });
            }
            if found.dim != wanted.dim {
                return Err(Error::DimMismatch {
                    expected: dim,
                    got: usize::try_from(found.dim).unwrap_or(usize::MAX),
                });
            }
            Ok(Self {
                path: path.to_path_buf(),
                header: found,
            })
        } else {
            let mut file = File::create(path).map_err(Error::Io)?;
            file.write_all(&wanted.encoded()).map_err(Error::Io)?;
            file.sync_all().map_err(Error::Io)?;
            Ok(Self {
                path: path.to_path_buf(),
                header: wanted,
            })
        }
    }

    /// Size in bytes of one record: a 16-byte ULID plus `dim` little-endian
    /// `f32`s.
    fn record_len(&self) -> usize {
        let dim = usize::try_from(self.header.dim).expect("dim fits usize; validated at open");
        ID_BYTES + dim * 4
    }

    /// Append `records`; one `write_all` for the whole batch followed by one
    /// `fsync`.
    ///
    /// # Errors
    /// - `Error::DimMismatch` if any vector's length doesn't match the
    ///   journal's dimension. No bytes are written in that case.
    /// - `Error::Io` on a filesystem failure.
    pub fn append(&self, records: &[(ItemId, Vec<f32>)]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let dim = usize::try_from(self.header.dim).expect("dim fits usize; validated at open");
        let mut buf = Vec::with_capacity(records.len() * self.record_len());
        for (id, vector) in records {
            if vector.len() != dim {
                return Err(Error::DimMismatch {
                    expected: dim,
                    got: vector.len(),
                });
            }
            buf.extend_from_slice(&id.to_bytes());
            for x in vector {
                buf.extend_from_slice(&x.to_le_bytes());
            }
        }
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .map_err(Error::Io)?;
        file.write_all(&buf).map_err(Error::Io)?;
        file.sync_all().map_err(Error::Io)
    }

    /// Read every complete record after the header; a trailing partial
    /// record (a crash remnant) is dropped, not an error.
    ///
    /// # Errors
    /// Returns `Error::Io` on a filesystem failure.
    pub fn replay(&self) -> Result<Vec<(ItemId, Vec<f32>)>> {
        let mut file = File::open(&self.path).map_err(Error::Io)?;
        file.seek(SeekFrom::Start(self.header.byte_len()))
            .map_err(Error::Io)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(Error::Io)?;

        let record_len = self.record_len();
        let complete_records = bytes.len() / record_len;
        if bytes.len() % record_len != 0 {
            tracing::debug!(
                path = %self.path.display(),
                "dropping partial trailing journal record",
            );
        }

        let dim = usize::try_from(self.header.dim).expect("dim fits usize; validated at open");
        let mut out = Vec::with_capacity(complete_records);
        for record in bytes[..complete_records * record_len].chunks_exact(record_len) {
            let mut id_bytes = [0_u8; ID_BYTES];
            id_bytes.copy_from_slice(&record[..ID_BYTES]);
            let id = ItemId::from_bytes(id_bytes);
            let vector: Vec<f32> = record[ID_BYTES..]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            debug_assert_eq!(
                vector.len(),
                dim,
                "chunked record must yield exactly `dim` floats"
            );
            out.push((id, vector));
        }
        Ok(out)
    }

    /// Number of complete records currently in the file (excludes any
    /// trailing partial record).
    ///
    /// # Errors
    /// Returns `Error::Io` if the file's metadata cannot be read.
    #[allow(clippy::len_without_is_empty)] // record count, not a collection accessor
    pub fn len(&self) -> Result<usize> {
        let size = std::fs::metadata(&self.path).map_err(Error::Io)?.len();
        let body = size.saturating_sub(self.header.byte_len());
        let record_len = u64::try_from(self.record_len()).expect("record length fits u64");
        Ok(usize::try_from(body / record_len).expect("record count fits usize"))
    }

    /// Truncate the file back to just the header, discarding all records.
    ///
    /// # Errors
    /// Returns `Error::Io` on a filesystem failure.
    pub fn clear(&self) -> Result<()> {
        let file = OpenOptions::new()
            .write(true)
            .open(&self.path)
            .map_err(Error::Io)?;
        file.set_len(self.header.byte_len()).map_err(Error::Io)?;
        file.sync_all().map_err(Error::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> ItemId {
        ulid::Ulid::from(n).to_string().parse().unwrap()
    }

    #[test]
    fn round_trip_records() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(&dir.path().join("journal.bin"), 4, "m@v1").unwrap();
        j.append(&[(id(1), vec![1.0, 2.0, 3.0, 4.0]), (id(2), vec![0.5; 4])])
            .unwrap();
        j.append(&[(id(3), vec![9.0; 4])]).unwrap();
        assert_eq!(j.len().unwrap(), 3);
        let got = j.replay().unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].0, id(1));
        assert_eq!(got[0].1, vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(got[2].1, vec![9.0; 4]);
        j.clear().unwrap();
        assert_eq!(j.len().unwrap(), 0);
        assert!(j.replay().unwrap().is_empty());
    }

    #[test]
    fn truncated_tail_is_dropped_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        let j = Journal::open(&p, 4, "m@v1").unwrap();
        j.append(&[(id(1), vec![1.0; 4]), (id(2), vec![2.0; 4])])
            .unwrap();
        let bytes = std::fs::read(&p).unwrap();
        std::fs::write(&p, &bytes[..bytes.len() - 5]).unwrap(); // cut into the last record
        let j = Journal::open(&p, 4, "m@v1").unwrap();
        let got = j.replay().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, id(1));
    }

    #[test]
    fn header_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        Journal::open(&p, 4, "m@v1").unwrap();
        assert!(matches!(
            Journal::open(&p, 8, "m@v1"),
            Err(Error::DimMismatch { .. })
        ));
        assert!(matches!(
            Journal::open(&p, 4, "other@v1"),
            Err(Error::ModelMismatch { .. })
        ));
    }

    #[test]
    fn bad_magic_is_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        std::fs::write(&p, b"NOPE\0\0").unwrap();
        assert!(matches!(
            Journal::open(&p, 4, "m"),
            Err(Error::IndexCorrupted { .. })
        ));
    }

    #[test]
    fn dim_mismatch_on_append_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(&dir.path().join("journal.bin"), 4, "m").unwrap();
        assert!(matches!(
            j.append(&[(id(1), vec![1.0; 3])]),
            Err(Error::DimMismatch { .. })
        ));
    }
}
