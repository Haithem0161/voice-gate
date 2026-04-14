//! Phase 3 enrollment integration tests.
//!
//! These tests exercise the full enrollment path end-to-end against the
//! real Silero VAD + WeSpeaker ResNet34 ONNX models and the fixture WAV
//! files populated by `make fixtures`. Tests gracefully skip if the
//! required models or fixtures are not present.
//!
//! Run:
//!   cargo test --test test_enrollment -- --nocapture --test-threads=1
//!
//! The `--test-threads=1` flag is recommended because each test that
//! loads a WeSpeaker session allocates ~100 MB of ONNX runtime state.
//! Parallel execution risks OOM on smaller machines.

use std::path::{Path, PathBuf};

use hound::{SampleFormat, WavReader};

use voicegate::enrollment::enroll::EnrollmentSession;
use voicegate::enrollment::profile::{Profile, ProfileError, PROFILE_MAGIC, PROFILE_VERSION};
use voicegate::ml::embedding::{EcapaTdnn, EMBEDDING_DIM};
use voicegate::ml::similarity::{cosine_similarity, l2_normalize};
use voicegate::ml::vad::SileroVad;

// --- Path helpers ---------------------------------------------------------

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn silero_path() -> PathBuf {
    repo_root().join("models").join("silero_vad.onnx")
}

fn wespeaker_path() -> PathBuf {
    repo_root()
        .join("models")
        .join("wespeaker_resnet34_lm.onnx")
}

fn fixture_path(name: &str) -> PathBuf {
    repo_root().join("tests").join("fixtures").join(name)
}

