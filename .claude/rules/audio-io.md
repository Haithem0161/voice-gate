---
paths:
  - "src/audio/**"
  - "src/pipeline/**"
  - "tests/test_audio*"
  - "tests/test_pipeline*"
---

# Audio I/O Rules (cpal + ringbuf + rubato + hound)

This file is the contract for anything that touches audio samples. Read it before writing code under `src/audio/` or `src/pipeline/`.

## The Frame-Size Contract

**32 ms per frame, always, end-to-end.** This is Decision D-001 in `docs/voicegate/research.md`. It cannot be changed without re-training or replacing Silero VAD.

| Stage | Sample rate | Samples per frame |
|-------|------------:|------------------:|
| cpal capture | 48 000 Hz | **1536** |
| Ring buffer | 48 000 Hz | 1536-sample chunks |
| Resampler 48 -> 16 | 48 000 -> 16 000 Hz | 1536 in, **512** out |
| Silero VAD input | 16 000 Hz | **512** (hard requirement) |
| ECAPA-TDNN window accumulator | 16 000 Hz | 24 000 samples (1.5 s) |
| Gate | 48 000 Hz | 1536 |
| cpal output | 48 000 Hz | 1536 |

- 32 ms × 48 000 Hz / 1000 = 1536 samples. 32 ms × 16 000 Hz / 1000 = 512 samples. These are exact, not rounded.
- Do NOT assume the cpal callback will deliver exactly 1536 samples. See "ALSA variable-callback handling" below.
- Do NOT choose 20 ms or 30 ms "because it's a round number." Silero VAD will silently produce garbage on wrong-sized inputs.
- `Config::validate()` rejects any `frame_size_ms` other than 32. The validation is there to fail loud rather than produce garbage audio.

## cpal Rules

- **Host format is `f32`, mono, 48 000 Hz.** If the selected device only supports stereo, downmix in the capture callback via `(l + r) * 0.5`. If it only supports i16, convert with `sample as f32 / i16::MAX as f32`. If it only supports 44.1 kHz, error out with a clear message (v1 does not resample at capture time; that is a future enhancement).
- **Buffer size request**: `BufferSize::Fixed(1536)` is a *request*, not a guarantee. The host may round, may ignore, may vary between calls. See "ALSA variable-callback handling" below.
- **Callbacks run on an RT-priority OS thread.** See `rust-desktop.md` § "Real-Time Safety Checklist". The entirety of the input callback is:
  ```rust
  move |data: &[f32], _info: &cpal::InputCallbackInfo| {
      // downmix if stereo (pre-allocated scratch)
      // push_slice into the producer
  }
  ```
- **Device selection**:
  - Named device: search `host.input_devices()?` for a device whose `name()?` matches the config value.
  - `"default"`: use `host.default_input_device()`.
  - If the named device is missing, log a warning and fall back to default.
- **Error callback**: log at `warn` level and flip an `AtomicBool` to signal the main thread. Do NOT panic.
- **Stream lifetime**: the `cpal::Stream` object must be kept alive (usually stored in a struct) for the callback to keep firing. Dropping the stream stops the callback.

## Ring Buffer (ringbuf 0.4 SPSC)

- `ringbuf::HeapRb<f32>` with `.split()` into (producer, consumer).
- **Capacity = 3 seconds × 48 000 Hz = 144 000 samples per queue.** Two queues (input, output), so ~576 KB × 2 = 1.1 MB total. This absorbs ALSA callback-size jitter and lets the worker block briefly without causing an xrun.
- **Producer**: cpal capture callback for the input queue; processing worker for the output queue.
- **Consumer**: processing worker for the input queue; cpal output callback for the output queue.
- **Push pattern**: `producer.push_slice(&samples)`. Returns the number of samples actually pushed. If the ring is full, samples are silently dropped -- increment an atomic overflow counter so the pipeline can surface it.
- **Pop pattern**: `consumer.pop_slice(&mut scratch)`. Returns the number of samples actually popped. If the ring is empty, returns 0 -- the worker should spin-wait or sleep briefly and retry.
- **Never `try_pop()` in a tight loop without a backoff.** That will burn 100% CPU. Use `std::hint::spin_loop()` for short waits or `std::thread::park_timeout(Duration::from_micros(500))` for longer.

