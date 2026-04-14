# VoiceGate Status Tracker

**Last updated:** 2026-04-14 (Phase 2 complete with load-bearing discrimination test PASSING)
**Overall status:** Phases 1 and 2 complete. Phase 2's load-bearing gate passed with huge margin: `test_embedding_discrimination` reports cosine(speaker_a, speaker_b) = 0.018 vs a < 0.5 threshold, validating D-002R (WeSpeaker ResNet34-LM as primary speaker embedding model). 43/43 tests pass across all suites (32 lib + 9 ml integration + 2 ort smoke). Three new gaps discovered and FIXED in-pass during Phase 2 execution (G-017 ORT version 1.17 -> 1.22, G-018 WeSpeaker needs Kaldi fbank features not raw audio, G-019 Silero v5 needs a 64-sample context buffer prepended to each input chunk). Phase 1's G-015 and G-016 remain deferred to Phase 4 (they are pipeline-integration concerns, not ML-primitive concerns).

## Phase Status Table

| # | Phase | Status | Started | Completed | New Modules | New Tests | New Crates | CLI/GUI Surface Added |
|---|-------|--------|---------|-----------|-------------|-----------|------------|------------------------|
| 1 | Foundation Morph + Audio Passthrough | structurally complete (manual gate partial) | 2026-04-14 | 2026-04-14 | 11/11 | 6/0 | 17/16 (+ ctrlc via `cargo add`) | `devices`, `run --passthrough` |
| 2 | ML Inference Primitives | complete (all 43 tests pass, discrimination cos = 0.018) | 2026-04-14 | 2026-04-14 | 5/4 (+ fbank) | 35/10 (fbank 8 + resampler 4 + embedding 5 + similarity 9 + vad integration 3 + discrimination 3 + math 3) | 18/16 (+ realfft via `cargo add`) | none |
| 3 | Enrollment + CLI | not started | — | — | 0/3 | 0/8 | 0/1 | `enroll --wav`, `enroll --mic`, `enroll --list-passages` |
| 4 | Gate + Pipeline Integration | not started | — | — | 0/4 | 0/11 | 0/0 | `run --headless` |
| 5 | GUI | not started | — | — | 0/4 | 0/1 | 0/1 | `run` (default → GUI), main screen, enrollment wizard |
| 6 | Cross-Platform Hardening + Release | not started | — | — | 0/2 | 0/7 | 0/1 | `doctor`, `enroll --anti-target`, AppImage, MSI |

Legend: `N/M` = N completed of M planned. Cells are filled in as each phase progresses.

## Cumulative Totals

| Metric | Before (backend template) | Current | Target (end of Phase 6) |
|--------|---------------------------|---------|-------------------------|
| Rust source modules | 12 (backend domains) | 12 | 23 desktop modules + 0 backend |
| Integration test files | 0 | 0 | 5 (`test_ml`, `test_enrollment`, `test_gate`, `test_pipeline`, plus phase-6 `test_audio_server_detect`/`test_profile_v1_to_v2_upgrade` in-file) |
| Rust crate dependencies | ~20 (axum/sqlx/tower/...) | 20 | 19 desktop crates (cpal, rubato, ort, ndarray, ringbuf, eframe, egui, hound, serde, toml, clap, anyhow, thiserror, tracing, tracing-subscriber, dirs, which, crc32fast, rfd, ureq; optional: pipewire) |
| `.claude/rules/*.md` files | 8 (backend rules) | 8 | 8 (desktop rules: rust-desktop, audio-io, ml-inference, gui, module-boundaries, cross-platform, testing, planning) |
| ONNX model files | 0 | 0 | 2 (`silero_vad.onnx`, `ecapa_tdnn.onnx`) |
| Test fixture WAVs | 0 | 0 | 6 (`speaker_a`, `speaker_b`, `speaker_a_enroll`, `silence`, `noise`, `mixed_ab`) |
| CLI subcommands | 0 | 0 | 4 (`devices`, `run`, `enroll`, `doctor`) |
| GUI screens | 0 | 0 | 2 (main control panel, enrollment wizard) |
| Shipping targets | 0 | 0 | 2 (`x86_64-unknown-linux-gnu` AppImage, `x86_64-pc-windows-msvc` MSI) |
| PRD §13 success criteria met | 0 / 10 | 0 / 10 | 10 / 10 |

