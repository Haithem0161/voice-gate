//! Speaker profile binary format with v1/v2 support.
//!
//! v1 layout (Phase 3):
//!   magic(4) + version(4) + dim(4) + embedding(4*D) + crc32(4)
//!
//! v2 layout (Phase 6, adds anti-targets):
//!   magic(4) + version(4) + dim(4) + embedding(4*D) +
//!   anti_target_count(1) + [name_len(1) + name(N) + embedding(4*D)]* +
//!   crc32(4)
//!
//! load() accepts both v1 and v2. save() always writes v2.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::enrollment::anti_target::AntiTarget;
use crate::ml::embedding::EMBEDDING_DIM;

pub const PROFILE_MAGIC: [u8; 4] = *b"VGPR";
pub const PROFILE_VERSION: u32 = 2;

#[derive(Debug, Clone)]
pub struct Profile {
    pub version: u32,
    pub embedding: Vec<f32>,
    pub anti_targets: Vec<AntiTarget>,
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
    pub fn new(embedding: Vec<f32>) -> Self {
        debug_assert_eq!(embedding.len(), EMBEDDING_DIM);
        Self {
            version: PROFILE_VERSION,
            embedding,
            anti_targets: Vec::new(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), ProfileError> {
        if self.embedding.len() != EMBEDDING_DIM {
            return Err(ProfileError::DimMismatch {
                expected: EMBEDDING_DIM,
                found: self.embedding.len(),
            });
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut body: Vec<u8> = Vec::with_capacity(2048);
        body.extend_from_slice(&PROFILE_MAGIC);
        body.extend_from_slice(&PROFILE_VERSION.to_le_bytes());
        body.extend_from_slice(&(EMBEDDING_DIM as u32).to_le_bytes());
        for &f in &self.embedding {
            body.extend_from_slice(&f.to_le_bytes());
        }

        // Anti-targets (v2)
        let count = self.anti_targets.len() as u8;
        body.push(count);
        for at in &self.anti_targets {
            let name_bytes = at.name.as_bytes();
            let name_len = name_bytes.len().min(255) as u8;
            body.push(name_len);
            body.extend_from_slice(&name_bytes[..name_len as usize]);
            for &f in &at.embedding {
                body.extend_from_slice(&f.to_le_bytes());
            }
        }

        let checksum = crc32fast::hash(&body);
        body.extend_from_slice(&checksum.to_le_bytes());

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

    pub fn load(path: &Path) -> Result<Self, ProfileError> {
        let mut f = fs::File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;

        let min_size = 4 + 4 + 4 + 4 + 4;
        if buf.len() < min_size {
            return Err(ProfileError::Truncated);
        }

        let mut magic = [0u8; 4];
        magic.copy_from_slice(&buf[0..4]);
        if magic != PROFILE_MAGIC {
            return Err(ProfileError::BadMagic(magic));
        }

        let version = u32::from_le_bytes(buf[4..8].try_into().expect("4 bytes"));
        if version != 1 && version != 2 {
            return Err(ProfileError::UnsupportedVersion(version));
        }

        let embedding_dim = u32::from_le_bytes(buf[8..12].try_into().expect("4 bytes")) as usize;
        if embedding_dim != EMBEDDING_DIM {
            return Err(ProfileError::DimMismatch {
                expected: EMBEDDING_DIM,
                found: embedding_dim,
            });
        }

        let emb_end = 12 + 4 * embedding_dim;
        if buf.len() < emb_end + 4 {
            return Err(ProfileError::Truncated);
        }

        let mut embedding = Vec::with_capacity(embedding_dim);
        for i in 0..embedding_dim {
            let start = 12 + 4 * i;
            embedding.push(f32::from_le_bytes(
                buf[start..start + 4].try_into().expect("4 bytes"),
            ));
        }

        let mut anti_targets = Vec::new();

        if version == 1 {
            // v1: body ends at emb_end, crc follows
            let body_len = emb_end;
            if buf.len() < body_len + 4 {
                return Err(ProfileError::Truncated);
            }
            let stored_crc =
                u32::from_le_bytes(buf[body_len..body_len + 4].try_into().expect("4 bytes"));
            let computed_crc = crc32fast::hash(&buf[..body_len]);
            if stored_crc != computed_crc {
                return Err(ProfileError::BadChecksum);
            }
        } else {
            // v2: anti-targets follow the embedding
            let mut pos = emb_end;
            if pos >= buf.len() - 4 {
                return Err(ProfileError::Truncated);
            }
            let at_count = buf[pos] as usize;
            pos += 1;

            for _ in 0..at_count {
                if pos >= buf.len() - 4 {
                    return Err(ProfileError::Truncated);
                }
                let name_len = buf[pos] as usize;
                pos += 1;
                if pos + name_len > buf.len() - 4 {
                    return Err(ProfileError::Truncated);
                }
                let name = String::from_utf8_lossy(&buf[pos..pos + name_len]).to_string();
                pos += name_len;

                if pos + 4 * embedding_dim > buf.len() - 4 {
                    return Err(ProfileError::Truncated);
                }
                let mut at_emb = Vec::with_capacity(embedding_dim);
                for i in 0..embedding_dim {
                    let start = pos + 4 * i;
                    at_emb.push(f32::from_le_bytes(
                        buf[start..start + 4].try_into().expect("4 bytes"),
                    ));
                }
                pos += 4 * embedding_dim;
                anti_targets.push(AntiTarget::new(name, at_emb));
            }

            // CRC covers everything up to the last 4 bytes
            let body_len = buf.len() - 4;
            let stored_crc =
                u32::from_le_bytes(buf[body_len..body_len + 4].try_into().expect("4 bytes"));
            let computed_crc = crc32fast::hash(&buf[..body_len]);
            if stored_crc != computed_crc {
                return Err(ProfileError::BadChecksum);
            }
        }

        Ok(Self {
            version: PROFILE_VERSION, // always upgrade in-memory to v2
            embedding,
            anti_targets,
        })
    }

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

    #[test]
    fn format_constants_match_spec() {
        assert_eq!(PROFILE_MAGIC, *b"VGPR");
        assert_eq!(PROFILE_VERSION, 2);
    }

    #[test]
    fn roundtrip_writes_and_reads_identically() {
        let dir = tempdir_for("roundtrip");
        let path = dir.join("p.bin");
        let original = Profile::new(unit_vec());
        original.save(&path).expect("save");
        let loaded = Profile::load(&path).expect("load");
        assert_eq!(loaded.version, PROFILE_VERSION);
        assert_eq!(loaded.embedding, original.embedding);
        assert!(loaded.anti_targets.is_empty());
    }

    #[test]
    fn v2_roundtrip_with_anti_targets() {
        let dir = tempdir_for("v2_rt");
        let path = dir.join("p.bin");
        let mut profile = Profile::new(unit_vec());
        let mut at_emb = vec![0.0f32; EMBEDDING_DIM];
        at_emb[1] = 1.0;
        profile
            .anti_targets
            .push(AntiTarget::new("brother".into(), at_emb.clone()));
        profile
            .anti_targets
            .push(AntiTarget::new("sister".into(), at_emb));
        profile.save(&path).expect("save");
        let loaded = Profile::load(&path).expect("load");
        assert_eq!(loaded.anti_targets.len(), 2);
        assert_eq!(loaded.anti_targets[0].name, "brother");
        assert_eq!(loaded.anti_targets[1].name, "sister");
        assert_eq!(loaded.anti_targets[0].embedding.len(), EMBEDDING_DIM);
    }

    #[test]
    fn v1_profile_loads_as_v2() {
        // Manually write a v1 profile
        let dir = tempdir_for("v1_compat");
        let path = dir.join("p.bin");
        let emb = unit_vec();
        let mut body = Vec::new();
        body.extend_from_slice(&PROFILE_MAGIC);
        body.extend_from_slice(&1u32.to_le_bytes()); // version 1
        body.extend_from_slice(&(EMBEDDING_DIM as u32).to_le_bytes());
        for &f in &emb {
            body.extend_from_slice(&f.to_le_bytes());
        }
        let crc = crc32fast::hash(&body);
        body.extend_from_slice(&crc.to_le_bytes());
        std::fs::write(&path, &body).unwrap();

        let loaded = Profile::load(&path).expect("load v1");
        assert_eq!(loaded.version, PROFILE_VERSION); // upgraded to v2
        assert_eq!(loaded.embedding, emb);
        assert!(loaded.anti_targets.is_empty());
    }

    #[test]
    fn bad_magic_rejected() {
        let dir = tempdir_for("bad_magic");
        let path = dir.join("p.bin");
        let mut body = vec![0u8; 12 + 4 * EMBEDDING_DIM + 4];
        body[0..4].copy_from_slice(b"XXXX");
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
    fn bad_checksum_rejected() {
        let dir = tempdir_for("bad_crc");
        let path = dir.join("p.bin");
        Profile::new(unit_vec()).save(&path).unwrap();
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
        let mut body = Vec::new();
        body.extend_from_slice(&PROFILE_MAGIC);
        body.extend_from_slice(&1u32.to_le_bytes());
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
}
