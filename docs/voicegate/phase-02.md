# Phase 2: ML Inference Primitives

**Goal:** Load Silero VAD and ECAPA-TDNN ONNX models, extract speaker embeddings from audio, and prove that two different speakers produce distinguishable embeddings (cosine similarity < 0.5) on real fixture WAVs. This phase contains the single biggest technical risk in the project.

**Dependencies:** Phase 1 (source tree, Cargo.toml, ring buffer, capture stub).
**Complexity:** M

---

## Section 1: Module & File Changes

### 1.1 Files to CREATE

**Rust source:**

```
src/ml/vad.rs               # Silero VAD wrapper with persistent GRU state
src/ml/embedding.rs         # ECAPA-TDNN wrapper, L2 normalization, sliding window buffer
src/ml/similarity.rs        # cosine_similarity + SpeakerVerifier (EMA smoothing)
```

**Python scripts:**

```
scripts/download_models.py  # downloads silero_vad.onnx from the official release
scripts/export_ecapa.py     # exports SpeechBrain ECAPA-TDNN to ONNX with equivalence check
```

**Test fixtures** (binary WAV files, not checked in unless small; see 1.3):

```
tests/fixtures/speaker_a.wav          # ~10 s, speaker A, 16 kHz mono
tests/fixtures/speaker_b.wav          # ~10 s, speaker B, 16 kHz mono
tests/fixtures/speaker_a_enroll.wav   # ~30 s, speaker A reading the enrollment passage
tests/fixtures/silence.wav            # ~5 s of room noise / silence
tests/fixtures/noise.wav              # ~5 s of non-speech noise (fan, keyboard)
```

**Integration test file:**

```
tests/test_ml.rs            # loads fixtures via hound, runs the full suite (see §6)
```

### 1.2 Files to MODIFY

| Path | Change |
|------|--------|
| `src/audio/resampler.rs` | Replace the Phase 1 stub with a real `rubato::FftFixedIn<f32>` wrapper (48 kHz → 16 kHz, 1536-sample input → 512-sample output). |
| `src/ml/mod.rs` | Add `pub mod vad; pub mod embedding; pub mod similarity;` |
| `src/lib.rs` | Re-export `ml::{vad, embedding, similarity}` and extend `VoiceGateError` with `Ml(String)` variant. |
| `Makefile` | Add a real `models:` target that runs both Python scripts. |

### 1.3 Fixture WAV sourcing

Test fixtures are binary blobs and should not bloat the repo. Decision: commit **small** fixtures (<500 KB each) directly; for the 30 s enrollment clip, prefer either:
- A locally-recorded clip committed via git-lfs, OR
- A download script (`scripts/download_fixtures.sh`) that pulls from LibriSpeech dev-clean for reproducibility.

Default to the download-script path so the repo stays lean. `scripts/download_fixtures.sh` is part of this phase's deliverables.

---

## Section 2: Dependencies & Build Config

**No new Rust crates** — `ort`, `ndarray`, `hound`, `rubato` were all pinned in Phase 1's `Cargo.toml`. Phase 2 only imports them.

**Python dependencies (model export environment):**

```
pip install speechbrain==1.0.* torch==2.* onnx==1.16.* onnxruntime==1.17.* numpy
```

Document this in `README.md` under a "Model export" section. The Python environment is **only required for contributors regenerating models**; end users download the pre-exported ONNX files from a release artifact.

**ONNX Runtime shared library** is a runtime prerequisite, not a build-time prerequisite (the `load-dynamic` feature on `ort` defers loading until the first session create). Installation instructions are in Phase 1's README rewrite; Phase 2 only documents them further for clarity.

**`Makefile` targets:**

```makefile
models: models/silero_vad.onnx models/ecapa_tdnn.onnx

models/silero_vad.onnx:
	python scripts/download_models.py

models/ecapa_tdnn.onnx:
	python scripts/export_ecapa.py --output models/ecapa_tdnn.onnx

fixtures:
	bash scripts/download_fixtures.sh
```

