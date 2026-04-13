# Phase 3: Enrollment + CLI

**Goal:** Give users a command-line way to enroll their voice from either a WAV file or live microphone input, and serialize the resulting speaker embedding to a versioned, checksummed `profile.bin` on disk. This phase makes Phase 4's end-to-end verification possible without any GUI.

**Dependencies:** Phase 2 (VAD, embedding extraction, similarity math).
**Complexity:** M

---

## Section 1: Module & File Changes

### 1.1 Files to CREATE

**Rust source:**

```
src/enrollment/mod.rs       # pub mod enroll; pub mod profile;
src/enrollment/enroll.rs    # EnrollmentSession — segments speech, extracts embeddings, averages
src/enrollment/profile.rs   # Profile struct + binary format (magic + version + dim + data + CRC32)
```

**Integration tests:**

```
tests/test_enrollment.rs    # round-trip tests for profile.bin + enrollment from WAV
```

### 1.2 Files to MODIFY

| Path | Change |
|------|--------|
| `src/main.rs` | Add `Enroll` subcommand with `--wav`, `--mic <seconds>`, `--list-passages`, `--output <path>` flags. |
| `src/lib.rs` | Add `pub mod enrollment;` and extend `VoiceGateError` with `Enrollment(String)` + `ProfileFormat(String)` variants. |
| `src/config/settings.rs` | Add `EnrollmentConfig { profile_path: String, min_duration_sec: u32, segment_duration_sec: u32 }` section. |
| `assets/enrollment_passages.txt` | Already exists from Phase 1; this phase's CLI command reads it. |

---

## Section 2: Dependencies & Build Config

**New crate:**

```toml
crc32fast = "1.4"      # CRC32 for profile.bin checksum
```

Rationale: `crc32fast` is tiny (~10 KB), no transitive deps, and is the de-facto Rust CRC32 crate. Adding it via `cargo add crc32fast` per CLAUDE.md rules.

No other new dependencies. `hound` (WAV reading), `dirs` (path resolution), `clap` (CLI flags) were all pinned in Phase 1.

---

## Section 3: Types, Traits & Public API

### 3.1 `src/enrollment/profile.rs`

**Binary format** (exactly per PRD §5.8, extended with dim header per Decision D-003):

```
Offset  Size  Field
------  ----  -----
  0      4    magic bytes "VGPR" (b"VGPR")
  4      4    version u32 little-endian (= 1 in Phase 3; may become 2 in Phase 6 for anti-targets)
  8      4    embedding_dim u32 little-endian (= 192 for ECAPA-TDNN, 256 for WeSpeaker fallback)
 12      4*D  embedding f32 little-endian, D floats where D = embedding_dim
 12+4*D  4    CRC32 of bytes [0..12+4*D] little-endian
```

**Rust API:**

```rust
pub const PROFILE_MAGIC: [u8; 4] = *b"VGPR";
pub const PROFILE_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct Profile {
    pub version: u32,
    pub embedding: Vec<f32>,  // L2-normalized, length = embedding_dim
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
    pub fn new(embedding: Vec<f32>) -> Self;
    pub fn save(&self, path: &Path) -> Result<(), ProfileError>;
    pub fn load(path: &Path) -> Result<Self, ProfileError>;

    /// Resolve the platform-appropriate profile path via `dirs::data_dir()`.
    pub fn default_path() -> anyhow::Result<PathBuf>;
}
```

**Validation rules:**
- `save()` writes all fields in the order above and appends the CRC32 of everything it just wrote.
- `load()` reads magic; rejects on mismatch. Reads version; rejects unknown. Reads dim. Reads `dim * 4` bytes of embedding. Reads CRC32. Computes CRC32 of the first `12 + 4*dim` bytes and compares.
- `load()` does **not** validate that the embedding is L2-normalized — that is the caller's responsibility at write time.
- Any I/O shorter than expected returns `Truncated`.

### 3.2 `src/enrollment/enroll.rs`

