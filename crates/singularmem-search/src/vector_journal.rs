//! Append-only journal of vectors added since the last compaction of the
//! `USearch` index. Format documented in `docs/formats/vectors-v2.md`.
//!
//! Driven by `VectorIndex::commit`/`compact`/`open`, which own the advisory
//! file lock that serialises concurrent writers; this module does no locking
//! of its own.
//!
//! The on-disk model id is capped at 512 bytes (see [`Journal::open`]); the
//! header is read with two bounded `read_exact` calls rather than one
//! best-effort `read`, so a header can never be misparsed from a short read.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use singularmem_core::ItemId;

use crate::error::{Error, Result};
use crate::fsync::sync_dir;

/// Magic bytes identifying a `journal.bin` file.
pub const JOURNAL_MAGIC: &[u8; 4] = b"SMVJ";
/// Current on-disk format version of the journal header.
pub const JOURNAL_VERSION: u16 = 1;
/// Size in bytes of a record's ULID prefix (big-endian, per the `ulid` crate).
const ID_BYTES: usize = 16;
/// Bytes of fixed-size header fields before the variable-length model id:
/// magic (4) + version (2) + dim (4) + model-id length (2).
const HEADER_PREFIX_LEN: usize = 4 + 2 + 4 + 2;
/// Maximum length in bytes of an on-disk model id. `encoded()`'s length
/// field can technically hold up to `u16::MAX`, but `open` bounds the id it
/// will create or accept to this cap so the header can be read with a
/// fixed-size buffer.
const MAX_MODEL_ID_LEN: usize = 512;

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
    /// Byte length of `header` once encoded, i.e. the offset of the first
    /// record in the file. Computed once at open/create time rather than
    /// re-encoding the header on every call that needs it.
    header_len: u64,
}