---

## Section 3: Types, Traits & Public API

### 3.1 `src/audio/resampler.rs`

```rust
pub struct Resampler48to16 {
    inner: rubato::FftFixedIn<f32>,
    input_scratch: Vec<Vec<f32>>,
    output_scratch: Vec<Vec<f32>>,
}

impl Resampler48to16 {
    pub fn new() -> anyhow::Result<Self>;

    /// Resample exactly 1536 samples @ 48 kHz → exactly 512 samples @ 16 kHz.
    /// The output slice MUST have capacity ≥ 512.
    pub fn process(&mut self, input_48k: &[f32], output_16k: &mut [f32]) -> anyhow::Result<usize>;
}
```

- `rubato::FftFixedIn::<f32>::new(48000, 16000, 1536, 2, 1)` — the `2` is sub-chunks (minimal), `1` is channel count.
- Pre-allocates input/output scratch `Vec<Vec<f32>>` at `new()` time. `process()` copies in, calls `inner.process_into_buffer`, copies out. **No allocations in `process()`.**

### 3.2 `src/ml/vad.rs`

```rust
pub const VAD_CHUNK_SAMPLES: usize = 512;

pub struct SileroVad {
    session: ort::Session,
    state: Vec<f32>,           // shape [2, 1, 128] → 256 floats; persists across calls
    sr_tensor: Vec<i64>,       // [16000], allocated once
    pub threshold: f32,        // default 0.5
}

impl SileroVad {
    pub fn load(model_path: &Path) -> anyhow::Result<Self>;

    /// Run VAD on exactly VAD_CHUNK_SAMPLES (512) samples at 16 kHz.
    /// Returns the raw speech probability.
    pub fn prob(&mut self, audio_16k: &[f32]) -> anyhow::Result<f32>;

    /// Convenience wrapper: `prob(...) > self.threshold`
    pub fn is_speech(&mut self, audio_16k: &[f32]) -> anyhow::Result<bool>;

    pub fn reset(&mut self);  // zero the GRU state (used at enrollment start)
}
```

- Input tensor creation uses `ort::Value::from_array` with an `ndarray::Array2<f32>` of shape `[1, 512]`.
- State tensor is `ndarray::Array3<f32>` of shape `[2, 1, 128]`.
- `sr` tensor is `ndarray::Array1<i64>` of shape `[1]`, value `16000`.
- Output `output` is read as `f32 [1]`; output `stateN` is copied back into `self.state`.

### 3.3 `src/ml/embedding.rs`

```rust
pub const EMBEDDING_DIM: usize = 192;
pub const MIN_WINDOW_SAMPLES_16K: usize = 8_000;   // 0.5 s @ 16 kHz
pub const MAX_WINDOW_SAMPLES_16K: usize = 24_000;  // 1.5 s @ 16 kHz
pub const REEXTRACT_INTERVAL_SAMPLES_16K: usize = 3_200;  // 200 ms @ 16 kHz

pub struct EcapaTdnn {
    session: ort::Session,
}

impl EcapaTdnn {
    pub fn load(model_path: &Path) -> anyhow::Result<Self>;

    /// Extract an L2-normalized EMBEDDING_DIM-float embedding from a variable-length
    /// 16 kHz audio slice. Must be at least MIN_WINDOW_SAMPLES_16K long.
    pub fn extract(&self, audio_16k: &[f32]) -> anyhow::Result<Vec<f32>>;
}

/// Rolling window for real-time embedding extraction. Owns a Vec<f32> of
/// MAX_WINDOW_SAMPLES_16K capacity. Caller pushes new 16 kHz chunks; the window
/// signals when it has enough data and enough time has passed to re-extract.
pub struct EmbeddingWindow {
    buf: Vec<f32>,
    samples_since_last_extract: usize,
}

impl EmbeddingWindow {
    pub fn new() -> Self;
    pub fn push(&mut self, audio_16k: &[f32]);
    pub fn should_extract(&self) -> bool;
    pub fn snapshot(&self) -> &[f32];   // returns current window contents
    pub fn mark_extracted(&mut self);
    pub fn reset(&mut self);
}
```