```rust
pub const MIN_ENROLL_DURATION_SAMPLES_16K: usize = 16_000 * 20;  // 20 seconds
pub const SEGMENT_SAMPLES_16K: usize = 16_000 * 3;               // 3-second segments
pub const MIN_SEGMENTS: usize = 5;
pub const MAX_SEGMENTS: usize = 12;

pub struct EnrollmentSession {
    vad: SileroVad,
    ecapa: EcapaTdnn,
    /// All 16 kHz audio accumulated so far (speech + silence mixed).
    accumulated_16k: Vec<f32>,
    /// Speech-only segments after VAD gating. Each is SEGMENT_SAMPLES_16K long.
    speech_segments: Vec<Vec<f32>>,
}

impl EnrollmentSession {
    pub fn new(vad: SileroVad, ecapa: EcapaTdnn) -> Self;

    /// Push a chunk of raw 16 kHz audio (no VAD filtering).
    pub fn push_audio(&mut self, audio_16k: &[f32]);

    /// How many seconds of RAW audio have been pushed.
    pub fn duration_seconds(&self) -> f32;

    /// Whether enough data has been gathered to finalize.
    pub fn is_ready(&self) -> bool;

    /// Segment `accumulated_16k` via VAD, extract one embedding per segment,
    /// average them, L2-normalize, and return the centroid.
    pub fn finalize(self) -> anyhow::Result<Vec<f32>>;
}
```

**Algorithm** (`finalize`):

1. Reset VAD state (`self.vad.reset()`).
2. Walk `accumulated_16k` in 512-sample chunks. For each chunk, call `vad.is_speech(chunk)`.
3. Maintain a "current speech run" buffer. On speech frame, append to the run. On silence frame, if the run is >= `SEGMENT_SAMPLES_16K`, emit it as a segment (trim to exactly `SEGMENT_SAMPLES_16K`) and start a new run. Drop short runs.
4. If fewer than `MIN_SEGMENTS` segments produced, return `anyhow::bail!("enrollment produced only N segments, need at least {MIN_SEGMENTS}. Speak for longer or more clearly.")`.
5. Truncate to at most `MAX_SEGMENTS` (keep the first N).
6. For each segment, call `ecapa.extract(segment)` → 192-dim vector.
7. Average all embeddings component-wise.
8. L2-normalize the mean.
9. Return.

### 3.3 `src/main.rs` — `Enroll` subcommand

```rust
#[derive(Subcommand)]
enum Commands {
    Devices,
    Run { /* phase 1 */ },

    /// Enroll a voice. Exactly one of --wav or --mic must be provided,
    /// OR --list-passages to print the enrollment passage and exit.
    Enroll {
        /// Read audio from a WAV file (any channels, any sample rate — auto-converted to 16 kHz mono).
        #[arg(long, conflicts_with_all = ["mic", "list_passages"])]
        wav: Option<PathBuf>,

        /// Record `N` seconds of live audio from the default (or --device) mic.
        #[arg(long, value_name = "SECONDS", conflicts_with_all = ["wav", "list_passages"])]
        mic: Option<u32>,

        /// Print the enrollment passage from assets/enrollment_passages.txt and exit.
        #[arg(long)]
        list_passages: bool,

        /// Override the output profile path. Defaults to platform data dir.
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,

        /// Override the input device (only applies to --mic mode).
        #[arg(long, value_name = "NAME")]
        device: Option<String>,
    },
}
```

### 3.4 Error additions

```rust
// In VoiceGateError:
#[error("enrollment failed: {0}")]
Enrollment(String),

#[error("profile format error: {0}")]
ProfileFormat(#[from] enrollment::profile::ProfileError),
```

---

## Section 4: Runtime Behavior

### 4.1 `voicegate enroll --wav <path>`

1. Print "Reading WAV file: {path}".
2. Open the WAV with `hound::WavReader::open(path)?`.
3. Inspect spec: channels, sample rate, bits per sample.
4. Read samples as `Vec<f32>`:
   - Integer formats → normalize by max value (e.g. `i16` → `/ 32768.0`).
   - `f32` → pass through.
5. Downmix to mono if multi-channel (average across channels).
6. Resample to 16 kHz if needed:
   - If already 16 kHz, pass through.
   - If 48 kHz, use `Resampler48to16` from Phase 2.
   - Other rates → use `rubato::FftFixedIn::<f32>::new(original_rate, 16000, chunk, 2, 1)`.
