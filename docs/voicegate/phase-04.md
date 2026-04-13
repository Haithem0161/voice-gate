# Phase 4: Gate + Pipeline Integration

**Goal:** Wire Phase 1's audio I/O, Phase 2's ML primitives, and Phase 3's enrolled profile into a full headless pipeline — `voicegate run --headless --profile <path>` — that gates the microphone in real time with <50 ms end-to-end latency and no clicks/pops.

**Dependencies:** Phases 1, 2, 3.
**Complexity:** M

---

## Section 1: Module & File Changes

### 1.1 Files to CREATE

```
src/gate/mod.rs             # pub mod gate;
src/gate/gate.rs            # AudioGate state machine + crossfade
src/pipeline/mod.rs         # pub mod processor;
src/pipeline/processor.rs   # PipelineProcessor — owns resampler, vad, ecapa, window, verifier, gate
```

**Integration tests:**

```
tests/test_gate.rs          # unit + micro-integration tests for AudioGate crossfade / hold
tests/test_pipeline.rs      # end-to-end fixture WAV → pipeline → output WAV assertion
```

### 1.2 Files to MODIFY

| Path | Change |
|------|--------|
| `src/main.rs` | Extend `Run` subcommand with `--headless`, `--profile <path>`; default `Run` without `--passthrough` or `--headless` still errors until GUI lands in Phase 5. |
| `src/lib.rs` | `pub mod gate; pub mod pipeline;`. Extend `VoiceGateError` with `Gate(String)` and `Pipeline(String)` variants. |
| `src/config/settings.rs` | Add **all remaining** config sections: `VadConfig`, `VerificationConfig`, `GateConfig` so the full PRD §5.9 TOML schema is supported. |
| `src/audio/capture.rs` | Expose `SupportedStreamConfig` return alongside the stream so the pipeline knows the actual negotiated rate (for the ALSA fallback case). |

---

## Section 2: Dependencies & Build Config

**No new crates.** This phase is pure integration — every dependency has already been pinned.

**Release build configuration sanity check:**
- `[profile.release]` already has `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`. These matter for the latency budget; verify the phase-6 CI matrix builds with the same settings.

---

## Section 3: Types, Traits & Public API

