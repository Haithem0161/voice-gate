# VoiceGate Roadmap

**Project:** VoiceGate — Real-Time Speaker Isolation for Discord
**Start Date:** 2026-04-13
**Target:** Cross-platform Rust desktop application (Windows 10/11, Ubuntu 22.04+) that gates the user's microphone in real time using speaker verification.
**Source PRD:** [voicegate-prd.md](../../voicegate-prd.md)

## Scope (Hard Numbers)

| Metric | Count |
|--------|------:|
| Phases | 6 |
| New Rust source modules | 18 |
| New integration test files | 5 |
| New ONNX models | 2 (Silero VAD, ECAPA-TDNN) |
| New Python scripts | 2 (download_models, export_ecapa) |
| New shell scripts | 1 (setup_pipewire.sh) |
| New `.claude/rules/` files | 6 (rust-desktop, audio-io, ml-inference, gui, module-boundaries, cross-platform) |
| Deleted `.claude/rules/` files | 6 (rust-backend, docker, api-design, auth, migrations, ddd) |
| CLI subcommands | 4 (`run`, `enroll`, `devices`, `run --headless`) |
| GUI screens | 2 (main control panel, enrollment wizard) |
| Target platforms | 2 (`x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`) |
| Success criteria (PRD §13) | 10 |

## Phase Overview Table

| # | Phase Name | Scope | Size | Depends On | Status |
|---|------------|-------|------|------------|--------|
| 1 | Foundation Morph + Audio Passthrough | Delete backend, rewrite rules/CLAUDE/Makefile/CI, scaffold `src/` tree, land mic→virtual-mic loopback on Linux | L | none | not started |
| 2 | ML Inference Primitives | Silero VAD + ECAPA-TDNN ONNX wrappers, resampler, similarity math, fixture WAVs, discrimination test | M | 1 | not started |
| 3 | Enrollment + CLI | `EnrollmentSession`, `profile.bin` format, `voicegate enroll --wav` and `--mic` subcommands | M | 2 | not started |
| 4 | Gate + Pipeline Integration | `AudioGate` state machine, threading model, full config, `voicegate run --headless` | M | 1, 2, 3 | not started |
| 5 | GUI | eframe/egui control panel, enrollment wizard, `AppController` mediator | M | 4 | not started |
| 6 | Cross-Platform Hardening + Release | Audio-server autodetect, VB-Cable detection, negative enrollment, profiling, CI matrix, packaging, install docs | M | 1-5 | not started |

### Phase Size Rubric (adapted for desktop project)

- **S**: <3 new modules, no new crates
- **M**: 3–6 new modules or 1–3 new crates
- **L**: >6 new modules OR requires new ONNX model OR introduces a new platform backend
- **XL**: Reserved for v2+ items

## Dependency Graph

```
                  Phase 1: Foundation Morph + Audio Passthrough (L)
                          │
                          ▼
                  Phase 2: ML Inference Primitives (M)
                          │
                          ▼
                  Phase 3: Enrollment + CLI (M)
                          │
        ┌─────────────────┘
        │
        ▼
Phase 4: Gate + Pipeline Integration (M) ◄── depends on 1, 2, 3
        │
        ▼
Phase 5: GUI (M)
        │
        ▼
Phase 6: Cross-Platform Hardening + Release (M) ◄── depends on all
```

**Parallelism notes:**
- No phases run in parallel. This is a small single-developer project and the work graph is strictly linear.
- Phases 2 and 3 could theoretically overlap (Phase 3's CLI skeleton doesn't require embeddings until verification), but the plan keeps them sequential for simplicity.
- Phase 1 is the only hard blocker for everything — it produces the scaffolding every later phase extends.

## New Modules by Phase

Maps each phase to new Rust source modules under `src/`.

| Phase | New Modules |
|-------|-------------|
| 1 | `main.rs`, `lib.rs`, `audio/mod.rs`, `audio/capture.rs`, `audio/output.rs`, `audio/ring_buffer.rs`, `audio/resampler.rs` (stub), `audio/virtual_mic.rs`, `config/mod.rs`, `config/settings.rs` (initial), `ml/mod.rs` (empty) |
| 2 | `ml/vad.rs`, `ml/embedding.rs`, `ml/similarity.rs`, `audio/resampler.rs` (fleshed out) |
| 3 | `enrollment/mod.rs`, `enrollment/enroll.rs`, `enrollment/profile.rs` |
| 4 | `gate/mod.rs`, `gate/gate.rs`, `pipeline/mod.rs`, `pipeline/processor.rs`, `config/settings.rs` (full) |
| 5 | `gui/mod.rs`, `gui/app.rs`, `gui/enrollment_wizard.rs`, `app_controller.rs` |
| 6 | `audio/audio_server.rs` (Linux autodetect), `enrollment/anti_target.rs` |

