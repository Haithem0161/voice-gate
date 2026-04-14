---
paths:
  - "src/ml/**"
  - "scripts/download_models.py"
  - "scripts/export_ecapa.py"
  - "tests/test_ml*"
---

# ML Inference Rules (ort + ndarray)

This file is the contract for `src/ml/` — everything that loads, runs, or interprets an ONNX model. Read it before touching `vad.rs`, `embedding.rs`, `similarity.rs`, or the Python model scripts.

## Models In Scope

| Model | File | Size | Purpose | Stateful? |
|-------|------|-----:|---------|----------:|
| Silero VAD | `models/silero_vad.onnx` | ~2 MB | Speech / non-speech classifier | **Yes** (GRU hidden state) |
| ECAPA-TDNN | `models/ecapa_tdnn.onnx` | ~80 MB | 192-dim speaker embedding | No |
| WeSpeaker (fallback) | `models/wespeaker_fallback.onnx` | ~25 MB | 256-dim speaker embedding | No |

See `docs/voicegate/research.md` §§ 2-3 for full background on both models.

## ort Session Creation

- `ort = { version = "2", features = ["load-dynamic"] }` in `Cargo.toml`.
- `load-dynamic` means the ONNX Runtime shared library (`libonnxruntime.so` / `onnxruntime.dll`) is resolved at runtime, NOT at build time. A missing shared library produces a runtime error on first `Session::builder().commit_from_file(...)`, NOT a build failure. This is the correct behavior: Phase 1 compiles without ONNX Runtime installed; Phase 2 onward fails loudly at session creation.
- **Create one session per model, not one per frame.** Session construction is slow (100+ ms) and allocates hundreds of MB. Cache the session inside the struct that owns the model.
- **Sessions are NOT Send+Sync by default** in ort 2. If the session must cross threads, wrap in `Arc<Mutex<Session>>` -- but the preferred pattern is to own the session on the processing worker thread and never hand it out.
- Execution providers: CPU default. CUDA and DirectML are out of scope for v1. A GUI toggle to enable them may come in v2.

```rust
use ort::{Session, SessionBuilder};

pub struct SileroVad {
    session: Session,
    state: Vec<f32>,  // 2 * 1 * 128 = 256 f32 values, persisted across calls
}

impl SileroVad {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, VoiceGateError> {
        let session = Session::builder()
            .map_err(|e| VoiceGateError::Ml(e.to_string()))?
            .commit_from_file(path)
            .map_err(|e| VoiceGateError::Ml(e.to_string()))?;
        Ok(Self {
            session,
            state: vec![0.0; 2 * 1 * 128],
        })
    }
}
```

## Silero VAD: Stateful GRU

**This is the single most important rule in this file.** Silero VAD uses a GRU (gated recurrent unit) as its temporal model. The hidden state MUST persist across calls. Specifically:

- The model has three inputs: `input` (f32, shape `[1, 512]`), `sr` (i64, shape `[1]`, value 16000), `state` (f32, shape `[2, 1, 128]`, initial zeros).
- The model has two outputs: `output` (f32, shape `[1]` — speech probability in [0, 1]) and `stateN` (f32, shape `[2, 1, 128]` — updated hidden state).
- After every call, copy `stateN` into the struct's `state` field and feed it as `state` on the next call.
- **Do NOT recreate the session per frame.** Do NOT re-initialize `state` to zeros per frame. Do NOT reset `state` on voiced frames. The only legitimate reason to reset `state` is after a detected silence gap longer than ~500 ms (5+ consecutive non-speech frames), because the GRU's "memory" of prior speech becomes misleading at that point.
- Failure mode of getting this wrong: VAD output will flicker wildly between 0 and 1 on clean speech.

## Tensor Shapes (the exact shapes you'll type)

### Silero VAD

```
Inputs:
  input: f32[1, 512]   -- 512 raw samples at 16 kHz (= 32 ms frame after 48 -> 16 resample)
  sr:    i64[1]        -- [16000]
  state: f32[2, 1, 128] -- GRU hidden state

Outputs:
  output: f32[1]       -- speech probability
  stateN: f32[2, 1, 128] -- new GRU state
```

### ECAPA-TDNN

```
Input:
  feats: f32[1, T]     -- raw 16 kHz audio, variable length. T should be in [8000, 24000] for sensible embeddings
                          (0.5 s to 1.5 s per research.md §3 "sliding window strategy").

Output:
  embedding: f32[1, 192]  -- speaker embedding, NOT L2-normalized. Caller normalizes.
```

- The input name for ECAPA may vary by export (common names: `feats`, `input`, `audio`). Run `scripts/inspect_onnx.py` (a tiny one-off helper, not committed unless useful) or call `session.inputs()` once at load time to confirm.
- **The caller is responsible for L2 normalization.** The ONNX model does NOT normalize internally. Forgetting this is the #2 pitfall in this module.

### WeSpeaker (fallback)

```
Input:
  feats: f32[1, T]
Output:
  embedding: f32[1, 256]  -- 256-dim, NOT 192
```

Notice the dimension difference. `const EMBEDDING_DIM: usize` in `src/ml/embedding.rs` is the single source of truth. `Profile::save` stores this dimension in the header so a profile from a different model is safely rejected.

