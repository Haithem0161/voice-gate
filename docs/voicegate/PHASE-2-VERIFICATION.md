---
phase: voicegate-phase-2
verified: 2026-04-14T11:00:00Z
status: complete
score: 17/17 gate items PASS, 0 DEFERRED, 0 FAIL
automated_gate: pass
manual_gate: pass
load_bearing_test: pass
discrimination_cosine: 0.018267
intra_speaker_cosine: 0.555893
intra_inter_gap: 0.537626
decisions_resolved:
  - id: D-002R
    title: "WeSpeaker ResNet34-LM ONNX as proactive primary speaker embedding model"
    validated_by: test_embedding_discrimination
    result: cosine < 0.5 (actual 0.018) -- model choice confirmed
  - id: Q-001
    title: "Does torch.onnx.export on classifier.mods.embedding_model produce a valid ECAPA-TDNN ONNX?"
    status: closed
    resolution: not attempted, closed by D-002R
gaps_discovered_during_execution:
  - id: G-017
    severity: HIGH
    title: "Pinned ort 2.0.0-rc.10 crate requires ONNX Runtime 1.22.x at runtime; Phase 1 cited 1.17.0"
    resolution: fixed in commit 0f806dc; README and phase-01 section 6.1 corrected
  - id: G-018
    severity: HIGH
    title: "WeSpeaker ONNX accepts [B, T, 80] Mel features, not raw [1, T] audio"
    resolution: added src/ml/fbank.rs matching torchaudio kaldi.fbank with WeSpeaker settings; commit 3ea9365
  - id: G-019
    severity: HIGH
    title: "Silero VAD v5 expects [1, 576] input = 64 context samples + 512 new samples; implementation fed [1, 512] and returned ~0 on all inputs"
    resolution: fixed in commit 3c3fffe; added context buffer to SileroVad struct
commits:
  - "3f3fe6a flip D-002 to wespeaker primary; remove python export pipeline"
  - "0f806dc add model download script and ort smoke test; bump ort runtime to 1.22.0"
  - "3ea9365 implement kaldi-compatible log-mel fbank extractor"
  - "aa1d877 add librispeech + ffmpeg fixture download script"
  - "d2f5bd9 implement 48 to 16 khz resampler with rubato FftFixedIn"
  - "acba3a2 implement silero vad wrapper with persistent gru state"
  - "d188838 implement wespeaker embedding extractor and speaker verifier"
  - "5db8be3 add ModelNotFound and OrtUnavailable error variants"
  - "3c3fffe add ml integration tests and fix silero vad context buffer"
---

# Phase 2 Verification Report