## Gap Analysis Summary

Per-pass summary with counts and category breakdown. Updated after each pass.

### Pass 0 — Plan authored

- **Date:** 2026-04-13
- **Method:** Initial plan authoring from the PRD. Not a formal gap analysis pass.
- **Coverage observations:**
  - All 10 PRD §13 success criteria are referenced in a phase file's verification section.
  - All 6 PRD §11 known challenges have documented mitigations (overlap accepted, anti-targets for similar voices, re-enrollment for voice variation, platform detection for virtual device, profiling for CPU).
  - All 18 TOML keys from PRD §5.9 are accounted for in `src/config/settings.rs` Phase 4 sections.
  - All 4 cpal/ML module files from PRD §4.2 are covered.

### Pass 1 — Initial (2026-04-13, complete)

- **Date:** 2026-04-13
- **Method:** Explore-agent automated PRD walk + human re-triage against actual phase file contents.
- **Raw agent findings:** 10 (0C / 0H / 2M / 8L)
- **Post-triage gaps:** 4 LOW + 1 cross-cutting observation. 5 agent findings were false positives (content already present in phase files — agent missed tracing init, clap derive feature, ring buffer sizing, bypass wiring, and intentional partial config).
- **Distribution (real gaps):** 0 CRITICAL / 0 HIGH / 0 MEDIUM / 4 LOW
- **Absorbed into:**
  - phase-01.md §6.1 — G-003 ONNX Runtime install (README scope)
  - phase-02.md §6.1 — G-007 EMA-during-silence rule
  - phase-03.md §6.1 — G-008 Profile forward-compat comment
  - phase-05.md §6.1 — G-010 Device picker refresh widget
  - research.md §8.1 — PRD §5.1/§5.3 pseudocode staleness (not a gap, observation)
- **Conclusion:** Plan is execution-ready. No blockers, no materially affected phases. See roadmap.md "Pass 1 — Initial" for details.

### Pass 2 — Iterative (2026-04-13, complete)

- **Date:** 2026-04-13
- **Method:** Three focused audits (Config keys / Threading / Success criteria) via Explore agent + manual re-triage.
- **Items checked:** 24 config keys + 4 threads + 9 inter-thread primitives + 10 success criteria = 47.
- **Raw agent findings:** 2 MEDIUM (both in Audit A).
- **Post-triage gaps:** 3 MEDIUM (added G-013 for under-stringent RMS threshold flagged during Audit C).
- **Distribution:** 0 CRITICAL / 0 HIGH / **3 MEDIUM** / 0 LOW. **All resolved in-pass with direct fixes to the affected phase files (not just documented).**
- **Absorbed into:**
  - phase-01.md §6.2 + direct §3.6 fix — G-011 `frame_size_ms` rename + `Config::validate()`.
  - phase-04.md §6.1 + direct §3.1 fix — G-012 `crossfade_ms` rename + exact 240-sample default.
  - phase-04.md §6.2 + direct §6 tests 10–11 + acceptance-threshold updates — G-013 tightened RMS thresholds from 0.2×/0.3× to 0.1×/0.15× to match PRD §13.3.
- **Audit B threading model:** **All 13 items ✅.** Clean bill of health — no splits, no duplication, no missing constraints.
- **Audit C success criteria:** All 10 PRD §13 criteria traceable to concrete tests. Criterion 3's test tightened via G-013.
- **Conclusion:** Plan is execution-ready. Pass 3 not required.

### Pass 3 — Execution discovery (2026-04-14, Phase 1 runtime testing)

