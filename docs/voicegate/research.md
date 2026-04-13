# VoiceGate Research & Decisions Log

Domain research and design decisions that inform the phase plans. Update this file whenever a phase's research or a gap analysis pass produces a new decision.

## 1. Speaker Verification Background

**Core problem:** Distinguish one known speaker's voice from other speakers in the same microphone input, in real time, with <50ms end-to-end latency.

**Approach:** Text-independent speaker verification via a neural speaker-embedding network. A fixed-length (192-dim) vector is extracted per short audio window; matching against an enrolled "template" embedding is a cosine similarity.

**Reference architecture:** x-vector / ECAPA-TDNN / ResNet-based speaker embeddings (see Desplanques et al., "ECAPA-TDNN: Emphasized Channel Attention, Propagation and Aggregation in TDNN Based Speaker Verification," Interspeech 2020).

**Why cosine similarity:** ECAPA-TDNN training uses angular margin losses (AAM-softmax). The learned embedding space is designed so that same-speaker vectors cluster by angle, not magnitude. L2-normalizing both vectors and taking the dot product is numerically equivalent to cosine similarity and cheaper than computing `acos`.

**Typical thresholds:**
- >0.85: very strict, risk of false negatives on the enrolled speaker
- 0.70: default balanced
- 0.60: loose, risk of false positives on similar voices
- The PRD exposes this as a GUI slider and defaults to 0.70.

## 2. Silero VAD

**Source:** https://github.com/snakers4/silero-vad
**Model file:** `silero_vad.onnx` (~2 MB)
**License:** MIT
**Paper:** None formally; see model card.

**ONNX tensor shapes:**
- Input `input`: `f32 [1, 512]` — 512 raw audio samples at 16 kHz (32 ms)
- Input `sr`: `i64 [1]` — sample rate scalar, 16000
- Input `state`: `f32 [2, 1, 128]` — GRU hidden state, zeros on first call, then fed forward
- Output `output`: `f32 [1]` — speech probability in [0, 1]
- Output `stateN`: `f32 [2, 1, 128]` — updated hidden state

**Critical implementation notes:**
- VAD is **stateful**. The GRU hidden state must persist across calls. Do not recreate the session per frame.
- 512 samples at 16 kHz = 32 ms. This dictates our **upstream frame size**: 32 ms at 48 kHz = 1536 samples. Using 32 ms frames end-to-end avoids VAD input-size mismatches.
- Inference latency: <1 ms on CPU. Effectively free.
- Do not call VAD on resampled output from a rubato resampler without confirming the rubato output block size equals 512 samples exactly. The project's chosen `FftFixedIn::new(48000, 16000, 1536, 1, 1)` is designed to emit exactly 512 samples per call.

**Decisions:**
- **Decision D-001 (2026-04-13):** Use 32 ms / 1536-sample frames end-to-end (capture, ring buffer, resampler, VAD, embedding window accumulation, gate) to align with Silero VAD input. Rationale: alternative (30 ms) forces awkward VAD windowing; alternative (20 ms) requires buffering two frames per VAD call. Source: PRD §5.4.

## 3. ECAPA-TDNN (Speaker Embedding)

**Source:** https://huggingface.co/speechbrain/spkrec-ecapa-voxceleb
**Training data:** VoxCeleb1+2
**Model size:** ~80 MB (ONNX after export)
**Output:** 192-dimensional speaker embedding, expected to be L2-normalized by the caller.

**Export path:**
- Upstream is a PyTorch model published by SpeechBrain, not a pre-made ONNX file. Export is done via `torch.onnx.export` with opset 14 and a dynamic `audio_length` axis.
- PRD §7.2 explicitly warns the SpeechBrain model may be wrapped in additional layers. The correct subgraph to export is `classifier.mods.embedding_model`, but verification is required.
- **Mandatory:** `scripts/export_ecapa.py` must include a PyTorch ↔ ONNX equivalence check asserting `max(|pytorch_out - onnx_out|) < 1e-4` on a fixed dummy input. If this check fails, we're exporting the wrong submodule.

**Fallback model (WeSpeaker):**
- https://github.com/wenet-e2e/wespeaker publishes official ONNX exports of speaker verification models (`voxceleb_resnet34.onnx` and similar).
- WeSpeaker's embedding dim is 256, not 192. Any code path that assumes 192 must go through a constant in `src/ml/embedding.rs`, not a hard-coded literal, so the fallback is a one-line change.

**Decisions:**
- **Decision D-002 (2026-04-13):** Primary model is SpeechBrain ECAPA-TDNN exported via our script. Fallback is WeSpeaker ResNet34 pre-exported ONNX. Fallback decision is made **inside Phase 2** based on the discrimination test, not deferred to a later phase. Rationale: discovering failure in Phase 4 forces rewind through Phase 3 (enrollment re-verification).
- **Decision D-003 (2026-04-13):** Embedding dimension is a `const EMBEDDING_DIM: usize` in `src/ml/embedding.rs`, not a literal. Profile.bin stores the dim in its header so a profile from a different model is safely rejected by `Profile::load`.

