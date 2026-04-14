---
paths:
  - "**/*test*"
  - "**/*spec*"
  - "tests/**"
  - ".github/workflows/**"
---

# Testing Rules (Rust Desktop / VoiceGate)

VoiceGate is an audio + ML desktop app. Tests exercise deterministic WAV fixtures and ONNX sessions. There is no database, no HTTP server, and no curl-based testing.

## Test Location

- **Unit tests**: inline in source files using `#[cfg(test)] mod tests { ... }`. Keep them close to the code they test.
- **Integration tests**: `tests/<name>.rs` at the workspace root. Each file is its own crate target. Use this for tests that need multiple modules, fixture WAVs, or a full ORT session.
- **Fixture audio**: `tests/fixtures/*.wav`. Read via `hound`. Do NOT commit large WAVs to git (see `.gitignore`); fetch or generate them in a setup script.

## Unit Test Pattern

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_identical_vectors_is_one() {
        let a = vec![0.5f32, 0.5, 0.5, 0.5];
        let b = a.clone();
        let score = cosine_similarity(&a, &b);
        assert!((score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors_is_zero() {
        let a = vec![1.0f32, 0.0];
        let b = vec![0.0f32, 1.0];
        let score = cosine_similarity(&a, &b);
        assert!(score.abs() < 1e-6);
    }
}
```

No `#[tokio::test]` — VoiceGate is synchronous outside of the eframe event loop. No shared state between tests; each test sets up its own inputs.

## Integration Test Pattern (Fixture WAVs + ONNX)

```rust
// tests/test_ml.rs
use hound::WavReader;
use voicegate::ml::{EcapaTdnn, SileroVad};
use voicegate::ml::similarity::cosine_similarity;

fn read_mono_f32(path: &str) -> Vec<f32> {
    let mut reader = WavReader::open(path).expect("fixture exists");
    reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / i16::MAX as f32)
        .collect()
}

#[test]
fn test_embedding_discrimination() {
    let vad = SileroVad::load("models/silero_vad.onnx").unwrap();
    let ecapa = EcapaTdnn::load("models/ecapa_tdnn.onnx").unwrap();

    let a = read_mono_f32("tests/fixtures/speaker_a.wav");
    let b = read_mono_f32("tests/fixtures/speaker_b.wav");

    let emb_a = ecapa.extract(&a).unwrap();
    let emb_b = ecapa.extract(&b).unwrap();

    let score = cosine_similarity(&emb_a, &emb_b);
    assert!(
        score < 0.5,
        "speakers A and B should be distinguishable (score = {})",
        score
    );
}
```

### Rules for ORT-backed tests

- **Skip gracefully if the ONNX Runtime shared library is missing.** Use `if std::path::Path::new("models/silero_vad.onnx").exists()` to gate model-loading tests. In CI, the models are either vendored or downloaded in a setup step; locally, they are whatever `make models` produced. A missing model should `eprintln!("skipping ...")` and return, not panic.
- **Never share a mutable ORT session between tests running in parallel.** Either create a fresh session per test, or mark them `#[ignore]` and run serially with `cargo test -- --ignored --test-threads=1`.
- **Tolerance, not equality.** Embedding outputs are floating-point; compare with `(a - b).abs() < 1e-4` or similar. Snapshot tests store the vector in a JSON/NPY file and diff element-wise with tolerance.

## What to Test

### Math / pure functions
- `cosine_similarity` with known orthogonal, parallel, and anti-parallel inputs
- L2 normalization preserves direction
- EMA smoothing with known alphas
- Gate state transitions (Open -> Closing -> Closed -> Opening -> Open) with synthetic similarity scores

### Audio math
- Resampler 48k -> 16k output length equals `input_len * 16000 / 48000` within 1 sample
- Ring buffer push/pop preserves sample order with no dropouts when capacity is sufficient
- Mono downmix of stereo input is the arithmetic mean

### ML primitives
- `test_vad_detects_speech` — VAD probability > 0.5 on a speech WAV
- `test_vad_rejects_silence` — VAD probability < 0.5 on a silence WAV
- `test_vad_rejects_noise` — VAD probability < 0.5 on non-speech noise
- `test_embedding_consistency` — same WAV fed twice yields cosine > 0.99
- **`test_embedding_discrimination`** — speaker_a vs speaker_b yields cosine < 0.5 (the single most load-bearing test; Phase 2 verification gate)

### Gate / pipeline
- `test_gate_passthrough_open` — gate in `Open` state produces bit-for-bit identical output
- `test_gate_crossfade_monotone` — crossfade output RMS decreases monotonically during `Closing`, increases during `Opening`
- `test_pipeline_blocks_stranger` — on `mixed_ab.wav` with `speaker_a.bin` profile, output RMS in speaker-B segments is <= 0.1 * input RMS (matches PRD §13.3 >90% silencing)
- `test_pipeline_mixed_ab` — speaker-A segments pass through essentially unchanged; speaker-B segments are heavily attenuated

### Profile round-trip
- `test_profile_save_load` — write `profile.bin`, read it back, cosine(original, loaded) > 0.9999
- `test_profile_bad_magic_rejected` — corrupt the first 4 bytes, `Profile::load` returns an error
- `test_profile_bad_crc_rejected` — flip a bit in the embedding, `Profile::load` returns an error

## Running Tests

```bash
make test                                      # Everything (unit + integration)
cargo test                                     # Same thing directly
cargo test --test test_ml                      # One integration test file
cargo test --test test_ml test_embedding       # All tests with "test_embedding" in the name
cargo test -- --nocapture                      # Show println!/eprintln! output
cargo test -- --test-threads=1                 # Serialize tests that share ORT sessions
cargo test -- --ignored                        # Run #[ignore]d tests (slow / require hardware)
```

## Clippy and Formatting

CI runs `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`. Any warning or formatting diff fails the build. Fix before committing.

Common Clippy fixes:
- Unused variables: prefix with `_` (e.g. `_unused`)
- Redundant clone: remove `.clone()` when the value is used once
- Unnecessary `unwrap()`: use `?` in a `-> Result` function, or `.expect("reason")` when the failure is a bug
- `needless_return`: drop explicit `return` on the last expression
- `unnecessary_cast`: common when converting between integer sizes; often `as usize` on a `u32` in arithmetic contexts where `usize` is already implied

## What NOT to Test

- Do NOT test cpal callbacks directly. Callbacks run on an RT-priority OS thread; unit-testing them is brittle. Test the functions they call.
- Do NOT test `eframe::App::update`. GUI logic is tested by extracting pure logic into `AppController` and testing that.
- Do NOT add network or file-download tests. Fixtures are either committed (small) or produced locally by `make models` / a fixture-generation script.
- Do NOT add sleep-based tests. If you need to wait for a thread, use a condvar or a channel, not `thread::sleep`.