- `extract` builds an `Array2<f32>` of shape `[1, audio_16k.len()]`, runs the session, extracts output `[1, EMBEDDING_DIM]`, and L2-normalizes.
- `EmbeddingWindow::push` copies bytes; when `buf.len() > MAX_WINDOW_SAMPLES_16K`, it discards the oldest samples (ring-buffer semantics but using a `Vec` + tail-shift for simplicity; not hot path).
- `should_extract()` returns `true` iff `buf.len() >= MIN_WINDOW_SAMPLES_16K` AND `samples_since_last_extract >= REEXTRACT_INTERVAL_SAMPLES_16K`.

### 3.4 `src/ml/similarity.rs`

```rust
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

pub fn l2_normalize(v: &mut [f32]) {
    let norm_sq: f32 = v.iter().map(|x| x * x).sum();
    let norm = norm_sq.sqrt().max(1e-12);
    for x in v.iter_mut() { *x /= norm; }
}

#[derive(Debug, Clone, Copy)]
pub enum VerifyResult {
    Match(f32),
    NoMatch(f32),
}

pub struct SpeakerVerifier {
    pub enrolled: Vec<f32>,  // EMBEDDING_DIM floats, L2-normalized
    pub threshold: f32,      // default 0.70
    pub ema_alpha: f32,      // default 0.3
    current_score: f32,      // EMA-smoothed running similarity
}

impl SpeakerVerifier {
    pub fn new(enrolled: Vec<f32>, threshold: f32, ema_alpha: f32) -> Self;
    pub fn update(&mut self, live: &[f32]) -> VerifyResult;
    pub fn current_score(&self) -> f32;
    pub fn reset(&mut self);  // zero the EMA state
}
```

### 3.5 Error additions

```rust
// In VoiceGateError:
#[error("ML model error: {0}")]
Ml(String),

#[error("ML model file not found: {0}")]
ModelNotFound(String),

#[error("ONNX Runtime not available. Install libonnxruntime.so (Linux) or onnxruntime.dll (Windows).")]
OrtUnavailable,
```

---

## Section 4: Runtime Behavior

### 4.1 Silero VAD call sequence (per 32 ms frame)