**Cumulative module count** (end of Phase 6): 23 source modules.

## New ONNX Models & External Assets by Phase

Replaces `planning.md`'s "New Business Engines" table. External assets gate phase verification, so they are tracked explicitly.

| Phase | New Assets |
|-------|-----------|
| 1 | `scripts/setup_pipewire.sh`, `assets/enrollment_passages.txt` |
| 2 | `models/silero_vad.onnx` (~2 MB, downloaded), `models/ecapa_tdnn.onnx` (~80 MB, exported via Python), `scripts/download_models.py`, `scripts/export_ecapa.py`, `tests/fixtures/speaker_a.wav`, `speaker_b.wav`, `speaker_a_enroll.wav`, `silence.wav`, `noise.wav` |
| 3 | `tests/fixtures/speaker_a_enroll.wav` (reused), `profile.bin` format (magic `VGPR`, version u32, embedding_dim u32, 192×f32, CRC32) |
| 4 | `tests/fixtures/mixed_ab.wav` |
| 5 | No new assets |
| 6 | `models/wespeaker_fallback.onnx` (if ECAPA discrimination fails in Phase 2), Linux AppImage, Windows MSI, CI release artifacts |

## New CLI / GUI Surface by Phase

Tracks user-visible surface area so `frontend-summary.md` updates are traceable to specific phases.

| Phase | CLI Flags / GUI Screens |
|-------|------------------------|
| 1 | `voicegate devices`, `voicegate run --passthrough` |
| 2 | (no new user surface — pure library additions) |
| 3 | `voicegate enroll --wav <file>`, `voicegate enroll --mic <seconds>`, `voicegate enroll --list-passages` |
| 4 | `voicegate run --headless --profile <path>` |
| 5 | `voicegate run` (default: opens GUI), main control panel, enrollment wizard |
| 6 | `voicegate enroll --anti-target <name> --mic <seconds>`, `voicegate doctor` (audio server / VB-Cable diagnostics), packaged AppImage + MSI |

## Gap Analysis Additions

Running log updated after each pass. Gap IDs are assigned as `G-XXX` in pass-order. Every gap must land in a phase file under a `Section 6.x` subsection before a pass is considered complete.

### Pass 1 — Initial (2026-04-13)

- **Date:** 2026-04-13
- **Method:** Line-by-line walk of [voicegate-prd.md](../../voicegate-prd.md) §§ 3–14 and Appendices A/C against all 6 phase files, roadmap, research. Automated by Explore agent; then manually re-triaged against the actual phase file contents (several agent findings turned out to be false positives — the content was already present).
- **Raw findings (from agent):** 10 items flagged (0 CRITICAL, 0 HIGH, 2 MEDIUM, 8 LOW).
- **Post-triage result:** 4 real gaps + 1 cross-cutting observation. The other 5 were false positives — the agent failed to find content that was already in the phase files (tracing init, clap derive feature, ring buffer capacity, bypass wiring, config default scaffolding).
- **Gaps absorbed:**
  - **G-003 (LOW)** → phase-01.md §6.1 — ONNX Runtime install steps pulled into Phase 1's README rewrite.
  - **G-007 (LOW)** → phase-02.md §6.1 — Explicit rule: EMA `current_score` is NOT touched during VAD-inactive frames (prevents silence-flicker).
  - **G-008 (LOW)** → phase-03.md §6.1 — Forward-compat doc comment on `Profile` pointing at the Phase 6 v2 format.
  - **G-010 (LOW)** → phase-05.md §6.1 — Device picker refresh widget detail (⟳ button, manual only, 200 ms cpal enumeration cost).
- **Cross-cutting observation** → research.md §8.1 — The PRD's §5.1 and §5.3 pseudocode uses stale 20 ms / 960-sample frame sizes; §5.4 then fixes it to 32 ms / 1536 samples. Decision D-001 and all phase files use the correct numbers. Flagged so future passes don't re-surface the divergence.
- **Counts:** 0 CRITICAL, 0 HIGH, 0 MEDIUM (after triage), 4 LOW, 1 observation. **No phase was blocked or materially changed.**
- **Conclusion:** Plan is execution-ready. Pass 2 is optional and would focus on: TOML key audit (all §5.9 keys present with defaults), threading-model ownership audit, PRD §13 success-criteria traceability. Pass 2 can be deferred until Phase 1 is under way.

### Pass 2 — Iterative (2026-04-13, complete)