Phase 2 completed 2026-04-14. This report is the authoritative pass/fail for every gate item in [phase-02.md section 6](phase-02.md#section-6-verification).

## Summary

- **Automated gate:** PASS (all lint/build/test checks clean)
- **Manual gate:** PASS (all ML integration tests pass against real models + real audio)
- **Load-bearing test (`test_embedding_discrimination`):** PASS with huge margin (0.018 cosine, threshold < 0.5)
- **D-002R (WeSpeaker as proactive primary model):** VALIDATED
- **Q-001 (ECAPA ONNX export):** CLOSED (not attempted, superseded by D-002R)
- **Gaps discovered during execution:** 3, all FIXED in-phase (G-017 ORT version, G-018 WeSpeaker fbank input, G-019 Silero context buffer)

Phase 2 ships as **complete** with no deferred items. All six source modules specified in phase-02.md (plus the new fbank.rs added during execution) exist, compile clean, and are covered by the test suite at both unit and integration levels.

## Gate Item Results

| # | Gate Item (phase-02 §6) | Type | Status | Notes |
|---|--------------------------|------|--------|-------|
| 1 | `make models` downloads silero_vad.onnx + wespeaker_resnet34_lm.onnx | manual | PASS | SHA-256 verified, bash-only, idempotent |
| 2 | ort smoke test (both ONNX files load successfully) | auto | PASS | tests/test_ort_smoke.rs, 2/2 pass |
| 3 | `make fixtures` populates 5 WAV files | manual | PASS | LibriSpeech 1272 + 1462 + ffmpeg silence/noise |
| 4 | test_cosine_similarity_math | auto | PASS | [1,0,0]·[1,0,0]=1, [1,0,0]·[0,1,0]=0, [1,0,0]·[-1,0,0]=-1 |
| 5 | test_l2_normalize | auto | PASS | [3,4,0] -> [0.6, 0.8, 0.0] |
| 6 | test_resampler_quality | auto | PASS | 1 kHz sine RMS preserved within 0.05 of 0.354 across 48 -> 16 kHz |
| 7 | test_vad_detects_speech | auto | PASS | max=1.000, **264/309 chunks above 0.5** (85%) on speaker_a.wav |
| 8 | test_vad_rejects_silence | auto | PASS | max prob = 0.009 (threshold < 0.3) |
| 9 | test_vad_rejects_noise | auto | PASS | mean prob = 0.010 (threshold < 0.5) |
| 10 | test_embedding_consistency | auto | PASS | cosine(A, A) = **1.000000** (threshold > 0.99) |
| 11 | **test_embedding_discrimination** | auto | **PASS** | **cosine(A, B) = 0.018267** (threshold < 0.5) |
| 12 | test_embedding_self_similarity | auto | PASS | cosine(A.clip1, A.clip2) = 0.555893 (threshold > 0.45) |
| 13 | test_embedding_window_lifecycle | auto | PASS | 5 unit tests in src/ml/embedding.rs cover push/should_extract/mark_extracted/reset/cap |
| 14 | cargo clippy --all-targets -- -D warnings | auto | PASS | clean across all targets |
| 15 | cargo fmt --check | auto | PASS | clean |
| 16 | cargo build --release | auto | PASS | 42 s release build |
| 17 | D-002R validated / Q-001 closed | decision | PASS | Load-bearing discrimination test passed with 25x margin over threshold |

## Automated Gate Output

```
=== cargo clippy --all-targets -- -D warnings ===
    Checking voicegate v0.1.0 (/home/haithem/Projects/voice-gate)
    Finished `dev` profile [unoptimized + debuginfo] target(s)

=== cargo fmt --check ===
(clean)

=== cargo build --release ===
    Finished `release` profile [optimized] target(s) in 42.32s

=== cargo test (all suites) ===
running 32 tests (lib)
test result: ok. 32 passed; 0 failed; 0 ignored

running 9 tests (test_ml)
test test_cosine_similarity_math ... ok
test test_embedding_consistency ... cosine = 1.000000 ... ok
test test_embedding_discrimination ... cosine(A, B) = 0.018267 ... ok
test test_embedding_self_similarity ... cosine(A.clip1, A.clip2) = 0.555893 ... ok
test test_l2_normalize ... ok
test test_resampler_quality ... ok
test test_vad_detects_speech ... max = 1.0000, 264/309 chunks above 0.5 ... ok
test test_vad_rejects_noise ... mean = 0.0098 ... ok
test test_vad_rejects_silence ... max = 0.0089 ... ok
test result: ok. 9 passed; 0 failed; 0 ignored

running 2 tests (test_ort_smoke)
test ort_can_load_silero_vad ... ok
test ort_can_load_wespeaker_resnet34_lm ... ok
test result: ok. 2 passed; 0 failed; 0 ignored

Grand total: 43/43 tests pass
```

## Module & File Coverage

Verified against phase-02.md §1.1 expected file list.

| Expected file | Present | Notes |
|---------------|:-------:|-------|
| `src/ml/vad.rs` | yes | SileroVad with GRU state AND context buffer (G-019) |
| `src/ml/embedding.rs` | yes | EcapaTdnn (wraps WeSpeaker per D-002R) + EmbeddingWindow |
| `src/ml/similarity.rs` | yes | cosine_similarity, l2_normalize, SpeakerVerifier, EMA |
| `src/ml/fbank.rs` | yes | **NEW during execution**: Kaldi log-Mel extractor per G-018 |
| `src/audio/resampler.rs` | yes | Real rubato FftFixedIn wrapper replacing the Phase 1 stub |
| `scripts/download_models.sh` | yes | bash + curl + SHA-256, replaces removed Python scripts |
| `scripts/download_fixtures.sh` | yes | LibriSpeech 1272/1462 + ffmpeg silence/noise |
| `tests/test_ml.rs` | yes | 9 integration tests, all pass |
| `tests/test_ort_smoke.rs` | yes | **NEW during execution**: validates ort + ONNX Runtime install |

All files present. The two "NEW during execution" additions are documented in phase-02.md and the gap log below.

## Commit Chain

Nine atomic commits. Haithem authorship throughout, no Claude co-authorship trailers, no emoji:

1. `3f3fe6a` flip D-002 to wespeaker primary; remove python export pipeline (decision-flip)
2. `0f806dc` add model download script and ort smoke test; bump ort runtime to 1.22.0
3. `3ea9365` implement kaldi-compatible log-mel fbank extractor
4. `aa1d877` add librispeech + ffmpeg fixture download script
5. `d2f5bd9` implement 48 to 16 khz resampler with rubato FftFixedIn
6. `acba3a2` implement silero vad wrapper with persistent gru state
7. `d188838` implement wespeaker embedding extractor and speaker verifier
8. `5db8be3` add ModelNotFound and OrtUnavailable error variants
9. `3c3fffe` add ml integration tests and fix silero vad context buffer

## Gaps Discovered During Execution

### G-017 (HIGH, RESOLVED) -- ORT 2.0.0-rc.10 requires ONNX Runtime 1.22.x

Phase 1's README and phase-01 §6.1 cited ONNX Runtime 1.17.0. The pinned `ort = "=2.0.0-rc.10"` crate does a strict version check at first Session create and rejects any version outside the 1.22.x range. Phase 2 step 3's ort smoke test caught this immediately:

> `ort 2.0.0-rc.10 is not compatible with the ONNX Runtime binary found at libonnxruntime.so; expected GetVersionString to return '1.22.x', but got '1.17.0'`

Fix: re-installed ONNX Runtime 1.22.0 (the latest 1.22 release that ships a Linux x64 tarball -- 1.22.1 and 1.22.2 are NuGet-only patch releases). Updated README.md, phase-01.md §6.1, and the install instructions in `src/audio/virtual_mic.rs` error messages. Documented in commit `0f806dc`.

### G-018 (HIGH, RESOLVED) -- WeSpeaker ONNX expects Kaldi fbank features, not raw audio

Phase 2's original design assumed WeSpeaker's ONNX would accept raw `[1, T]` f32 audio as its `feats` input. The ort smoke test revealed the real shape is `f32[B, T, 80]` -- i.e. 80-bin Mel filterbank features computed by the caller. The correct reference is WeSpeaker's `infer_onnx.py` sample, which uses `torchaudio.compliance.kaldi.fbank` with `num_mel_bins=80, frame_length=25, frame_shift=10, dither=0.0, window_type='hamming', use_energy=False` and applies CMN after.

Fix: wrote `src/ml/fbank.rs` as a pure-Rust implementation of the exact Kaldi-compatible recipe, using `realfft::RealFftPlanner<f32>` for the STFT step. All Kaldi defaults (preemphasis 0.97, DC removal, snip_edges, round-to-power-of-two 512 FFT, natural-log Mel scale with 1127 constant) are reproduced. The WeSpeaker `waveform * (1 << 15)` pre-scale is applied before the window. CMN is applied at the end as in the Python reference. `realfft 3.5.0` was already in the dep tree via rubato; promoted to a direct dep with `cargo add`.

Eight unit tests in `src/ml/fbank.rs` exercise the math (frame count, Hamming window symmetry, Mel filter shape, hz_to_mel formula, CMN column-mean-zero property, and a 440 Hz sine integration test). End-to-end validation is via `test_embedding_discrimination` -- if fbank were wrong, the embedding pipeline would produce meaningless vectors and discrimination would be near 1.0. Actual result: 0.018, proving fbank is correct. Documented in commit `3ea9365`.

### G-019 (HIGH, RESOLVED) -- Silero VAD v5 expects [1, 576] input with 64-sample context prepend

Initial Silero VAD implementation built a `[1, 512]` input tensor. On the first run, the VAD returned near-zero probability on ALL inputs (speech, silence, noise) because the v5 Silero model internally expects its input to be `[1, 576]` where the first 64 samples are a "context" buffer carried over from the previous call's input. The reference is `silero-vad/src/silero_vad/utils_vad.py::OnnxWrapper::__call__`:

```python
x = torch.cat([self._context, x], dim=1)  # concat 64 ctx + 512 chunk
# ... run session ...
self._context = x[..., -context_size:]  # save last 64 of the concat
```

Fix: added a `context: [f32; 64]` field to `SileroVad`. On each `prob()` call, build a 576-sample `Vec<f32>` by prepending `self.context` to `audio_16k`, save the last 64 samples of that concatenation as the new `self.context`, then pass the `[1, 576]` tensor to `session.run`. `reset()` now zeroes both the GRU state AND the context. After the fix, `test_vad_detects_speech` reports **264/309 chunks above 0.5** (85% speech detection) on 10 s of continuous speech. Documented in commit `3c3fffe`.

## Decision: Phase 2 complete, proceed to Phase 3

Phase 2's goal was to prove the ML primitives work end-to-end on real audio. That goal is met:

- Silero VAD reliably fires on speech (85% detection rate on LibriSpeech dev-clean) and rejects silence and noise (max/mean < 0.01).
- WeSpeaker ResNet34-LM produces 256-dim embeddings that cleanly separate different speakers (cosine 0.018) while recognizing the same speaker across different content (cosine 0.556).
- The 48 -> 16 kHz resampler, Kaldi fbank extractor, and EMA-smoothed SpeakerVerifier are all unit-tested and integration-tested.
- D-002R (WeSpeaker as proactive primary) is validated. There is no fallback to consider because the primary works.

Phase 3 (Enrollment + CLI) can begin immediately. It builds on:
- `EcapaTdnn::extract` to produce the enrolled centroid
- `l2_normalize` to sanitize the centroid
- `Profile` serialization with the `EMBEDDING_DIM = 256` constant from D-002R / D-003

No blockers remain.