**Sliding window strategy** (PRD §5.5):

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Window length | 1.5 s (24000 samples at 16 kHz) | Long enough for stable embedding, short enough for real-time |
| Minimum window before first extraction | 0.5 s (8000 samples) | Avoid noisy early embeddings |
| Re-extract interval | 200 ms (3200 samples) | 5 Hz update rate; CPU budget ~50 ms per call × 5/s = 250 ms/s ≈ 25% of one core worst case |
| EMA alpha | 0.3 | Balances responsiveness and flicker |

## 4. Cross-Platform Audio I/O (cpal)

**Library:** https://docs.rs/cpal/latest/cpal/
**Backends:**
- Windows: WASAPI (default)
- Linux: ALSA (works alongside PipeWire/PulseAudio which expose ALSA-compatible interfaces)
- macOS: CoreAudio (out of scope for v1)

**Format contract:**
- `f32` samples, 48 kHz, mono (downmix in the capture callback if a stereo device is selected)
- Frame size: fixed 1536 samples (32 ms) — see D-001
- **ALSA buffer-size quirk:** some ALSA devices reject `BufferSize::Fixed(1536)` with `StreamConfigNotSupported`. Fall back to `BufferSize::Default` and accumulate variable-size callback deliveries in the ring buffer; drain fixed-size chunks from the processing thread.

**Thread safety:**
- cpal callbacks are called on a real-time audio thread owned by the OS. **No allocations, no locks, no syscalls** in callbacks. Hand off to the processing thread via the SPSC ring buffer.

**Decisions:**
- **Decision D-004 (2026-04-13):** Use `ringbuf` 0.4 SPSC for both input (callback → worker) and output (worker → callback) queues. Capacity = 3 seconds of audio (432 000 f32 samples × 2 queues ≈ 3.5 MB). Rationale: lock-free, zero-alloc push/pop, battle-tested. Source: PRD §4.2, §6.
- **Decision D-005 (2026-04-13):** Handle ALSA's variable-callback case by pushing whatever the callback provides into the ring buffer and having the worker thread pop exactly 1536-sample chunks. The worker blocks on an async condvar (or spins with backoff) when the ring is under-full. Rationale: simplest portable handling; PRD §5.1 acknowledges this need explicitly.

## 5. Virtual Microphone (Platform-Specific)

This is the **biggest cross-platform divergence** in the project.

### Windows: VB-Audio Virtual Cable

- **Install:** User-installed, free, from https://vb-audio.com/Cable/
- **Device names:**
  - VoiceGate writes to: `"CABLE Input (VB-Audio Virtual Cable)"` (appears as an output device)
  - Discord reads from: `"CABLE Output (VB-Audio Virtual Cable)"` (appears as an input device)
- **Detection:** scan `cpal::Host::output_devices()` for the CABLE Input name. If missing, surface an error with the install link in the GUI.

### Linux: PipeWire virtual source (zero-install)

- **No third-party software.** VoiceGate creates its own virtual source at startup and destroys it on exit.
- **Phase 1 approach** (`pw-cli` shell commands, mirrors PRD Appendix C):
  1. Create `voicegate_sink` null-sink
  2. Create `voicegate_mic` virtual source
  3. `pw-link voicegate_sink:monitor_MONO voicegate_mic:input_MONO`
  4. VoiceGate writes audio to `voicegate_sink` via cpal (appears as an output device); Discord reads from `voicegate_mic` as an input device.
- **Phase 6 approach** (`pipewire-rs`): direct PipeWire API calls create a source node whose `process` callback pulls from our output ring buffer. Eliminates the `pw-cli` subprocess dependency. Gated behind the `pipewire-native` cargo feature.
- **PulseAudio fallback** (Ubuntu 22.04 without PipeWire, rare): `pactl load-module module-null-sink` + `module-remap-source`. Tracked module indices for `pactl unload-module` on teardown.

**Decisions:**
- **Decision D-006 (2026-04-13):** Phase 1 uses `pw-cli` shell commands (simplest path to passthrough). Phase 6 adds a `pipewire-rs` native implementation behind the `pipewire-native` feature flag, plus the PulseAudio fallback. Default feature set keeps the `pw-cli` path.
- **Decision D-007 (2026-04-13):** Audio-server autodetect at startup (`pw-cli info` → PipeWire; fallback `pactl info` → PulseAudio; else clear error) is a Phase 6 concern. Phase 1 assumes PipeWire is present (`which pw-cli` check only).

## 6. Threading Model

Three audio-touching threads (PRD §6) plus the UI thread:

| Thread | Priority | Owner | Constraints |
|--------|----------|-------|-------------|
| Input callback (cpal) | RT | OS audio driver | No alloc, no lock, no syscall. Only operation: push to input ring buffer. |
| Processing worker | Normal | Our `std::thread::spawn` | Pulls from input ring, runs ML pipeline, pushes to output ring. May allocate freely between frames but should reuse buffers. |
| Output callback (cpal) | RT | OS audio driver | No alloc, no lock, no syscall. Only operation: pop from output ring buffer. |
| UI (egui) | Normal | `eframe::run_native` main thread | Reads status atomics. Writes config via `Arc<RwLock<Config>>`. Calls `Context::request_repaint()` when the processing worker signals a status change. |

