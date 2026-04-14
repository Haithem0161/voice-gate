//! Speaker profile binary format: `VGPR` magic + version + dim + embedding + CRC32.
//!
//! The on-disk layout is fixed and versioned. Phase 3 writes version 1 only.
//! Phase 6 will introduce version 2 with appended anti-target embeddings; see
//! the doc comment on [`Profile`] below for the forward-compatibility note.
//!
//! Layout (all multi-byte integers little-endian):
//!
//! ```text
//! Offset  Size     Field
//! ------  -------  -----
//!   0       4      magic "VGPR" (b"VGPR")
//!   4       4      version u32
//!   8       4      embedding_dim u32
//!  12       4 * D  embedding f32[D]
//! 12 + 4D   4      crc32 of bytes [0 .. 12 + 4*D]
//! ```
//!
//! For Phase 2's WeSpeaker ResNet34-LM model (per D-002R), `D = 256` and
//! the total file size is `12 + 1024 + 4 = 1040` bytes.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::ml::embedding::EMBEDDING_DIM;

pub const PROFILE_MAGIC: [u8; 4] = *b"VGPR";
pub const PROFILE_VERSION: u32 = 1;

/// Speaker profile -- currently version 1 (self-embedding only).
///
/// Phase 6 adds version 2, which extends this struct with a
/// `Vec<AntiTarget>` of up to `MAX_ANTI_TARGETS` (8) "not-me" embeddings
/// for margin-based discrimination against similar-sounding speakers.
/// Phase 3 writes only version 1; Phase 6's loader accepts both and
/// up-converts v1 to an in-memory v2 with `anti_targets = vec![]`.
///
/// Do NOT add an `anti_targets` field in Phase 3 -- it lands in Phase 6
/// alongside the v2 serializer. See phase-03.md section 6.1 (G-008) and
/// the decisions log in research.md for the full forward-compat story.
#[derive(Debug, Clone)]
pub struct Profile {
    pub version: u32,
    /// L2-normalized speaker embedding. Length must equal [`EMBEDDING_DIM`].
    pub embedding: Vec<f32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("invalid magic bytes (expected VGPR, found {0:?})")]
    BadMagic([u8; 4]),
    #[error("unsupported profile version {0}")]
    UnsupportedVersion(u32),
    #[error("embedding dimension mismatch (expected {expected}, found {found})")]
    DimMismatch { expected: usize, found: usize },
    #[error("checksum mismatch")]
    BadChecksum,
    #[error("unexpected end of file")]
    Truncated,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl Profile {
    /// Construct a new profile from an L2-normalized embedding. The caller
    /// is responsible for normalization; `save()` does not verify it.
    pub fn new(embedding: Vec<f32>) -> Self {
        debug_assert_eq!(
            embedding.len(),
            EMBEDDING_DIM,
            "Profile::new: embedding must be exactly {EMBEDDING_DIM} floats"
        );
        Self {
            version: PROFILE_VERSION,
            embedding,
        }
    }

    /// Serialize this profile to `path`. Uses an atomic tmp+rename pattern
    /// so that a crash mid-write cannot leave a truncated profile in place.
    pub fn save(&self, path: &Path) -> Result<(), ProfileError> {
        // Sanity: the format math assumes exactly EMBEDDING_DIM floats.
        if self.embedding.len() != EMBEDDING_DIM {
            return Err(ProfileError::DimMismatch {
                expected: EMBEDDING_DIM,
                found: self.embedding.len(),
            });
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Build the body in memory so we can compute its CRC before hitting
        // the filesystem. Body size is small (1040 bytes for D=256).
        let body_len = 4 + 4 + 4 + 4 * EMBEDDING_DIM;
        let mut body: Vec<u8> = Vec::with_capacity(body_len + 4);
        body.extend_from_slice(&PROFILE_MAGIC);
        body.extend_from_slice(&self.version.to_le_bytes());
        body.extend_from_slice(&(EMBEDDING_DIM as u32).to_le_bytes());
        for &f in &self.embedding {
            body.extend_from_slice(&f.to_le_bytes());
        }
        debug_assert_eq!(body.len(), body_len);

        let checksum = crc32fast::hash(&body);
        body.extend_from_slice(&checksum.to_le_bytes());

        // Atomic write: write to <path>.tmp, fsync, rename over <path>.
        let mut tmp = path.to_path_buf().into_os_string();
        tmp.push(".tmp");
        let tmp_path = PathBuf::from(tmp);

        {
            let mut f = fs::File::create(&tmp_path)?;
            f.write_all(&body)?;
            f.sync_all()?;
        }
        fs::rename(&tmp_path, path)?;

        Ok(())
    }

    /// Load and validate a profile from `path`. Rejects wrong magic, wrong
    /// version, wrong embedding dimension, bad checksum, and truncated files.
    /// Does NOT validate that the embedding is L2-normalized -- that was
    /// the writer's responsibility.
    pub fn load(path: &Path) -> Result<Self, ProfileError> {
        let mut f = fs::File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;

        // Minimum size: magic(4) + version(4) + dim(4) + at least 1 float + crc(4)
        let min_size = 4 + 4 + 4 + 4 + 4;
        if buf.len() < min_size {
            return Err(ProfileError::Truncated);
        }

        // Magic
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&buf[0..4]);
        if magic != PROFILE_MAGIC {
            return Err(ProfileError::BadMagic(magic));
        }

        // Version
        let version = u32::from_le_bytes(buf[4..8].try_into().expect("4 bytes"));
        if version != PROFILE_VERSION {
            return Err(ProfileError::UnsupportedVersion(version));
        }

        // Embedding dim
        let embedding_dim = u32::from_le_bytes(buf[8..12].try_into().expect("4 bytes")) as usize;
        if embedding_dim != EMBEDDING_DIM {
            return Err(ProfileError::DimMismatch {
                expected: EMBEDDING_DIM,
                found: embedding_dim,
            });
        }

        // Full expected file length
        let body_len = 12 + 4 * embedding_dim;
        let expected_len = body_len + 4;
        if buf.len() < expected_len {
            return Err(ProfileError::Truncated);
        }

        // CRC: compute over the body bytes [0..body_len], compare to the
        // trailing 4 bytes.
        let stored_crc =
            u32::from_le_bytes(buf[body_len..body_len + 4].try_into().expect("4 bytes"));
        let computed_crc = crc32fast::hash(&buf[..body_len]);
        if stored_crc != computed_crc {
            return Err(ProfileError::BadChecksum);
        }

        // Decode the embedding floats.
        let mut embedding = Vec::with_capacity(embedding_dim);
        for i in 0..embedding_dim {
            let start = 12 + 4 * i;
            let end = start + 4;
            let f = f32::from_le_bytes(buf[start..end].try_into().expect("4 bytes"));
            embedding.push(f);
        }

        Ok(Self { version, embedding })
    }

    /// Resolve the platform-appropriate profile path via `dirs::data_dir()`.
    ///
    /// - Linux: `~/.local/share/voicegate/profile.bin`
    /// - Windows: `%APPDATA%\voicegate\profile.bin`
    /// - macOS: `~/Library/Application Support/voicegate/profile.bin`
    pub fn default_path() -> anyhow::Result<PathBuf> {
        let dir =
            dirs::data_dir().ok_or_else(|| anyhow::anyhow!("could not resolve data directory"))?;
        Ok(dir.join("voicegate").join("profile.bin"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_vec() -> Vec<f32> {
        let mut v = vec![0.0f32; EMBEDDING_DIM];
        v[0] = 1.0;
        v
    }

    #[test]
    fn format_constants_match_spec() {
        assert_eq!(PROFILE_MAGIC, *b"VGPR");
        assert_eq!(PROFILE_VERSION, 1);
    }

    #[test]
    fn roundtrip_writes_and_reads_identically() {
        let dir = tempdir_for("roundtrip");
        let path = dir.join("p.bin");
        let original = Profile::new(unit_vec());
        original.save(&path).expect("save");
        let loaded = Profile::load(&path).expect("load");
        assert_eq!(loaded.version, original.version);
        assert_eq!(loaded.embedding.len(), EMBEDDING_DIM);
        assert_eq!(loaded.embedding, original.embedding);
    }

    #[test]
    fn file_size_matches_formula() {
        let dir = tempdir_for("size");
        let path = dir.join("p.bin");
        Profile::new(unit_vec()).save(&path).unwrap();
        let expected = 12 + 4 * EMBEDDING_DIM + 4;
        let actual = std::fs::metadata(&path).unwrap().len() as usize;
        assert_eq!(actual, expected);
    }

    #[test]
    fn bad_magic_rejected() {
        let dir = tempdir_for("bad_magic");
        let path = dir.join("p.bin");
        let mut body = vec![0u8; 12 + 4 * EMBEDDING_DIM + 4];
        body[0..4].copy_from_slice(b"XXXX");
        // Fill version + dim with valid values so the bad-magic branch is
        // what we trip, not the truncation branch.
        body[4..8].copy_from_slice(&1u32.to_le_bytes());
        body[8..12].copy_from_slice(&(EMBEDDING_DIM as u32).to_le_bytes());
        std::fs::write(&path, &body).unwrap();

        match Profile::load(&path) {
            Err(ProfileError::BadMagic(m)) => assert_eq!(&m, b"XXXX"),
            other => panic!("expected BadMagic, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_version_rejected() {
        let dir = tempdir_for("bad_version");
        let path = dir.join("p.bin");
        let mut body = vec![0u8; 12 + 4 * EMBEDDING_DIM + 4];
        body[0..4].copy_from_slice(&PROFILE_MAGIC);
        body[4..8].copy_from_slice(&99u32.to_le_bytes());
        body[8..12].copy_from_slice(&(EMBEDDING_DIM as u32).to_le_bytes());
        std::fs::write(&path, &body).unwrap();

        match Profile::load(&path) {
            Err(ProfileError::UnsupportedVersion(v)) => assert_eq!(v, 99),
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }

    #[test]
    fn dim_mismatch_rejected() {
        let dir = tempdir_for("bad_dim");
        let path = dir.join("p.bin");
        let bogus_dim = 999u32;
        let mut body = vec![0u8; 12 + 4 * bogus_dim as usize + 4];
        body[0..4].copy_from_slice(&PROFILE_MAGIC);
        body[4..8].copy_from_slice(&PROFILE_VERSION.to_le_bytes());
        body[8..12].copy_from_slice(&bogus_dim.to_le_bytes());
        std::fs::write(&path, &body).unwrap();

        match Profile::load(&path) {
            Err(ProfileError::DimMismatch { expected, found }) => {
                assert_eq!(expected, EMBEDDING_DIM);
                assert_eq!(found, bogus_dim as usize);
            }
            other => panic!("expected DimMismatch, got {other:?}"),
        }
    }

    #[test]
    fn bad_checksum_rejected() {
        let dir = tempdir_for("bad_crc");
        let path = dir.join("p.bin");
        Profile::new(unit_vec()).save(&path).unwrap();

        // Flip one bit in the middle of the embedding.
        let mut data = std::fs::read(&path).unwrap();
        let mid = 12 + 4 * (EMBEDDING_DIM / 2);
        data[mid] ^= 0x01;
        std::fs::write(&path, &data).unwrap();

        match Profile::load(&path) {
            Err(ProfileError::BadChecksum) => {}
            other => panic!("expected BadChecksum, got {other:?}"),
        }
    }

    #[test]
    fn truncated_file_rejected() {
        let dir = tempdir_for("truncated");
        let path = dir.join("p.bin");
        // Only 8 bytes: magic + version, no dim/embedding/crc.
        let mut body = Vec::new();
        body.extend_from_slice(&PROFILE_MAGIC);
        body.extend_from_slice(&PROFILE_VERSION.to_le_bytes());
        std::fs::write(&path, &body).unwrap();

        match Profile::load(&path) {
            Err(ProfileError::Truncated) => {}
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    #[test]
    fn default_path_contains_voicegate_and_profile() {
        let p = Profile::default_path().expect("resolve");
        let s = p.to_string_lossy();
        assert!(s.contains("voicegate"));
        assert!(s.ends_with("profile.bin"));
    }

    /// Create a unique per-test directory under the target/ tree. We don't
    /// use tempfile because VoiceGate deliberately minimizes dev dependencies
    /// and the directory is tiny + cleaned up on cargo clean.
    fn tempdir_for(name: &str) -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("voicegate-profile-{name}-{pid}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