### 3.1 `src/config/settings.rs` (full Config per PRD §5.9)

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub audio: AudioConfig,
    pub vad: VadConfig,
    pub verification: VerificationConfig,
    pub gate: GateConfig,
    pub enrollment: EnrollmentConfig,
    pub gui: GuiConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VadConfig {
    pub threshold: f32,            // default 0.5
    pub model_path: String,        // default "models/silero_vad.onnx"
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VerificationConfig {
    pub threshold: f32,            // default 0.70
    pub embedding_window_sec: f32, // default 1.5
    pub embedding_interval_ms: u32,// default 200
    pub ema_alpha: f32,            // default 0.3
    pub model_path: String,        // default "models/ecapa_tdnn.onnx"
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GateConfig {
    pub hold_frames: u32,          // default 5 (160 ms at 32 ms per frame)

    /// Crossfade duration in milliseconds. Matches PRD §5.9 TOML key `crossfade_ms`.
    /// Default: 5.0 ms. Converted to samples on demand via `crossfade_samples()`.
    pub crossfade_ms: f32,
}

impl GateConfig {
    /// Derived: `crossfade_ms * sample_rate / 1000`. For the v1 default of 5.0 ms
    /// at 48 000 Hz this is 240 samples (note: PRD §5.9 comment says "256 samples
    /// ≈ 5 ms" but the exact conversion is 240). `AudioGate::new` accepts the
    /// derived sample count, so the GUI and TOML both express the user-facing value
    /// in milliseconds and the gate internally works in samples.
    pub fn crossfade_samples(&self, sample_rate: u32) -> usize {
        (self.crossfade_ms * sample_rate as f32 / 1000.0) as usize
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EnrollmentConfig {
    pub profile_path: String,      // "auto" → Profile::default_path()
    pub min_duration_sec: u32,     // default 20
    pub segment_duration_sec: u32, // default 3
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GuiConfig {
    pub show_similarity_meter: bool,
    pub show_waveform: bool,
}
```

All `Default` impls match the PRD §5.9 TOML values verbatim.

### 3.2 `src/gate/gate.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateState {
    Open,
    Closed,
    Opening { progress: usize },  // samples into the fade-in curve
    Closing { progress: usize },  // samples into the fade-out curve
}

pub struct AudioGate {
    state: GateState,
    hold_frames: u32,
    crossfade_samples: usize,
    frames_since_match: u32,
}

impl AudioGate {
    pub fn new(hold_frames: u32, crossfade_samples: usize) -> Self;

    /// Process exactly one frame (1536 samples @ 48 kHz).
    /// `is_match` is the current verification decision (from SpeakerVerifier).
    /// Modifies `frame` in place: passes it through, zeros it, or crossfades it.
    pub fn process(&mut self, frame: &mut [f32], is_match: bool);

    pub fn state(&self) -> GateState;
    pub fn is_open(&self) -> bool;

    /// Force-closed (used by --bypass=off equivalents, Phase 5 bypass toggle).
    pub fn force_closed(&mut self);
    /// Force-open (used by --bypass=on, Phase 5 bypass toggle).
    pub fn force_open(&mut self);
}
```

**State-machine rules** (per PRD §5.7, made precise):

| Current state | `is_match` | `should_be_open` | Transition |
|---------------|-----------|------------------|-----------|
| `Closed` | true | true | `Closed → Opening { progress: 0 }`; apply linear fade-in to this frame. |
| `Open` | false | false | `Open → Closing { progress: 0 }`; apply linear fade-out to this frame. |
| `Closed` | false | false | Zero the frame in place. |
| `Open` | true | true | Pass through unchanged. |
| `Opening { p }` | \* | true | Continue fade-in from sample `p`. If `p + frame.len() >= crossfade_samples`, transition to `Open` within the frame. |
| `Opening { p }` | \* | false | Abort fade-in: transition to `Closing { progress: 0 }` starting from current gain level. |
| `Closing { p }` | \* | false | Continue fade-out from sample `p`. If complete, `→ Closed` within the frame. |
| `Closing { p }` | \* | true | Abort fade-out: transition to `Opening { progress: 0 }` starting from current gain level. |

`should_be_open = frames_since_match < hold_frames`.

**Crossfade math** (linear; sine-shaped is nicer but linear is sufficient for ~5 ms):

```rust
gain(i) = i as f32 / crossfade_samples as f32    // fade-in
gain(i) = 1.0 - (i as f32 / crossfade_samples as f32)  // fade-out
```

For aborted transitions (e.g. `Opening` interrupted by no-match), compute the current gain at the interruption point and start the new fade from that gain so there is no discontinuity.

### 3.3 `src/pipeline/processor.rs`

```rust
pub struct PipelineProcessor {
    resampler: Resampler48to16,
    vad: SileroVad,
    ecapa: EcapaTdnn,
    window: EmbeddingWindow,
    verifier: SpeakerVerifier,
    gate: AudioGate,

    // Scratch buffers pre-allocated at new().
    scratch_16k: [f32; VAD_CHUNK_SAMPLES],

    // Live status for Phase 5 GUI readers.
    status: Arc<PipelineStatus>,
}

pub struct PipelineStatus {
    /// Latest similarity score as f32 bit-cast into u32.
    pub similarity: AtomicU32,
    /// 0 = closed, 1 = opening, 2 = open, 3 = closing
    pub gate_state: AtomicU8,
    /// 0 = silence, 1 = speech
    pub vad_active: AtomicU8,
    /// 0 = normal, 1 = bypass-on (force open), 2 = bypass-off (force closed)
    pub bypass_mode: AtomicU8,
}

impl PipelineProcessor {
    pub fn new(
        config: &Config,
        profile: Profile,
        status: Arc<PipelineStatus>,
    ) -> anyhow::Result<Self>;

    /// Process one 1536-sample frame at 48 kHz in place.
    /// Worker-thread-only. Single-threaded; not Sync.
    pub fn process_frame(&mut self, frame: &mut [f32; 1536]) -> anyhow::Result<()>;
}
```

**Invariants:**
- `process_frame` performs **zero allocations** after `new()`.
- `process_frame` does not log at `info` level (only `debug`).
- The `Arc<PipelineStatus>` is shared with the GUI thread (Phase 5) and the main thread. Writes are `Relaxed` — the GUI doesn't care about strict ordering.

### 3.4 `src/main.rs` — extended `Run` subcommand

```rust
Run {
    /// Phase 1 passthrough — mic → virtual mic with no processing.
    #[arg(long, conflicts_with = "headless")]
    passthrough: bool,

    /// Headless gated pipeline — requires --profile.
    #[arg(long)]
    headless: bool,

    /// Path to profile.bin. Defaults to Profile::default_path().
    #[arg(long, value_name = "PATH", required_if_eq("headless", "true"))]
    profile: Option<PathBuf>,

    /// Override input device.
    #[arg(long, value_name = "NAME")]
    input_device: Option<String>,
}
```

### 3.5 Error additions

```rust
#[error("gate error: {0}")]
Gate(String),

#[error("pipeline error: {0}")]
Pipeline(String),
```

---

## Section 4: Runtime Behavior

### 4.1 Threading model (full — PRD §6)

```
┌────────────────────────────────────────────────────────────────────┐
│ main thread                                                        │
│   Config::load → Profile::load → create_virtual_mic.setup →        │
│   spawn input_stream, spawn worker, spawn output_stream →          │
│   park until shutdown signal → teardown →                          │
│   vmic.teardown → exit                                             │
└────────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────────┐   ┌──────────────────────┐
│ cpal input callback (RT thread)        │   │ cpal output callback │
│ ─────────────────────────────────      │   │ (RT thread)          │
│ on each audio buffer:                  │   │ ──────────────────── │
│   input_producer.push_slice(buffer)    │   │ on each buffer:      │
│   (no alloc, no lock, no log)          │   │   output_consumer    │
└──────────────────┬─────────────────────┘   │     .pop_slice(out)  │
                   │                          │   (no alloc, no     │
                   ▼                          │    lock, no log)    │
┌──────────────────────────────────────┐      └──────────▲──────────┘
│ processing worker thread              │                 │
│ ────────────────────────────         │                 │
│ loop {                                │                 │
│   if shutdown { break }               │                 │
│   wait for 1536 samples in input ring │                 │
│   pop 1536 into [f32; 1536]           │                 │
│   pipeline.process_frame(&mut frame)  │                 │
│   output_producer.push_slice(&frame)──┼─────────────────┘
│ }                                     │
└──────────────────────────────────────┘
```

### 4.2 Startup sequence for `voicegate run --headless`

1. `Config::load()`.
2. `let profile = Profile::load(config.enrollment.profile_path or CLI arg)?;`
3. `let status = Arc::new(PipelineStatus::default());`
4. `let mut vmic = create_virtual_mic(); let out_device = vmic.setup()?;`
5. Allocate input ring buffer (144 000 samples) and output ring buffer (144 000 samples).
6. `let capture = start_capture(&config.audio.input_device, input_producer)?;`
7. `let pipeline = PipelineProcessor::new(&config, profile, status.clone())?;`
8. Spawn worker thread with `move` captures: `input_consumer`, `output_producer`, `pipeline`, shutdown `Arc<AtomicBool>`.
9. `let output = start_output(&out_device, output_consumer)?;`
10. Install Ctrl-C handler → flip shutdown flag.
11. Park main thread on a condvar until shutdown is signaled.
12. Drop `capture` and `output` (streams end).
13. Join worker thread.
14. `vmic.teardown()?;`
15. Exit 0.

### 4.3 Worker thread main loop

```rust
let mut frame = [0.0f32; 1536];
loop {
    if shutdown.load(Relaxed) { break; }

    // Block until at least 1536 samples are available. Strategy: spin with exponential
    // backoff up to 1 ms, then yield. This keeps latency low without busy-burning a core.
    let mut got = 0;
    while got < 1536 {
        if shutdown.load(Relaxed) { return; }
        let n = input_consumer.pop_slice(&mut frame[got..]);
        got += n;
        if n == 0 {
            std::thread::sleep(Duration::from_micros(500));
        }
    }

    // Run the full pipeline in place on the frame.
    if let Err(e) = pipeline.process_frame(&mut frame) {
        tracing::error!("pipeline error: {e}");
        // Fail-safe: zero the frame and keep going. Never drop frames.
        frame.fill(0.0);
    }

    // Push to output. Should never block because output ring capacity > frame size.
    let pushed = output_producer.push_slice(&frame);
    debug_assert_eq!(pushed, 1536);
}
```

### 4.4 `process_frame` sequence

1. Resample 48 kHz → 16 kHz. `self.resampler.process(frame, &mut self.scratch_16k)?;` Produces exactly 512 samples.
2. Run VAD on `self.scratch_16k`. Returns `prob`. Update `status.vad_active` = (prob > threshold).
3. If `prob > threshold`:
    1. `self.window.push(&self.scratch_16k);`
    2. If `self.window.should_extract()`:
        1. `let live = self.ecapa.extract(self.window.snapshot())?;`
        2. `let result = self.verifier.update(&live);`
        3. `self.window.mark_extracted();`
        4. `status.similarity.store(self.verifier.current_score().to_bits(), Relaxed);`
4. Determine `is_match` for THIS frame:
    - If bypass=on: true (forced open).
    - If bypass=off: false (forced closed).
    - Else: `self.verifier.current_score() > config.verification.threshold`.
5. `self.gate.process(frame, is_match);` — mutates the ORIGINAL 48 kHz frame in place.
6. Write gate state to status: `status.gate_state.store(gate.state().into(), Relaxed);`
7. Return.

### 4.5 Real-time constraints (reminder)

- `process_frame` runs on the worker thread, which is NOT the cpal RT callback. Allocations here are TOLERATED but should be minimized because they can cause latency spikes.
- The input/output cpal callbacks do ONLY `push_slice` / `pop_slice`. No other work.
- `tracing::error!` inside `process_frame` is acceptable (error path only) but `info!`/`debug!` in hot path are not.

### 4.6 Hold-time semantics

`frames_since_match` is incremented every frame where the `is_match` computed in step 4 is `false`, and reset to 0 on `true`. `should_be_open = frames_since_match < hold_frames`.

With default `hold_frames = 5` and 32 ms/frame, this gives **160 ms** of audio passthrough after the last positive match. That is the minimum natural speech pause before the gate closes, which matches what a human conversational partner would tolerate.

### 4.7 Bypass semantics (exposed for Phase 5 GUI)

- `bypass_mode == 0` (normal): the gate decision follows the verifier.
- `bypass_mode == 1` (force open): every frame passes through unchanged. Useful for "I know this is me talking, stop gating" moments.
- `bypass_mode == 2` (force closed): every frame becomes silence. Useful for mute.

Phase 5 wires this to a UI toggle. Phase 4 exposes the atomic; no CLI flag yet.

---

## Section 5: Cross-Platform & Resource Handling

### 5.1 Profile path resolution (runtime)

`--profile` CLI flag → `config.enrollment.profile_path` (if non-`"auto"`) → `Profile::default_path()`.

### 5.2 Model path resolution (runtime)

Same chain as Phase 2. `config.vad.model_path` and `config.verification.model_path` are consulted first; `resolve_model_path` is the fallback.

### 5.3 Capture stream rate mismatch

If the selected input device doesn't support 48 kHz at all, `start_capture` fails with a clear error. Phase 6 may add an automatic resample-at-capture fallback; Phase 4 treats 48 kHz as a hard requirement.

### 5.4 Virtual mic teardown on panic

If the worker thread panics (should never happen because `process_frame` catches all errors, but defensive), the main thread's condvar wait will be unblocked by the panic propagating. `vmic.teardown()` MUST still run. Wrap the spawn in a `catch_unwind` or use a drop-guard:

```rust
struct VirtualMicGuard<'a>(&'a mut dyn VirtualMic);
impl Drop for VirtualMicGuard<'_> {
    fn drop(&mut self) { let _ = self.0.teardown(); }
}
```

### 5.5 Long-running stability

- Memory: no `Vec::push` inside `process_frame`; `EmbeddingWindow::push` trims old samples, keeping allocation bounded. Verify with `heaptrack` during the soak test.
- File descriptors: no per-frame file I/O. `Profile::load` happens exactly once at startup.
- No clock drift issues — the ring buffers handle it. If the input rate is slightly slower than output, the output ring will underflow and cpal will insert silence; this is audible but not a crash.

---

## Section 6: Verification

### Pre-test setup

1. Phase 3 has produced `tests/fixtures/speaker_a_enroll.wav`, `speaker_a.wav`, `speaker_b.wav`.
2. A new fixture `tests/fixtures/mixed_ab.wav` is created: speaker A and speaker B talking alternately (not overlapping) for ~15 seconds. Document how it was recorded in `research.md`.

### Automated tests

3. **`test_gate_passthrough_open`** — `AudioGate::new(5, 240)`, force-open, process a sine frame, assert output equals input. (240 samples = 5 ms at 48 kHz, the v1 default.)
4. **`test_gate_silence_closed`** — Force-closed, process a sine, assert all zeros.
5. **`test_gate_crossfade_monotonic`** — Start with `is_match=false` steady-state, then supply `is_match=true` for a single frame. Assert the output frame has a monotonically increasing envelope over the first `crossfade_samples` samples.
6. **`test_gate_no_clicks`** — Steady sine at 440 Hz; toggle `is_match` true/false every 10 frames; compute the per-sample first-difference of the output; assert the max absolute first-difference is < 0.05 (no hard discontinuities). This is the "no clicks" test.
7. **`test_gate_hold_time`** — `is_match=true` for 3 frames, then `false` for 4 frames. With `hold_frames=5`, assert the gate stays open for the first 5 false-frames (i.e. it closes on frame 4 or 5, not immediately).
8. **`test_gate_aborted_fade_in`** — Send one `true` frame (starts `Opening`), then one `false` frame before the crossfade completes. Assert no click at the transition (first-difference < 0.05).
9. **`test_pipeline_passes_enrolled`** — Build a `PipelineProcessor` with a profile enrolled from `speaker_a_enroll.wav`. Feed it `speaker_a.wav` (resampled to 48 kHz if needed). Assert the output RMS ≥ 0.8 × input RMS (most audio passes through).
10. **`test_pipeline_blocks_stranger`** — Same pipeline, feed `speaker_b.wav`. Assert the output RMS ≤ **0.1 × input RMS** (≥90% silenced — matches PRD §13.3 literally).
11. **`test_pipeline_mixed_ab`** — Feed `mixed_ab.wav`. Split the known-A and known-B time ranges (documented in fixture metadata). Assert the A ranges have output RMS ≥ 0.7 × input RMS and the B ranges have output RMS ≤ **0.15 × input RMS** (slightly looser than pure-stranger to account for VAD hysteresis and crossfade bleed at A→B transitions).

### Manual smoke tests

12. **Headless run on Linux:**
    - Preconditions: Phase 3 `voicegate enroll --wav tests/fixtures/speaker_a_enroll.wav` has produced the default profile.
    - `cargo run --release -- run --headless` starts.
    - Speaker A talks into the mic; recording `voicegate_mic` shows speaker A's audio.
    - A different person (or playback of `speaker_b.wav` through room speakers) talks; recording `voicegate_mic` shows silence or near-silence.
    - Ctrl-C exits cleanly; PipeWire nodes torn down.
13. **Latency measurement:**
    - Use a click-track loopback: play a known click track into the mic at a known t₀; record the output from `voicegate_mic`; find t₁ of the click in the recording; `latency = t₁ - t₀`.
    - Assert `latency < 50 ms` (PRD §13.4).
    - Document methodology + results in [research.md](research.md) §7.
14. **CPU usage measurement:**
    - Run `htop` / `top` while the pipeline is active with a real speaker.
    - Assert average CPU < 10% of one core on a mid-range machine (PRD §13.5).
    - Document in research.md §7.
15. **30-minute soak test:**
    - Run the pipeline for 30 minutes with a real speaker talking intermittently.
    - Assert no crashes, no memory growth (`ps -o rss`), no PipeWire node leaks, no audible glitches.

### Lint / build

16. `cargo clippy -- -D warnings` clean.
17. `cargo fmt --check` clean.
18. `cargo build --release` succeeds.

### Acceptance thresholds

- `test_gate_no_clicks` max first-difference < 0.05.
- `test_pipeline_passes_enrolled` output RMS ≥ 0.8 × input RMS.
- `test_pipeline_blocks_stranger` output RMS ≤ **0.1** × input RMS (matches PRD §13.3 >90% silencing).
- `test_pipeline_mixed_ab` B-range output RMS ≤ **0.15** × input RMS; A-range ≥ 0.7 × input RMS.
- Measured end-to-end latency < 50 ms.
- Measured CPU < 10% of one core.
- 30-minute soak test: no crashes, no memory growth (defined as RSS delta < 5 MB between minute 1 and minute 30).

---

## Section 6+: PRD Gap Additions

### 6.1 Config key `gate.crossfade_ms` unit handling (Pass 2, G-012, MEDIUM)

**Gap:** PRD §5.9 specifies the TOML key as `crossfade_ms = 5` (a duration in milliseconds). The Phase 4 draft originally defined the Rust field as `crossfade_samples: usize` with value 256. This created:

1. **Deserialization mismatch** — the TOML key `crossfade_ms` does not map to a field named `crossfade_samples`, so serde would either reject the config or ignore the value.
2. **Unit-mismatch time bomb** — if a user reads the PRD schema and writes `crossfade_samples = 5` into their config (mimicking the PRD's `5`), they would get **5 samples ≈ 0.1 ms of crossfade**, a ~50× underscaling that causes audible clicks. Exactly the regression the crossfade was designed to prevent.
3. **Wrong sample count** — the PRD comment says "256 samples ≈ 5 ms at 48 kHz", but the exact conversion is `5 ms × 48 000 Hz / 1000 = 240` samples. The "256" value is a power-of-two approximation, off by 6.67%.

**Resolution (applied above in §3.1):**

1. **`GateConfig.crossfade_ms: f32`** matches the PRD TOML key exactly. Default `5.0`.
2. **Helper `GateConfig::crossfade_samples(&self, sample_rate: u32) -> usize`** derives the sample count as `(crossfade_ms * sample_rate / 1000) as usize`. For the default this is 240, not 256.
3. **`AudioGate::new(hold_frames, crossfade_samples)`** still takes the sample count, not the duration. The pipeline constructor computes `config.gate.crossfade_samples(config.audio.sample_rate)` once at startup and passes it to `AudioGate::new`.
4. **Test `test_gate_passthrough_open` uses `240`** (the v1 default) to match the default GateConfig, not the stale `256` from the PRD comment.

**Rationale for two-field approach (duration in config, samples in gate):** users think in milliseconds; the crossfade math works in samples. The helper method is the single conversion point. No runtime unit confusion possible.

### 6.2 Pipeline-blocks-stranger threshold aligned to PRD §13.3 (Pass 2, G-013, MEDIUM)

**Gap:** PRD §13.3 success criterion 3 requires "Other voices in the same room are silenced **>90%** of the time". Phase 4's original `test_pipeline_blocks_stranger` asserted `output RMS ≤ 0.2 × input RMS`, which is a **80%** amplitude reduction (≥80% silenced), under-stringent relative to the 90% target. While 0.2× RMS is ~96% power reduction — so the practical outcome exceeds the criterion in power terms — RMS is the amplitude-domain measurement users perceive directly and the PRD's literal reading is amplitude-based.

**Resolution (applied to §6 tests 10 and 11 + acceptance thresholds):** Tighten the RMS thresholds to match the PRD criterion literally:

- `test_pipeline_blocks_stranger`: `output RMS ≤ 0.1 × input RMS` (was 0.2×) — matches the >90% silencing target directly.
- `test_pipeline_mixed_ab` B-range: `output RMS ≤ 0.15 × input RMS` (was 0.3×) — slightly looser than the pure-stranger case because VAD hysteresis and crossfade bleed over at A→B transitions, but still above the naïve 0.3× threshold.
- `test_pipeline_passes_enrolled`: unchanged at `≥ 0.8 × input RMS` — the "voice passes through clearly" criterion has no numeric target in PRD §13.2, and 0.8× is a reasonable floor given gate crossfade attenuation at utterance boundaries.

The updated test assertions and acceptance-thresholds section below this phase's verification section are the source of truth; the original 0.2×/0.3× values are deprecated.