**Inter-thread communication:**
- Ring buffers: `ringbuf::HeapRb<f32>` SPSC (one producer + one consumer per queue)
- Config: `Arc<RwLock<Config>>` (infrequent writes, cheap reads)
- Status: `Arc<AtomicU32>` for similarity score (f32 bit-cast via `to_bits`/`from_bits`), `Arc<AtomicU8>` for gate/VAD LED states
- Shutdown: `Arc<AtomicBool>`

## 7. Latency Budget

Target: <50 ms end-to-end (PRD §13.4).

| Stage | Budget | Notes |
|-------|-------:|-------|
| Input callback → ring buffer | ~0 ms | Just a memcpy |
| Worker pickup jitter | <5 ms | Condvar wake latency |
| Resample 48→16 kHz | <1 ms | rubato `FftFixedIn` |
| Silero VAD | <1 ms | Stateful GRU, 512-sample input |
| ECAPA-TDNN embedding | 5–15 ms | Only runs when VAD active and 200 ms since last |
| Similarity + EMA | <<1 ms | 192-dim dot product |
| Gate + crossfade | <<1 ms | In-place mul |
| Output ring buffer → output callback | ~0 ms | Just a memcpy |
| **Frame duration itself** | 32 ms | Can't be avoided |
| **Total** | ~40 ms | Under budget with headroom |

If a frame is late, the gate defaults to its last decision (no frame skip visible to Discord).

## 8. Decisions Log (index)

| ID | Date | Decision | Phase | Source |
|----|------|----------|------:|--------|
| D-001 | 2026-04-13 | 32 ms / 1536-sample frames end-to-end | 1 | Research §2 |
| D-002 | 2026-04-13 | Primary ECAPA-TDNN, fallback WeSpeaker, decide inside Phase 2 | 2 | Research §3 |
| D-003 | 2026-04-13 | Embedding dim stored in profile.bin header (not literal) | 2, 3 | Research §3 |
| D-004 | 2026-04-13 | `ringbuf` 0.4 SPSC, 3s capacity per queue | 1 | Research §4 |
| D-005 | 2026-04-13 | ALSA variable-callback handled via worker pop in fixed chunks | 1 | Research §4 |
| D-006 | 2026-04-13 | Phase 1 `pw-cli` shell; Phase 6 `pipewire-rs` + PulseAudio fallback | 1, 6 | Research §5 |
| D-007 | 2026-04-13 | Audio-server autodetect is Phase 6 only; Phase 1 assumes PipeWire | 1, 6 | Research §5 |

## 8.1 PRD pseudocode is stale relative to Decision D-001

**Observation (from Pass 1 gap analysis):** The PRD contains pseudocode in two places that uses the 20 ms / 960-sample frame size, then later sections fix the design at 32 ms / 1536 samples:

- **PRD §5.1** (Audio Capture): `buffer_size: cpal::BufferSize::Fixed(960)` and `"Frame size: **20ms**"`
- **PRD §5.3** (Resampler): `"20ms at 48kHz = 960 samples"`, `"20ms at 16kHz = 320 samples"`, `rubato::FftFixedIn::<f32>::new(48000, 16000, 960, 1, 1)`
- **PRD §5.4** (then corrects itself): `"Decision: Use 32ms frame size to align with Silero VAD's expected 512-sample input. This gives us 1536 samples at 48kHz per frame"`

**Resolution:** Decision D-001 (this file, §2) and all phase files use **32 ms / 1536 samples / 512 samples** consistently. Executors must ignore the §5.1 and §5.3 pseudocode frame-size numbers. When reading the PRD, treat §5.4's "Decision" paragraph as authoritative and §5.1/§5.3 as illustrative only.

The plan's phase files are internally consistent — no executor action is required beyond following the phase files. This note exists so future gap-analysis passes don't re-flag the PRD ↔ phase-file divergence as a new issue.

---

## 9. Open Questions

Questions to resolve during implementation. Each must have an owner and a phase by which it must be answered.

| ID | Question | Must answer by | Notes |
|----|----------|---------------:|-------|
| Q-001 | Does `torch.onnx.export` on `classifier.mods.embedding_model` directly produce a valid ECAPA-TDNN ONNX, or does it need a custom forward wrapper? | End of Phase 2 | PRD §7.2 warns about this. If Q-001 is "needs wrapper," the fallback from D-002 may trigger. |
| Q-002 | What's the actual end-to-end latency measured with a click-track loopback on real hardware? | End of Phase 4 | Budget is 50 ms per PRD §13.4. Needs measurement to validate. |
| Q-003 | Does CPU usage stay <10% on a mid-range machine during active use? | End of Phase 4 | PRD §13.5. Measurement methodology goes here once established. |
| Q-004 | Does `pipewire-rs` 0.8 compile on Ubuntu 22.04's stock PipeWire, or does it need a newer runtime? | Start of Phase 6 | If compat is poor, default feature set stays on `pw-cli`. |