- **Date:** 2026-04-14
- **Method:** Running `voicegate run --passthrough` end-to-end on the dev machine (PipeWire 1.0.5, cpal 0.15, Ubuntu 24.10-class). Not a pre-execution audit; gaps found by executing the built binary and reading real error messages.
- **Gaps found:** 3 (1 HIGH fixed in-place, 1 MEDIUM deferred to Phase 2, 1 HIGH deferred to Phase 2)
- **Absorbed into:**
  - phase-01.md §6.3 — **G-014 (HIGH, FIXED)**: PRD Appendix C's `pw-cli create-node adapter ... && pw-link` does not work on PipeWire 1.0.x because `pw-cli create-node` creates client-owned nodes that die with the pw-cli process. Replaced by spawning `pw-loopback` as a long-running child process in `PwCliVirtualMic::setup`; Drop impl reaps it. `scripts/setup_pipewire.sh` rewritten to match. Code lands in commit `fdf2204`.
  - phase-01.md §6.4 — **G-015 (MEDIUM, DEFERRED)**: cpal 0.15's ALSA backend reports only f32 in `supported_input_configs` for the dev machine's Intel HDA / ALC623 input, but actual 48 kHz f32 open fails with `snd_pcm_hw_params` I/O error 5. `arecord` at 48000 S16_LE works fine. Deferred to Phase 2's resampler work; Phase 2 will add i16→f32 capture conversion and optional 44100→48000 pre-resample.
  - phase-01.md §6.5 — **G-016 (HIGH, DEFERRED)**: cpal 0.15 on Linux enumerates only ALSA-level device names (`default`, `pipewire`, `hw:*`), not individual PipeWire nodes. Even though `voicegate_sink` is successfully created as a PipeWire node by `pw-loopback`, cpal's `Host::output_devices()` does not expose it, so `start_output("voicegate_sink", ...)` fails. Pragmatic Phase 2 fix: set `PIPEWIRE_NODE=voicegate_sink` in the process environment before building the cpal output stream, or use `pw-metadata` for per-process default routing. Long-term fix: bypass cpal on Linux and write directly via `pipewire-rs` (Phase 6 `pipewire-native` feature).
- **Distribution:** 2 HIGH / 1 MEDIUM / 0 LOW.
- **Status:** G-014 resolved in step 7 of Phase 1 execution. G-015 and G-016 documented as Phase 2 work items.
- **Report:** [PHASE-1-VERIFICATION.md](PHASE-1-VERIFICATION.md).

### Pass 4 — Execution discovery (2026-04-14, Phase 2 runtime testing)

- **Date:** 2026-04-14
- **Method:** Running the Phase 2 integration test suite against real ONNX models and real LibriSpeech fixtures. Not a pre-execution audit; gaps found by executing `cargo test --test test_ml -- --nocapture --test-threads=1` and reading the real session errors.
- **Gaps found:** 3 (all HIGH, all FIXED in-pass). Plus **D-002R validated** via the load-bearing discrimination test.
- **Absorbed into:**
  - phase-01.md §6.1 (corrected) + README.md — **G-017 (HIGH, FIXED)**: The pinned `ort = "=2.0.0-rc.10"` crate requires ONNX Runtime 1.22.x, not 1.17.0 as Phase 1 cited. The first ort `Session::builder` call panics with "expected 1.22.x, but got 1.17.0". Re-installed 1.22.0 (the latest 1.22 release that ships a Linux x64 tarball — 1.22.1 and 1.22.2 are NuGet-only patch releases). Code lands in commit `0f806dc`.
  - phase-02.md §§1.1, 3.3, 3.6, 4.2 + new `src/ml/fbank.rs` — **G-018 (HIGH, FIXED)**: WeSpeaker's ONNX accepts `f32[B, T, 80]` Kaldi fbank features, not raw `[1, T]` audio. The ort smoke test caught this at step 5 of Phase 2 execution. Wrote a pure-Rust `FbankExtractor` matching `torchaudio.compliance.kaldi.fbank` with WeSpeaker's exact settings (80 Mel bins, 25 ms frame, 10 ms shift, Hamming window, preemphasis 0.97, CMN, `waveform * (1 << 15)` pre-scale). `realfft 3.5.0` promoted from transitive to direct dep. Code lands in commit `3ea9365`.
  - phase-02.md + `src/ml/vad.rs` — **G-019 (HIGH, FIXED)**: Silero VAD v5 expects `[1, 576]` input = 64 context samples from the previous call + 512 new samples. Initial implementation fed `[1, 512]` and got ~0 probability on ALL inputs. Reference is `silero-vad/src/silero_vad/utils_vad.py::OnnxWrapper::__call__`. Added a `context: [f32; 64]` field to `SileroVad`; each `prob()` call prepends the context, runs the session, saves the last 64 samples of the concatenated input as the new context. After the fix, `test_vad_detects_speech` reports 264/309 chunks (85%) above 0.5 on 10 s of LibriSpeech speech. Code lands in commit `3c3fffe`.