- **Date:** 2026-04-13
- **Method:** Three focused audits: (A) TOML config-key traceability across PRD §5.9 ↔ phase files, (B) threading model ownership per PRD §6, (C) PRD §13 success-criteria verification traceability. Explore-agent automated audit + manual re-triage against actual file contents.
- **Audit A — Config keys (24 keys checked):** 22 ✅ / 1 ⚠ / 1 ❌. Two unit-mismatch bugs at the serde boundary — see G-011 and G-012 below.
- **Audit B — Threading (4 threads + 9 primitives checked):** **All 13 ✅.** No splits, no duplicated ownership, no missing real-time constraints. PRD's `Arc<AtomicF32>` reference is correctly resolved as `Arc<AtomicU32>` bit-cast in phase-04.md §3.3 (stable Rust has no `AtomicF32`).
- **Audit C — Success criteria (10 criteria checked):** All 10 traceable to concrete test steps. Criterion 3 (>90% silencing) flagged as having under-stringent threshold → absorbed as G-013.
- **Gaps absorbed (all MEDIUM, ALL FIXED IN THIS PASS, not just documented):**
  - **G-011** (phase-01.md §6.2 + direct fix to §3.6 `AudioConfig`): Renamed `frame_size_samples: usize` → `frame_size_ms: u32` to match PRD §5.9 TOML key exactly. Added `frame_size_samples()` helper method for on-demand conversion. Added `Config::validate()` that rejects any value other than `frame_size_ms == 32` at load time with a clear error (Silero VAD alignment per Decision D-001). Prevents the silent serde-mismatch failure mode.
  - **G-012** (phase-04.md §6.1 + direct fix to §3.1 `GateConfig`): Renamed `crossfade_samples: usize` → `crossfade_ms: f32`. Added `crossfade_samples(sample_rate)` helper. Default corrected from "256 (~5 ms)" to **exactly 240 samples** (5.0 ms × 48 kHz / 1000 = 240; PRD's 256 was a power-of-two approximation off by 6.67%). `test_gate_passthrough_open` updated from `AudioGate::new(5, 256)` to `AudioGate::new(5, 240)`.
  - **G-013** (phase-04.md §6.2 + direct fix to §6 tests 10–11 + acceptance thresholds): Tightened `test_pipeline_blocks_stranger` from `≤ 0.2 × input RMS` to **`≤ 0.1 × input RMS`** (≥90% silencing, matches PRD §13.3 literally). `test_pipeline_mixed_ab` B-range tightened from `≤ 0.3×` to `≤ 0.15×`.
- **Counts:** 0 CRITICAL, 0 HIGH, **3 MEDIUM** (all resolved), 0 LOW.
- **Conclusion:** Plan is execution-ready and internally consistent. The config schema now deserializes cleanly from PRD §5.9 TOML without rename-attributes or unit-conversion traps. Test thresholds match the PRD success criteria literally. **Pass 3 is not required at this time.**

### Pass 3+ — Continue until 0 true gaps

A pass is only "done" when the next pass finds 0 new gaps at any severity. Target: ≤3 passes.

### Verification Pass (final, end of Phase 6)

Audit N=20 representative items across all phases (mix of Critical/High/Medium/Low) against the built artifact. Each item must have:
1. A concrete file/function in `src/`
2. A verification step in its phase file
3. A passing test or documented manual check

Report as YAML frontmatter in [PHASES-1-6-VERIFICATION.md](PHASES-1-6-VERIFICATION.md).

## Known Risks (Roll-Up)

These are tracked at the roadmap level because they can invalidate multiple phases at once.

1. **ECAPA-TDNN ONNX export** — PRD §7.2 flags export uncertainty. Mitigated by Phase 2 verification gate (`test_embedding_discrimination` must pass) and in-phase WeSpeaker fallback decision.
2. **Overlapping speech** — PRD §11.1 acceptance in v1; negative enrollment in Phase 6 adds second line of defense.
3. **ALSA buffer-size quirks** — some devices reject `BufferSize::Fixed(1536)`. Phase 1 must handle `BufferSize::Default` by buffering variable callbacks into the ring buffer.
4. **PipeWire permission / version edge cases** (PRD §11.4) — Phase 1 uses `pw-cli` shell commands (simplest path); Phase 6 adds PipeWire-native → `pw-cli` → `pactl` → clear-error fallback chain.
5. **CLAUDE.md Context7 rot** — rule-file rewrites must explicitly re-enumerate crates (`cpal`, `ort`, `eframe`, `egui`, `rubato`, `hound`, `ringbuf`, `ndarray`, `dirs`, `thiserror`, `serde`, `toml`, `anyhow`, `tracing`, `which`, `pipewire`). Phase 1 verification grep catches backend residue.
6. **Backend `.memory/cursor.json` uncommitted changes** — must be moved to `.memory/cursor.json` at repo root before deleting `backend/` directory.