## L2 Normalization + Cosine Similarity

```rust
pub fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}
```

- `cosine_similarity` assumes both vectors are pre-normalized (L2). The function name is aspirational — it's actually a dot product with a contract that inputs are unit vectors.
- If you skip L2 normalization, `cosine_similarity` returns the inner product, which is proportional to the true cosine but scaled by the product of the norms. It will still order embeddings correctly, but the threshold (0.70) will be meaningless.
- Never use `f32::acos` to get the angle — angular margin losses in ECAPA-TDNN training mean the embedding space is angular, and the cosine itself is the meaningful quantity.

## EMA Smoothing + Hysteresis

```rust
pub struct SpeakerVerifier {
    centroid: Vec<f32>,       // enrolled profile, L2 normalized, len = EMBEDDING_DIM
    current_score: f32,       // EMA-smoothed similarity
    alpha: f32,               // default 0.3 per research.md §3
    threshold: f32,           // default 0.70 per PRD
}

impl SpeakerVerifier {
    pub fn update(&mut self, live_embedding: &[f32], vad_active: bool) {
        if !vad_active {
            // Do NOT update current_score during silence. Leave it at its last value
            // so hold-time can apply. Updating with a raw 0.0 would cause spurious
            // gate transitions at every silence gap. (Phase 2 gap G-007.)
            return;
        }
        let raw = cosine_similarity(&self.centroid, live_embedding);
        self.current_score = self.alpha * raw + (1.0 - self.alpha) * self.current_score;
    }

    pub fn score(&self) -> f32 { self.current_score }
    pub fn is_verified(&self) -> bool { self.current_score >= self.threshold }
}
```

- **Never update `current_score` during VAD-inactive frames.** This is gap G-007 from Pass 1 of the gap analysis. Silence frames would cause the EMA to drift toward 0 and trigger false "unknown speaker" transitions.
- EMA alpha = 0.3 balances responsiveness against flicker. Lower alpha = more smoothing = slower response; higher alpha = noisier but quicker to react. Do not tune this without first tuning the threshold.

## Context7 is MANDATORY for this module

Before touching `ort`:

1. `resolve-library-id "ort"` — note that `ort` 2.x has a different API from 1.x. Training-data memory of the 1.x `Environment::builder()...` pattern is WRONG.
2. `query-docs` with the specific operation you need: "ort 2 session create from file", "ort 2 run input tensor f32", "ort 2 get output by name", etc.
3. The 2.x `Value::from_array(...)` → `session.run(inputs)` → `outputs["name"].try_extract_tensor::<f32>()` pattern is current as of 2024; verify via Context7 on every new integration.

Before touching `ndarray`:

1. `resolve-library-id "ndarray"`.
2. `query-docs` for tensor construction (`Array2::from_shape_vec`, `ArrayView` borrowing semantics) when you need a specific shape.

## Python Scripts

`scripts/download_models.py` (Phase 2):

- Downloads `silero_vad.onnx` from `https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx` (or the official GitHub release page). Writes to `models/silero_vad.onnx`.
- Uses `requests` or `urllib.request`. Verifies the file size is in the expected range (1 MB - 5 MB) and fails loudly if the download is clearly wrong (e.g. an HTML error page).

`scripts/export_ecapa.py` (Phase 2):

- Loads SpeechBrain's `speechbrain/spkrec-ecapa-voxceleb` via `SpeakerRecognition.from_hparams(...)`.
- Extracts the embedding submodule: `model.mods.embedding_model`.
- Calls `torch.onnx.export(..., opset_version=14, dynamic_axes={"feats": {1: "T"}})`.
- **MANDATORY PyTorch vs ONNX equivalence check**:
  ```python
  dummy = torch.randn(1, 16000)
  pt_out = model.mods.embedding_model(dummy).detach().numpy()
  sess = onnxruntime.InferenceSession("models/ecapa_tdnn.onnx")
  ort_out = sess.run(None, {"feats": dummy.numpy()})[0]
  max_abs_diff = np.max(np.abs(pt_out - ort_out))
  assert max_abs_diff < 1e-4, f"PyTorch vs ONNX divergence: {max_abs_diff}"
  ```
- If this check fails, the wrong submodule was exported and WeSpeaker fallback is engaged (Decision D-002).

## Common Pitfalls

- **Forgetting L2 normalization** — cosine_similarity returns garbage. The enrollment centroid and every live embedding must be L2-normalized before comparison.
- **Recreating the Silero session per frame** — silent VAD flicker, no obvious error.
- **Resetting Silero GRU state on every call** — same symptom as above.
- **Feeding ECAPA a 48 kHz buffer** — embeddings will be garbage. ECAPA expects 16 kHz. Always resample first.
- **Assuming 192-dim** — hard-code nothing. `const EMBEDDING_DIM` in `src/ml/embedding.rs` is the source of truth.
- **Assuming input tensor name is "input"** — varies by model. Check `session.inputs()` once at load time.
- **Using `as f32` to convert i64 tensor values without bounds checking** — not a concern for our models, but noted.
- **Panicking in `SileroVad::update` on an ort error** — the processing worker must keep running even if one frame fails. Return a default probability (e.g. last known) and increment an atomic error counter.