impl Journal {
    /// Open `journal.bin` at `path`, validating an existing header against
    /// `dim`/`model_id`.
    ///
    /// **Nothing is written when the file is absent**: the header is derived
    /// from `dim`/`model_id` and the file is created by the first
    /// [`append`](Journal::append). Opening a fully compacted vector
    /// directory — the common case for a search-only process, and the only
    /// case that can work on a read-only filesystem — therefore touches no
    /// file at all. [`replay`](Journal::replay), [`len`](Journal::len) and
    /// [`clear`](Journal::clear) treat an absent file as an empty journal.
    ///
    /// `model_id` is capped at 512 bytes so the header can always be read
    /// with a fixed-size buffer (see the module doc).
    ///
    /// # Errors
    /// - `Error::IndexCorrupted` if an existing file's header is unreadable.
    /// - `Error::DimMismatch` / `Error::ModelMismatch` if an existing
    ///   header disagrees with `dim` / `model_id`.
    /// - `Error::Embedding` if `model_id` is longer than 512 bytes.
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
            let mut file = File::open(path).map_err(Error::Io)?;
            let mut prefix = [0_u8; HEADER_PREFIX_LEN];
            if let Err(e) = file.read_exact(&mut prefix) {
                return Err(if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    Error::IndexCorrupted {
                        path: path.to_path_buf(),
                        reason: "journal header truncated".to_string(),
                    }
                } else {
                    Error::Io(e)
                });
            }
            // Only the id length (bytes 10..12) is needed before we know how
            // much more to read; full validation happens in `decode` below.
            let id_len = usize::from(u16::from_le_bytes([prefix[10], prefix[11]]));
            let mut id_bytes = vec![0_u8; id_len];
            if let Err(e) = file.read_exact(&mut id_bytes) {
                return Err(if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    Error::IndexCorrupted {
                        path: path.to_path_buf(),
                        reason: "journal header model id truncated".to_string(),
                    }
                } else {
                    Error::Io(e)
                });
            }
            let mut buf = Vec::with_capacity(HEADER_PREFIX_LEN + id_len);
            buf.extend_from_slice(&prefix);
            buf.extend_from_slice(&id_bytes);
            let found = JournalHeader::decode(&buf, path)?;
            if found.model_id != wanted.model_id {
                return Err(Error::ModelMismatch {
                    path: path.to_path_buf(),
                    found_model: found.model_id,
                    expected_model: wanted.model_id,
                });
            }
            if found.dim != wanted.dim {
                return Err(Error::DimMismatch {
                    expected: usize::try_from(found.dim).unwrap_or(usize::MAX),
                    got: dim,
                });
            }
            let header_len = HEADER_PREFIX_LEN + id_len;
            Ok(Self {
                path: path.to_path_buf(),
                header: found,
                header_len: u64::try_from(header_len).expect("header length fits in a u64"),
            })
        } else {
            if wanted.model_id.len() > MAX_MODEL_ID_LEN {
                return Err(Error::Embedding {
                    context: "journal model id",
                    reason: "model id longer than 512 bytes".to_string(),
                });
            }
            let header_len = wanted.encoded().len();
            Ok(Self {
                path: path.to_path_buf(),
                header: wanted,
                header_len: u64::try_from(header_len).expect("header length fits in a u64"),
            })
        }
    }

    /// Write the header out, creating `journal.bin`. Called by the first
    /// [`append`](Journal::append).
    ///
    /// # Durability guarantee
    ///
    /// On return, both the header bytes **and the directory entry naming the
    /// file** are durable: the file is `fsync`ed and then its parent
    /// directory is too. Without the second `fsync`, the first
    /// `commit(false)` in a directory's life could return `Ok` and still
    /// lose the whole journal to a power cut — the bytes would be on the
    /// platter but the name pointing at them would not, so the recovered
    /// directory would have no journal to replay and the caller would have
    /// been told its vectors were durable.
    ///
    /// The parent-directory `fsync` is the same [`sync_dir`] compaction uses
    /// for its renames; on Windows, which has no directory handle, it is a
    /// no-op.
    fn create_file(&self) -> Result<()> {
        let mut file = File::create(&self.path).map_err(Error::Io)?;
        file.write_all(&self.header.encoded()).map_err(Error::Io)?;
        file.sync_all().map_err(Error::Io)?;
        match self.path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => sync_dir(dir),
            // A bare relative filename has no directory to sync; the
            // process's cwd is not ours to open.
            _ => Ok(()),
        }
    }

    /// Size in bytes of one record: a 16-byte ULID plus `dim` little-endian
    /// `f32`s.
    fn record_len(&self) -> usize {
        let dim = usize::try_from(self.header.dim).expect("dim fits usize; validated at open");
        ID_BYTES + dim * 4
    }

    /// Append `records`; one `write_all` for the whole batch followed by one
    /// `fsync`. Buffers the whole batch in memory before writing, so peak
    /// memory use is proportional to the batch size; concurrent callers must
    /// serialize their own `append`/compaction sequence (Task 4's commit
    /// lock — this module does no locking of its own).
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
        if !self.path.exists() {
            // Lazily created (see `open`); the caller holds the commit lock,
            // so no other writer can be racing this create.
            self.create_file()?;
        }
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .map_err(Error::Io)?;
        file.write_all(&buf).map_err(Error::Io)?;
        file.sync_all().map_err(Error::Io)
    }

    /// Read every complete record after the header; a trailing partial
    /// record (a crash remnant) is dropped, not an error. An absent file is
    /// an empty journal. Reads the whole journal into memory at once, so peak
    /// memory use is proportional to the journal's size.
    ///
    /// # Errors
    /// Returns `Error::Io` on a filesystem failure.
    pub fn replay(&self) -> Result<Vec<(ItemId, Vec<f32>)>> {
        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::Io(e)),
        };
        file.seek(SeekFrom::Start(self.header_len))
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
    /// trailing partial record). An absent file is an empty journal.
    ///
    /// # Errors
    /// Returns `Error::Io` if the file's metadata cannot be read.
    #[allow(clippy::len_without_is_empty)] // record count, not a collection accessor
    pub fn len(&self) -> Result<usize> {
        let size = match std::fs::metadata(&self.path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(Error::Io(e)),
        };
        let body = size.saturating_sub(self.header_len);
        let record_len = u64::try_from(self.record_len()).expect("record length fits u64");
        Ok(usize::try_from(body / record_len).expect("record count fits usize"))
    }

    /// Truncate the file back to just the header, discarding all records. An
    /// absent file is already empty and is left absent — clearing must never
    /// be the thing that creates the journal.
    ///
    /// # Errors
    /// Returns `Error::Io` on a filesystem failure.
    pub fn clear(&self) -> Result<()> {
        let file = match OpenOptions::new().write(true).open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(Error::Io(e)),
        };
        file.set_len(self.header_len).map_err(Error::Io)?;
        file.sync_all().map_err(Error::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> ItemId {
        ulid::Ulid::from(n).to_string().parse().unwrap()
    }

    /// Hand-assembled `journal.bin` header bytes, independent of
    /// `JournalHeader::encoded`, for exercising `decode`'s corruption
    /// branches directly.
    fn raw_header(magic: &[u8], version: u16, dim: u32, model_id: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(magic);
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&dim.to_le_bytes());
        let id_len = u16::try_from(model_id.len()).unwrap();
        out.extend_from_slice(&id_len.to_le_bytes());
        out.extend_from_slice(model_id);
        out
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
        // Append so the header actually reaches disk: `open` is lazy.
        Journal::open(&p, 4, "m@v1")
            .unwrap()
            .append(&[(id(1), vec![1.0; 4])])
            .unwrap();
        match Journal::open(&p, 8, "m@v1") {
            Err(Error::DimMismatch { expected, got }) => {
                // `expected` is the on-disk dimension, `got` is the caller's.
                assert_eq!(expected, 4);
                assert_eq!(got, 8);
            }
            other => panic!("expected DimMismatch, got {other:?}"),
        }
        assert!(matches!(
            Journal::open(&p, 4, "other@v1"),
            Err(Error::ModelMismatch { .. })
        ));
    }

    #[test]
    fn bad_magic_is_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        std::fs::write(&p, raw_header(b"NOPE", JOURNAL_VERSION, 4, b"m@v1")).unwrap();
        assert!(matches!(
            Journal::open(&p, 4, "m@v1"),
            Err(Error::IndexCorrupted { .. })
        ));
    }

    #[test]
    fn too_short_header_is_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        // Only 6 bytes total, short of the 12-byte fixed prefix.
        std::fs::write(&p, b"SMVJ\x01\x00").unwrap();
        assert!(matches!(
            Journal::open(&p, 4, "m@v1"),
            Err(Error::IndexCorrupted { .. })
        ));
    }

    #[test]
    fn unsupported_version_is_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        std::fs::write(&p, raw_header(JOURNAL_MAGIC, 99, 4, b"m@v1")).unwrap();
        assert!(matches!(
            Journal::open(&p, 4, "m@v1"),
            Err(Error::IndexCorrupted { .. })
        ));
    }

    #[test]
    fn truncated_model_id_is_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        // Header claims a 4-byte model id but only 1 byte follows the prefix.
        let mut bytes = raw_header(JOURNAL_MAGIC, JOURNAL_VERSION, 4, b"m@v1");
        bytes.truncate(HEADER_PREFIX_LEN + 1);
        std::fs::write(&p, bytes).unwrap();
        assert!(matches!(
            Journal::open(&p, 4, "m@v1"),
            Err(Error::IndexCorrupted { .. })
        ));
    }

    #[test]
    fn non_utf8_model_id_is_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        std::fs::write(
            &p,
            raw_header(JOURNAL_MAGIC, JOURNAL_VERSION, 4, &[0xFF, 0xFE, 0x00, 0x00]),
        )
        .unwrap();
        assert!(matches!(
            Journal::open(&p, 4, "m@v1"),
            Err(Error::IndexCorrupted { .. })
        ));
    }

    #[test]
    fn golden_bytes_header_and_record() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        let j = Journal::open(&p, 2, "m@v1").unwrap();
        j.append(&[(id(1), vec![1.0, 2.0])]).unwrap();

        // Pinned byte-for-byte per the worked example in
        // `.superpowers/sdd/task-3-report.md`: this test fails if field
        // order or endianness ever silently changes.
        #[rustfmt::skip]
        let expected: Vec<u8> = vec![
            // header (16 bytes)
            0x53, 0x4D, 0x56, 0x4A, // magic "SMVJ"
            0x01, 0x00,             // version = 1 (LE u16)
            0x02, 0x00, 0x00, 0x00, // dim = 2 (LE u32)
            0x04, 0x00,             // model_id_len = 4 (LE u16)
            b'm', b'@', b'v', b'1', // model_id = "m@v1"
            // record (24 bytes)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // ULID big-endian value 1
            0x00, 0x00, 0x80, 0x3F, // 1.0f32 (LE)
            0x00, 0x00, 0x00, 0x40, // 2.0f32 (LE)
        ];
        assert_eq!(std::fs::read(&p).unwrap(), expected);
    }

    #[test]
    fn open_creates_nothing_until_the_first_append() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        let j = Journal::open(&p, 4, "m@v1").unwrap();
        assert!(!p.exists(), "open must not create the file");
        assert_eq!(j.len().unwrap(), 0);
        assert!(j.replay().unwrap().is_empty());
        j.clear().unwrap();
        assert!(!p.exists(), "clear must not create the file either");

        j.append(&[(id(1), vec![1.0; 4])]).unwrap();
        assert!(p.exists(), "the first append creates the header");
        assert_eq!(j.len().unwrap(), 1);
        assert_eq!(j.replay().unwrap()[0].0, id(1));
    }

    /// `create_file` fsyncs the parent directory as well as the file, so the
    /// directory entry naming a brand-new `journal.bin` survives a power cut.
    /// The fsync itself is not observable from a test; what is observable is
    /// that adding it did not break creation — including in a directory
    /// reached by a relative path, where `Path::parent` is the interesting
    /// edge case.
    #[test]
    fn create_file_writes_the_header_and_survives_a_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("journal.bin");
        let j = Journal::open(&p, 4, "m@v1").unwrap();
        j.create_file().expect("creating the journal must succeed");
        assert!(p.exists(), "create_file must create the file");
        assert_eq!(
            std::fs::read(&p).unwrap(),
            j.header.encoded(),
            "the file must hold exactly the header"
        );

        // A bare filename has no parent directory to fsync; creation must
        // still succeed rather than trying to open "".
        let bare = Journal::open(Path::new("journal-bare-name.bin"), 4, "m@v1").unwrap();
        assert!(bare.path.parent().unwrap().as_os_str().is_empty());
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