1. Caller provides exactly 512 f32 samples at 16 kHz (the resampler's output for one 1536-sample 48 kHz frame).
2. Build input tensor from `&self.state` clone + `&audio_16k` + `&self.sr_tensor`.
3. `session.run(inputs)` → outputs `{output, stateN}`.
4. Read `output[0]` as the speech probability.
5. Copy `stateN` back into `self.state` (reuses the existing `Vec<f32>` — no realloc).
6. Return the probability.

**Allocations per call:** one temporary input tensor per session run. ort lets us avoid reallocation by holding pre-built `Value` handles, but the first implementation allocates per call for simplicity. If profiling (Phase 4 verification) shows this as a hot spot, we optimize then.

### 4.2 ECAPA-TDNN call sequence (per 200 ms extraction trigger)

1. Worker thread checks `embedding_window.should_extract()`.
2. If true, call `ecapa.extract(embedding_window.snapshot())` on the current window contents.
3. `extract()` builds a `[1, N]` f32 tensor, runs the session, reads `[1, 192]` output, L2-normalizes, returns.
4. Caller passes the result to `SpeakerVerifier::update()`.
5. `embedding_window.mark_extracted()` resets the interval counter.

**Latency:** 5–15 ms for 0.5–1.5 s windows. Runs on the worker thread, so input/output callbacks are unaffected. Fires ~5 times/second when VAD is active.

### 4.3 Sliding window lifecycle

```
frame N:   vad=false → skip embedding path entirely (window is left alone)
frame N+1: vad=true  → push 512 samples into window; buf.len()=512, not enough to extract
frame N+2: vad=true  → push 512; buf.len()=1024
...
frame N+16: vad=true → push 512; buf.len()=8192 (≥MIN), samples_since=8192
             → should_extract() = true → extract → mark_extracted()
frame N+17..22: vad=true → push 512 × 6 = 3072 samples
frame N+23: vad=true → push 512; samples_since=3584 (≥REEXTRACT_INTERVAL=3200)
             → should_extract() = true → extract
```

### 4.4 EMA smoothing math

```
current = ema_alpha * raw + (1 - ema_alpha) * current
```

With `ema_alpha = 0.3`, a single outlier moves `current` by 30% of the delta. Five consecutive extractions at a given value converge within 2% of that value. This gives ~1 s to settle after a speaker change at 200 ms update rate.

**Reset conditions:**
- Re-enrollment: reset `current_score` to 0 (or the enrolled centroid's self-similarity, which is 1.0).
- Silence period (VAD off) longer than 500 ms: leave `current_score` unchanged; next speech reuses it (avoids flicker on pauses).

### 4.5 Model session lifetime

- `SileroVad::load` and `EcapaTdnn::load` are called **once** at app startup by the worker thread.
- Sessions are owned by the worker thread and never shared across threads (simpler than `Arc` and we don't need parallel inference).
- Model paths resolved via: CLI override → config → executable-relative `models/{silero_vad,ecapa_tdnn}.onnx`.

---

## Section 5: Cross-Platform & Resource Handling

### 5.1 ONNX Runtime shared library

- **Linux:** system-wide `/usr/local/lib/libonnxruntime.so` + `ldconfig`. `ort` with `load-dynamic` searches `LD_LIBRARY_PATH`, then `/usr/lib`, then `/usr/local/lib`.
- **Windows:** `onnxruntime.dll` next to `voicegate.exe` is the standard Windows DLL search order and is the approach Phase 6 packaging uses.
- **Missing library:** `ort::Session::builder()?.commit_from_file(...)` returns `Error::LoadLibrary`. Wrap with `VoiceGateError::OrtUnavailable` + install instructions.

### 5.2 Model file lookup

```rust
fn resolve_model_path(name: &str) -> anyhow::Result<PathBuf> {
    // 1. env var override (VOICEGATE_MODELS_DIR)
    // 2. executable-relative: $exe_dir/models/{name}
    // 3. cwd-relative: ./models/{name}
    // 4. error with clear instruction to run `make models`
}
```

### 5.3 Fixture path resolution in tests

Integration tests in `tests/test_ml.rs` use `env!("CARGO_MANIFEST_DIR")` to resolve fixture paths. This works regardless of where `cargo test` is invoked from.

### 5.4 Python export script portability

- `scripts/export_ecapa.py` uses `speechbrain.inference.speaker.EncoderClassifier.from_hparams(source="speechbrain/spkrec-ecapa-voxceleb")`, which downloads to a local cache.
- Works on Linux, Windows, macOS — pure Python.
- **The script runs on the contributor's machine, not in CI**. Phase 2's Makefile target assumes a local Python env. Phase 6 publishes the exported ONNX as a release artifact so end users never run the export themselves.

### 5.5 Fixture licensing

LibriSpeech is CC BY 4.0; re-distribution is allowed with attribution. `scripts/download_fixtures.sh` downloads from the official OpenSLR mirror at runtime, so the repo itself never ships the audio data.

---

## Section 6: Verification

**This is the CRITICAL RISK GATE for the entire project.** If the discrimination test fails, we do not proceed to Phase 3; we execute Decision D-002 (WeSpeaker fallback) inside this phase.

### Pre-test setup

1. `make models` completes successfully, producing `models/silero_vad.onnx` (~2 MB) and `models/ecapa_tdnn.onnx` (~80 MB).
2. `scripts/export_ecapa.py` runs the PyTorch ↔ ONNX equivalence check and asserts `max(|pytorch_out - onnx_out|) < 1e-4` on a fixed seed dummy input. If this fails, the script exits non-zero and must be debugged before the Rust side is touched.
3. `make fixtures` populates `tests/fixtures/` with all 5 WAV files.

### Automated tests (`cargo test --test test_ml`)

4. **`test_cosine_similarity_math`** — Hand-verified vectors:
   - `cosine_similarity([1,0,0], [1,0,0]) == 1.0`
   - `cosine_similarity([1,0,0], [0,1,0]) == 0.0`
   - `cosine_similarity([1,0,0], [-1,0,0]) == -1.0`
5. **`test_l2_normalize`** — `l2_normalize([3,4,0])` produces `[0.6, 0.8, 0.0]`.
6. **`test_resampler_quality`** — Resample a 440 Hz sine at 48 kHz → 16 kHz → back via a reference rubato resampler. Assert RMS error < 0.02.
7. **`test_vad_detects_speech`** — Load `speaker_a.wav`, run VAD on the middle 512 samples. Assert `prob > 0.5`.
8. **`test_vad_rejects_silence`** — Load `silence.wav`, run VAD. Assert `prob < 0.3`.
9. **`test_vad_rejects_noise`** — Load `noise.wav`, run VAD. Assert `prob < 0.5`. (Noise can trigger VAD occasionally — tolerance is looser.)
10. **`test_embedding_consistency`** — Extract embedding from `speaker_a.wav` twice. Assert cosine > 0.99.
11. **`test_embedding_discrimination`** ⚠ **LOAD-BEARING TEST** — Extract embedding from `speaker_a.wav` and `speaker_b.wav`. Assert cosine < 0.5.
12. **`test_embedding_self_similarity`** — Extract from `speaker_a.wav` and `speaker_a_enroll.wav` (same speaker, different content). Assert cosine > 0.65. This is looser than `consistency` because the content differs.
13. **`test_embedding_window_lifecycle`** — Push chunks into an `EmbeddingWindow`, verify `should_extract()` transitions, `snapshot` returns the expected slice, `mark_extracted` resets the counter.

### Fallback decision gate

14. If **any** of tests 10/11/12 fails despite the Python equivalence check passing:
    1. Document the failure in this file's §6+ with the exact cosine values observed.
    2. Switch `scripts/download_models.py` to download `wespeaker_resnet34.onnx` from https://github.com/wenet-e2e/wespeaker releases.
    3. Update `EMBEDDING_DIM` to 256 in `src/ml/embedding.rs`.
    4. Re-run tests 10/11/12.
    5. If WeSpeaker **also** fails, the project is blocked on a model-quality issue, not an engineering issue. Stop Phase 2 and escalate.

### Lint / build

15. `cargo clippy -- -D warnings` clean.
16. `cargo fmt --check` clean.
17. `cargo build --release` succeeds.

### Acceptance thresholds

- `test_embedding_discrimination` cosine < 0.5 — **non-negotiable**.
- `test_embedding_consistency` cosine > 0.99 — verifies determinism.
- `test_embedding_self_similarity` cosine > 0.65 — verifies same-speaker generalization across content.
- All other tests pass as written.

---

## Section 6+: PRD Gap Additions

### 6.1 EMA smoothing — silence behavior clarification (Pass 1, G-007, LOW)

**Gap:** Section 4.4 documents the EMA formula but doesn't explicitly state what happens during VAD-inactive frames. The PRD is silent on this and research.md §3 only covers it in passing.

**Addition (clarification to §4.4):**

During VAD-inactive frames (VAD probability below threshold), the pipeline **does not** call `SpeakerVerifier::update`. The `current_score` field is **left unchanged**. This matters because:

- Short natural speech pauses (<500 ms) don't reset the similarity meter to zero, which would cause visible flicker in the GUI and spurious gate closes.
- When speech resumes within the gate's hold window, the EMA picks up from its last value, giving instant continuity.
- Explicit `reset()` is only called at enrollment start and on bypass-mode changes — never from the pipeline's frame loop.

This rule is a runtime behavior, not a new type. `SpeakerVerifier::update` remains the only mutation path; the pipeline just avoids calling it when VAD is silent.