7. Load models: `SileroVad::load(resolve_model_path("silero_vad.onnx"))?` + `EcapaTdnn::load(resolve_model_path("ecapa_tdnn.onnx"))?`.
8. Create `EnrollmentSession::new(vad, ecapa)`.
9. `session.push_audio(&resampled_16k)`.
10. If `!session.is_ready()` (i.e. duration < 20 s), warn but continue; short clips may still produce usable enrollments if they have enough speech.
11. `let embedding = session.finalize()?;`
12. `let profile = Profile::new(embedding);`
13. Resolve output path: `--output` CLI arg → config `enrollment.profile_path` → `Profile::default_path()` (e.g. `~/.local/share/voicegate/profile.bin` on Linux).
14. Create parent directory if missing.
15. `profile.save(&out_path)?`
16. Print "Profile saved: {path}".
17. Exit 0.

### 4.2 `voicegate enroll --mic <seconds>`

1. Print "Recording for {N} seconds. Read the passage aloud:" followed by the Appendix A passage.
2. Load models (same as 4.1 step 7).
3. Set up a capture stream (reuse `start_capture` from Phase 1) with a local input ring buffer.
4. Create a `Resampler48to16` and a `EnrollmentSession`.
5. Start the capture stream.
6. Worker loop for `N` seconds:
    - Pop 1536 samples from the input ring (spin-wait if empty).
    - Resample 48 → 16 kHz.
    - `session.push_audio(&resampled)`.
    - Print a simple progress indicator every second (e.g. `stderr.write(".")`).
7. Stop the capture stream.
8. `let embedding = session.finalize()?;`
9. Same as 4.1 steps 12–17.

**Note:** Mic enrollment does NOT touch the virtual microphone — it only captures. PipeWire/VB-Cable setup is skipped in enroll mode.

### 4.3 `voicegate enroll --list-passages`

1. Read `assets/enrollment_passages.txt` (locate via `env!("CARGO_MANIFEST_DIR")` in dev, or `$exe_dir/assets/` in packaged builds).
2. Print the contents to stdout.
3. Exit 0.

### 4.4 Profile save atomicity

- Write to `<path>.tmp` first, fsync, then `rename` to `<path>`. This is the standard atomic-replace pattern and prevents corruption from a crash mid-write.
- If `<path>.tmp` already exists (previous crashed run), overwrite it.

### 4.5 Backward compatibility

- `Profile::load` currently accepts only `version == 1`.
- Phase 6 introduces `version == 2` (with appended anti-target count + anti-target embeddings). `load()` at that point accepts both and up-converts version 1 to an in-memory version 2 with zero anti-targets.
- **Phase 3 does NOT need to predict the version-2 byte layout.** That is Phase 6's problem.

---

## Section 5: Cross-Platform & Resource Handling

### 5.1 Profile path resolution

```rust
impl Profile {
    pub fn default_path() -> anyhow::Result<PathBuf> {
        let dir = dirs::data_dir()
            .ok_or_else(|| anyhow::anyhow!("could not resolve data directory"))?;
        Ok(dir.join("voicegate").join("profile.bin"))
    }
}
```

| OS | Resolved to |
|----|-------------|
| Linux | `~/.local/share/voicegate/profile.bin` |
| Windows | `%APPDATA%\voicegate\profile.bin` |
| macOS | `~/Library/Application Support/voicegate/profile.bin` (not supported in v1 but the path is consistent) |

### 5.2 Passage asset resolution

Same pattern as model resolution:
1. `VOICEGATE_ASSETS_DIR` env var.
2. `$exe_dir/assets/enrollment_passages.txt`.
3. `$CARGO_MANIFEST_DIR/assets/enrollment_passages.txt` (dev only).

### 5.3 WAV format compatibility

`hound` handles:
- PCM i8, i16, i24, i32
- IEEE f32
- 1–8 channels
- Any integer sample rate

**Not handled:** 64-bit float WAVs, A-law/μ-law, compressed WAVs. Document that users should convert to 16-bit PCM or 32-bit float first.

### 5.4 Mic enrollment privacy

- Enroll-mic never writes the raw audio to disk — only the final 192-float embedding. The PRD's "Runs locally, no data leaves the machine" promise (§2.2) covers this explicitly.
- Add a comment in `enroll.rs` making this invariant explicit: "DO NOT write `accumulated_16k` to disk at any point."

---

## Section 6: Verification

### Pre-test setup