- **Also resolved:** Q-001 (the ECAPA ONNX export question) is **closed** as superseded by D-002R. Phase 2 proved that D-002R's WeSpeaker path works end-to-end.
- **Distribution:** 3 HIGH, all fixed. 0 deferred.
- **Report:** [PHASE-2-VERIFICATION.md](PHASE-2-VERIFICATION.md).

### Pass 5+ — Continue until 0 true gaps

Target: ≤5 passes total. Each pass appends subsections to affected phase files (6.1, 6.2, ...).

### Verification Pass — Final

Held after Phase 6 completes. Audits N=20 representative items across all phases. Report lives in `PHASES-1-6-VERIFICATION.md` with YAML frontmatter:

```yaml
---
phase: voicegate-phases-1-6
verified: <ISO timestamp>
status: complete | gaps_found
score: N/M must-haves verified
gaps: [...]
prd_13_success_criteria:
  - "Enrollment completes in <60 seconds": true | false
  - "Enrolled user's voice passes through clearly": true | false
  - "Other voices silenced >90% of the time": true | false
  - "End-to-end latency <50ms": true | false
  - "CPU usage <10%": true | false
  - "No audible clicks/artifacts at gate transitions": true | false
  - "Runs stably for hours without crashes or memory leaks": true | false
  - "Threshold is tunable enough for similar voices": true | false
  - "Works on Windows 10/11 AND Ubuntu 22.04+ with same codebase": true | false
  - "Linux: virtual mic created/destroyed automatically (zero extra install)": true | false
---
```

## Blockers & Notes

### Current blockers

None. Plan is authored; implementation is ready to begin with Phase 1.

### Critical dependencies (roll-up from phase files)

| Phase | Blocker | Notes |
|-------|---------|-------|
| 2 | ECAPA-TDNN ONNX export success | If `test_embedding_discrimination` fails, fall back to WeSpeaker inside Phase 2 (Decision D-002). |
| 2 | ONNX Runtime shared library installed | Prerequisite, documented in README. `ort` with `load-dynamic` defers loading until first session create. |
| 4 | Hardware access to a real mic + a second speaker | Needed for Phase 4 smoke tests. `mixed_ab.wav` fixture is the automated-test substitute. |
| 6 | Fresh Linux + Windows VMs | For packaging verification. Cheapest path: GitHub Actions `ubuntu-latest` and `windows-latest` runners with `cargo build --release` verifying headless operation, plus manual VM runs for GUI. |

### Known limitations (documented, not fixed in v1)

1. **Overlapping speech** (PRD §11.1) — accepted. Mitigated by mic proximity.
2. **SIGKILL leaves PipeWire nodes behind** — Phase 1 documents the manual recovery. Phase 6 may add a systemd user service file to handle orphans.
3. **macOS support** — out of scope for v1.
4. **Unsigned Windows MSI** — v1 ships unsigned; users will see a SmartScreen warning. Signing deferred to v1.1.
5. **ECAPA-TDNN downloaded on first run (80 MB)** — documented in README + GUI prompt. Avoids inflating the installer.

### Parallel track notes

None. This is a single-developer linear plan.