fn tempdir_for(name: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("voicegate-enroll-{name}-{pid}-{nanos}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn models_available() -> bool {
    if !silero_path().exists() || !wespeaker_path().exists() {
        eprintln!("skipping: models missing, run `make models`");
        return false;
    }
    true
}

fn fixture_available(name: &str) -> bool {
    let p = fixture_path(name);
    if !p.exists() {
        eprintln!(
            "skipping: fixture {} missing, run `make fixtures`",
            p.display()
        );
        return false;
    }
    true
}

fn read_fixture_16k_mono(name: &str) -> Option<Vec<f32>> {
    if !fixture_available(name) {
        return None;
    }
    let path = fixture_path(name);
    let mut reader = WavReader::open(&path).expect("WAV opens");
    let spec = reader.spec();
    assert_eq!(spec.channels, 1, "{name} must be mono");
    assert_eq!(spec.sample_rate, 16_000, "{name} must be 16 kHz");
    assert_eq!(
        spec.sample_format,
        SampleFormat::Int,
        "{name} must be int PCM"
    );
    Some(
        reader
            .samples::<i16>()
            .map(|s| s.expect("valid i16 sample") as f32 / i16::MAX as f32)
            .collect(),
    )
}

// --- Profile format tests (duplicating profile.rs unit tests at the -----
//     integration level per phase-03 section 6) ---------------------------

#[test]
fn test_profile_roundtrip() {
    let dir = tempdir_for("roundtrip");
    let path = dir.join("p.bin");

    // Build a random-ish embedding and L2-normalize it (matching the
    // contract SpeakerVerifier::new expects).
    let mut embedding = vec![0.0f32; EMBEDDING_DIM];
    for (i, v) in embedding.iter_mut().enumerate() {
        *v = ((i * 7 + 3) as f32).sin();
    }
    l2_normalize(&mut embedding);

    let profile = Profile::new(embedding.clone());
    profile.save(&path).expect("save");

    let loaded = Profile::load(&path).expect("load");
    assert_eq!(loaded.version, PROFILE_VERSION);
    assert_eq!(loaded.embedding.len(), EMBEDDING_DIM);
    // Bit-identical: CRC32 + fixed little-endian encoding guarantees this.
    assert_eq!(loaded.embedding, embedding);
}

#[test]
fn test_profile_bad_magic() {
    let dir = tempdir_for("bad_magic");
    let path = dir.join("p.bin");
    let mut body = vec![0u8; 12 + 4 * EMBEDDING_DIM + 4];
    body[0..4].copy_from_slice(b"XXXX");
    body[4..8].copy_from_slice(&PROFILE_VERSION.to_le_bytes());
    body[8..12].copy_from_slice(&(EMBEDDING_DIM as u32).to_le_bytes());
    std::fs::write(&path, &body).unwrap();

    match Profile::load(&path) {
        Err(ProfileError::BadMagic(m)) => assert_eq!(&m, b"XXXX"),
        other => panic!("expected BadMagic, got {other:?}"),
    }
}

#[test]
fn test_profile_bad_crc() {
    let dir = tempdir_for("bad_crc");
    let path = dir.join("p.bin");

    let mut embedding = vec![0.1f32; EMBEDDING_DIM];
    l2_normalize(&mut embedding);
    Profile::new(embedding).save(&path).unwrap();

    // Flip one bit in the middle of the payload.
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
fn test_profile_unsupported_version() {
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
fn test_profile_truncated() {
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

// --- End-to-end enrollment tests -----------------------------------------

fn extract_direct_embedding(model: &mut EcapaTdnn, audio: &[f32]) -> Vec<f32> {
    // Use the same 1.5-second window used by the live pipeline to match
    // how the centroid is computed from 3-second segments. We cap at
    // 24_000 samples (= MAX_WINDOW_SAMPLES_16K) for consistency.
    let take = audio.len().min(24_000);
    let snippet = &audio[..take];
    model.extract(snippet).expect("extract")
}

#[test]
fn test_enroll_from_wav() {
    if !models_available() {
        return;
    }
    let Some(audio_enroll) = read_fixture_16k_mono("speaker_a_enroll.wav") else {
        return;
    };
    let Some(audio_verify) = read_fixture_16k_mono("speaker_a.wav") else {
        return;
    };

    // Run enrollment on the 30 s clip.
    let vad = SileroVad::load(&silero_path()).expect("silero");
    let ecapa = EcapaTdnn::load(&wespeaker_path()).expect("wespeaker");
    let mut session = EnrollmentSession::new(vad, ecapa);
    session.push_audio(&audio_enroll);
    let centroid = session.finalize().expect("finalize");
    assert_eq!(centroid.len(), EMBEDDING_DIM);

    // Save to a temp profile, reload, and compare against a fresh
    // embedding from a DIFFERENT clip of the same speaker.
    let dir = tempdir_for("enroll_wav");
    let path = dir.join("speaker_a.bin");
    Profile::new(centroid).save(&path).expect("save");
    let loaded = Profile::load(&path).expect("load");

    let mut ecapa2 = EcapaTdnn::load(&wespeaker_path()).expect("reload wespeaker");
    let fresh = extract_direct_embedding(&mut ecapa2, &audio_verify);

    let cos = cosine_similarity(&loaded.embedding, &fresh);
    println!("test_enroll_from_wav: cosine(centroid, fresh_A) = {cos:.6}");
    // Phase 2 measured cosine(speaker_a_clip1, speaker_a_enroll_clip) =
    // 0.556 on raw single-window embeddings. The enrolled centroid is
    // the average of 8 different segment embeddings from speaker_a_enroll,
    // which is more stable than a single-window pick -- empirically it
    // lifts the intra-speaker cosine to ~0.6+. 0.5 is a generous floor.
    assert!(
        cos > 0.5,
        "enrolled centroid vs same-speaker fresh embedding cos = {cos}, expected > 0.5"
    );
}

#[test]
fn test_enroll_discrimination() {
    if !models_available() {
        return;
    }
    let Some(audio_enroll) = read_fixture_16k_mono("speaker_a_enroll.wav") else {
        return;
    };
    let Some(audio_b) = read_fixture_16k_mono("speaker_b.wav") else {
        return;
    };

    let vad = SileroVad::load(&silero_path()).expect("silero");
    let ecapa = EcapaTdnn::load(&wespeaker_path()).expect("wespeaker");
    let mut session = EnrollmentSession::new(vad, ecapa);
    session.push_audio(&audio_enroll);
    let centroid = session.finalize().expect("finalize");

    let mut ecapa2 = EcapaTdnn::load(&wespeaker_path()).expect("reload wespeaker");
    let speaker_b_emb = extract_direct_embedding(&mut ecapa2, &audio_b);

    let cos = cosine_similarity(&centroid, &speaker_b_emb);
    println!("test_enroll_discrimination: cosine(centroid_A, speaker_b) = {cos:.6}");
    assert!(
        cos < 0.5,
        "enrolled speaker A centroid vs speaker B embedding cos = {cos}, expected < 0.5"
    );
}

#[test]
fn test_enroll_too_short() {
    if !models_available() {
        return;
    }

    let vad = SileroVad::load(&silero_path()).expect("silero");
    let ecapa = EcapaTdnn::load(&wespeaker_path()).expect("wespeaker");
    let mut session = EnrollmentSession::new(vad, ecapa);

    // Push 2 seconds of zero audio. No speech segments will be produced.
    session.push_audio(&vec![0.0f32; 2 * 16_000]);

    match session.finalize() {
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("need at least"),
                "expected 'need at least N segments' error, got: {msg}"
            );
        }
        Ok(_) => panic!("finalize should fail on 2 s of silence"),
    }
}