## ALSA Variable-Callback Handling

Some ALSA devices reject `BufferSize::Fixed(1536)` with `StreamConfigNotSupported`. On those devices, the capture callback is invoked with whatever size ALSA chose -- commonly 480, 512, 960, 1024, or 2048 samples. The code must handle this:

1. On `StreamConfigNotSupported`, retry `build_input_stream` with `BufferSize::Default`.
2. In the callback, `push_slice(data)` -- whatever the size is. The ring buffer absorbs the jitter.
3. The processing worker pops exactly 1536 samples per frame. If the ring is under-full, the worker waits briefly and retries.
4. The 3-second ring capacity guarantees no dropouts for any reasonable ALSA chunk size.

## Rubato Resampling (Phase 2+)

- `rubato::FftFixedIn::<f32>::new(48_000, 16_000, 1536, 1, 1)` for capture-side downsampling to Silero VAD's required 16 kHz.
- **Input chunk size is fixed at 1536**. Output is exactly 512 samples (verify this assumption with a test on initialization; do not assume).
- `FftFixedIn` is stateless between calls and allocates internally on each call. Acceptable in the worker thread; NOT acceptable in a callback.
- For upsampling 16 kHz -> 48 kHz (not needed in v1 but may be in future), use `FftFixedOut` with the reversed config.

## Hound (WAV I/O)

- Used for: reading fixture WAVs in tests, writing debug dumps from the worker (feature-gated).
- **Fixture read pattern**:
  ```rust
  let mut reader = hound::WavReader::open("tests/fixtures/speaker_a.wav")?;
  let spec = reader.spec();
  assert_eq!(spec.channels, 1);
  assert_eq!(spec.sample_rate, 16_000);  // or 48_000, depending on the fixture
  let samples: Vec<f32> = match spec.sample_format {
      hound::SampleFormat::Int => reader
          .samples::<i16>()
          .map(|s| s.unwrap() as f32 / i16::MAX as f32)
          .collect(),
      hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
  };
  ```
- **Never call hound from inside a callback.** WAV I/O is file I/O; it allocates and can block.
- All fixture WAVs are mono. If stereo, error out or downmix -- do not silently interpret as mono.

## Virtual Microphone (see `cross-platform.md` for the full table)

- Trait: `pub trait VirtualMic: Send { fn setup(&mut self) -> Result<String>; fn teardown(&mut self) -> Result<()>; fn discord_device_name(&self) -> &str; }`
- Linux: `PwCliVirtualMic` shells out to `pw-cli create-node adapter` + `pw-link` per PRD Appendix C. Returns `"voicegate_sink"` as the cpal output device name to write to.
- Windows: `VbCableVirtualMic` scans cpal output devices for `"CABLE Input (VB-Audio Virtual Cable)"`. If missing, error with the install link.
- `teardown()` must be idempotent -- calling it twice is fine.
- `teardown()` must be called in main.rs's shutdown path, AFTER dropping the cpal streams. Order matters.

## Testing Audio Paths

See `.claude/rules/testing.md`. Highlights:

- Unit-test pure-logic helpers (downmix, frame alignment arithmetic) without touching cpal.
- Integration-test the capture + ring buffer path by stubbing cpal with an in-memory driver OR by recording a fixture WAV through the real pipeline and byte-comparing.
- `test_ring_buffer_sample_order` pushes N samples, pops N samples, asserts the order is preserved.
- `test_gate_passthrough_open` verifies that a gate in Open state is bit-for-bit identity over a 1-frame input.
- `test_resampler_length_invariant` verifies that `FftFixedIn(48k, 16k, 1536, 1, 1)` emits exactly 512 samples per call.

## Context7 is MANDATORY for this module

Before writing or modifying code that touches `cpal`, `ringbuf`, `rubato`, or `hound`:

1. `resolve-library-id` on the crate name
2. `query-docs` with the specific method you need (e.g. "cpal build_input_stream callback signature f32", "ringbuf 0.4 HeapRb split producer consumer")
3. Use the returned example as the template

The API shapes for `cpal` (0.15 vs 0.17), `ringbuf` (0.3 vs 0.4), and `rubato` (0.14 vs 0.15) have all changed in recent releases. Memory is wrong. Always check.