1. Phase 2 fixture download script has populated `tests/fixtures/speaker_a_enroll.wav`, `speaker_b.wav`.
2. Models are present from Phase 2 (`make models`).

### Automated tests (`cargo test --test test_enrollment`)

3. **`test_profile_roundtrip`** — Create a random 192-float vector, L2-normalize, wrap in `Profile::new`, save to a tempdir path, load back, assert the loaded embedding is bit-identical.
4. **`test_profile_bad_magic`** — Write a file starting with `b"XXXX"`, load. Assert `ProfileError::BadMagic`.
5. **`test_profile_bad_crc`** — Save a valid profile, flip one bit in the middle of the embedding, load. Assert `ProfileError::BadChecksum`.
6. **`test_profile_unsupported_version`** — Write a file with magic=VGPR, version=99. Assert `ProfileError::UnsupportedVersion`.
7. **`test_profile_truncated`** — Write only the first 8 bytes. Assert `ProfileError::Truncated`.
8. **`test_enroll_from_wav`** — Run `EnrollmentSession` on `speaker_a_enroll.wav`. Save. Load. Extract a separate embedding from `speaker_a.wav` (different content, same speaker). Assert `cosine(loaded_centroid, fresh_embedding) > 0.65`.
9. **`test_enroll_discrimination`** — Enroll from `speaker_a_enroll.wav`, then extract from `speaker_b.wav`. Assert `cosine(centroid, speaker_b_embedding) < 0.5`.
10. **`test_enroll_too_short`** — Push only 2 seconds of audio. Assert `finalize()` returns an error mentioning "at least 5 segments".

### CLI smoke tests (manual, documented)

11. `cargo run --release -- enroll --list-passages` prints the PRD Appendix A passage.
12. `cargo run --release -- enroll --wav tests/fixtures/speaker_a_enroll.wav --output /tmp/test_profile.bin` exits 0, creates the file, prints "Profile saved: /tmp/test_profile.bin".
13. `hexdump -C /tmp/test_profile.bin | head -2` shows `56 47 50 52` (magic `VGPR`) at offset 0.
14. `cargo run --release -- enroll --mic 20` records 20 s from the default mic and saves to the default path. Verified on Linux with a real microphone; tester speaks the passage.
15. After step 14, `ls -l ~/.local/share/voicegate/profile.bin` shows a file of exactly `4 + 4 + 4 + 192*4 + 4 = 780` bytes.

### Lint / build

16. `cargo clippy -- -D warnings` clean.
17. `cargo fmt --check` clean.
18. `cargo build --release` succeeds.

### Acceptance thresholds

- `test_enroll_from_wav` cosine > 0.65 between centroid and a fresh same-speaker embedding on different content.
- `test_enroll_discrimination` cosine < 0.5 between centroid and a different speaker.
- Profile file is exactly 780 bytes for `embedding_dim == 192`. (`12 + 192*4 + 4 = 784`. Correction: 12 + 768 + 4 = **784**. Use 784 in test 15.)

---

## Section 6+: PRD Gap Additions

### 6.1 Anti-target forward-compatibility note in Profile (Pass 1, G-008, LOW)

**Gap:** Phase 3's `Profile` struct doesn't hint at the Phase 6 v2 format (which adds anti-target embeddings). An executor reading Phase 3 in isolation might design the `Profile` struct without room for the future extension and get surprised.

**Addition:** Add a doc-comment on the `Profile` struct in `src/enrollment/profile.rs`:

```rust
/// Speaker profile — currently version 1 (self-embedding only).
///
/// Phase 6 adds version 2, which extends this struct with a
/// `Vec<AntiTarget>` of up to MAX_ANTI_TARGETS (8) "not-me" embeddings
/// for margin-based discrimination against similar-sounding speakers.
/// Phase 3 writes only version 1; Phase 6's loader accepts both and
/// up-converts v1 to an in-memory v2 with `anti_targets = vec![]`.
///
/// Do NOT add an `anti_targets` field in Phase 3 — it lands in Phase 6
/// alongside the v2 serializer.
pub struct Profile { ... }
```

This comment is the ONLY Phase-3 concession to Phase 6. The binary format, the `AntiTarget` struct, `update_with_anti_targets`, and the v2 serializer all land in Phase 6.
