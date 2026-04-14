---
paths:
  - "src/**/*.rs"
  - "Cargo.toml"
  - "Cargo.lock"
---

# Rust Desktop App Rules (VoiceGate)

VoiceGate is a single-binary Rust desktop app. There is no async runtime outside of what `eframe` internally needs for the UI thread. There is no shared database, no HTTP server, and no dependency injection container. State is owned by the main thread and passed explicitly to workers via `Arc<...>` + channels + ring buffers.

## Crate Layout

Single binary crate, not a workspace. `Cargo.toml` sits at the repo root and contains exactly one `[[bin]]` target named `voicegate`. Module tree:

```
src/
  main.rs              clap CLI entry point -- subcommands call into lib
  lib.rs               pub mod re-exports + top-level VoiceGateError enum
  audio/               cpal + ringbuf + rubato
    mod.rs
    capture.rs         input stream, 32 ms frames, SPSC push
    output.rs          output stream, SPSC pop
    ring_buffer.rs     ringbuf::HeapRb<f32> SPSC wrapper
    resampler.rs       rubato::FftFixedIn (Phase 2+)
    virtual_mic.rs     VirtualMic trait + Linux PwCli + Windows VbCable
  config/              TOML Config
    mod.rs
    settings.rs        Config::load, Config::validate, Config::default
  ml/                  ort + ndarray (Phase 2+)
    mod.rs
    vad.rs             Silero VAD (stateful GRU)
    embedding.rs       ECAPA-TDNN (192-dim L2-normalized)
    similarity.rs      cosine_similarity + SpeakerVerifier with EMA
  enrollment/          (Phase 3+)
    mod.rs
    enroll.rs          EnrollmentSession + CLI subcommand
    profile.rs         VGPR magic + CRC32 profile format
  gate/                (Phase 4+)
    mod.rs
    gate.rs            AudioGate state machine (Open/Closing/Closed/Opening)
  pipeline/            (Phase 4+)
    mod.rs
    processor.rs       worker thread orchestration
  gui/                 (Phase 5+)
    mod.rs
    app.rs             eframe::App impl
    enrollment_wizard.rs
  app_controller.rs    (Phase 5+) mediator between GUI and pipeline/enrollment
```

Each directory has a `mod.rs` that re-exports the public surface of its children. `src/lib.rs` re-exports only what `main.rs` (and integration tests under `tests/`) needs. Private helpers stay private.

## Module Boundaries (enforced by code review, not tooling)

See `module-boundaries.md` for the full rules. Summary:

- `audio/` imports only `cpal`, `ringbuf`, `rubato`, `std`. No `ort`, no `eframe`.
- `ml/` imports only `ort`, `ndarray`, `std`. No `cpal`, no `eframe`.
- `config/`, `gate/`, `pipeline/`, `enrollment/`, `similarity` are pure-logic layers over `&[f32]` / `Vec<f32>` / typed structs.
- `gui/` imports `eframe`, `egui`, `rfd`. It does NOT import `cpal` or `ort` directly. It talks to `app_controller::AppController`, which owns the pipeline handle.
- `main.rs` is the only file allowed to wire the GUI-less paths (e.g. `enroll --wav`, `run --headless`) directly to their subsystems.
- Upward imports are forbidden. `audio/` must not `use crate::gui::...`, etc.

## Error Types

Top-level `VoiceGateError` enum in `src/lib.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum VoiceGateError {
    #[error("audio device error: {0}")]
    Audio(String),

    #[error("virtual microphone setup failed: {0}")]
    VirtualMic(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("ML inference error: {0}")]
    Ml(String),

    #[error("enrollment error: {0}")]
    Enrollment(String),

    #[error("gate state error: {0}")]
    Gate(String),
}
```

- Handler surface (anything called from `main.rs`) returns `anyhow::Result<T>`.
- Crate-internal boundaries (module public functions) return `Result<T, VoiceGateError>` where the variant indicates the domain.
- `anyhow` is for error composition in the application layer. `thiserror` is for domain errors that need stable variants for GUI error-string mapping.
- `thiserror` v2: use `{0}` for tuple variants and `{field_name}` for named fields. `#[from]` auto-generates `From` impls from wrapped source errors.
- Do NOT propagate `cpal::BuildStreamError` or `ort::Error` into `VoiceGateError` directly -- wrap in `.map_err(|e| VoiceGateError::Audio(e.to_string()))` at the call site to keep the error enum stable.

## No Unnecessary Async

- The audio pipeline is synchronous. `std::thread::spawn` owns the processing worker. No `tokio`, no `async-std`.
- `eframe` runs the UI on a dedicated thread; treat its internals as opaque.
- Never add `tokio` "just to have channels" -- use `std::sync::mpsc` or `crossbeam-channel` if a channel is needed.
- Ring buffers (`ringbuf::HeapRb<f32>` SPSC) are the primary audio-data channel between callbacks and the worker.

## State Sharing Between Threads

| Shared thing | Type | Who writes | Who reads |
|--------------|------|------------|-----------|
| Audio samples (input) | `ringbuf` SPSC (producer -> consumer) | cpal input callback | processing worker |
| Audio samples (output) | `ringbuf` SPSC (producer -> consumer) | processing worker | cpal output callback |
| Config (live-editable) | `Arc<RwLock<Config>>` | GUI thread | processing worker (read-only) |
| Similarity score | `Arc<AtomicU32>` (f32 via to_bits) | processing worker | GUI thread |
| Gate state LED | `Arc<AtomicU8>` | processing worker | GUI thread |
| VAD state LED | `Arc<AtomicU8>` | processing worker | GUI thread |
| Shutdown signal | `Arc<AtomicBool>` | main (Ctrl-C handler) | all threads |

- `Arc<Mutex<T>>` is allowed ONLY when there is no simpler primitive. Prefer `RwLock` for read-heavy config, atomics for scalar status values, and channels for events.
- `AtomicF32` does NOT exist in stable Rust. Use `AtomicU32` with `f32::to_bits` / `f32::from_bits`. Apply `Ordering::Relaxed` unless you specifically need cross-thread ordering.
- NEVER take a `Mutex::lock()` from inside a cpal callback. That is the single fastest way to cause an xrun.

## Real-Time Safety Checklist (audio callbacks)

Before landing any code inside a cpal callback or the audio processing worker's hot loop:

- [ ] No `Vec::new()`, `Vec::with_capacity()`, `Box::new()`, `Arc::new()`, or any other allocation
- [ ] No `Mutex::lock()`, `RwLock::write()`, `RwLock::read()`
- [ ] No `std::fs::*`, no network calls, no `std::process::Command`
- [ ] No `println!`, `eprintln!`, `tracing::info!`, `tracing::warn!`, `tracing::error!` (these touch global state / stdout locks)
- [ ] No `.unwrap()` on things that could fail at runtime. If it fails, drop the frame silently and record an atomic counter to surface later
- [ ] Buffers are pre-allocated outside the loop and reused. Stack-allocated `[f32; 1536]` is fine for a frame-scratch buffer
- [ ] Shared state access is via atomics or lock-free ring buffers ONLY

The worker thread (not a callback) MAY allocate between frames, but SHOULD still pre-allocate scratch buffers and reuse them. ML inference via `ort` allocates internally -- that is acceptable because the worker is not on the RT-priority callback thread.

## Cargo.toml Discipline

- The initial `Cargo.toml` was hand-written in Phase 1 to pin all dependencies in one place. Every subsequent change uses `cargo add <crate>` or `cargo add <crate>@<version>`, NEVER manual edits.
- Version pinning: use `"X.Y"` (not `"X.Y.Z"`, not `"^X.Y"`) unless there is a specific need to lock a patch version. This lets `cargo update` pick up patch releases while requiring explicit minor-version bumps.
- Feature flags are declared in `[features]` and documented in a comment above the flag. `pipewire-native` is off by default.
- Target-specific dependencies go in `[target.'cfg(target_os = "linux")'.dependencies]`. Gate the code with `#[cfg(target_os = "linux")]`.
- Do NOT introduce new crates without a Context7 query first. The dep graph is intentionally small (~20 crates); every addition is a judgment call about binary size, compile time, and platform support.

## Logging

- `tracing` + `tracing-subscriber` with `env-filter`. Initialize in `main.rs`: `tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env()).init()`.
- Default level: `info`. Crate-local: `voicegate=debug`.
- `RUST_LOG=voicegate=debug cargo run -- run --passthrough` for verbose local debugging.
- NEVER call any `tracing` macro from inside a cpal callback or the processing worker's hot loop. Log from the setup/teardown paths only, or use atomic counters and dump them from a timer task.

## CLI (clap derive)

`clap = { version = "4", features = ["derive"] }`. Everything uses `#[derive(Parser)]` / `#[derive(Subcommand)]` / `#[derive(Args)]`. The top-level `Cli` struct lives in `main.rs` and has no business logic -- it parses args, matches on the subcommand, and calls the relevant library function.

```rust
#[derive(Parser)]
#[command(name = "voicegate", version, about = "Real-time speaker isolation")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Devices,
    Run { #[arg(long)] passthrough: bool, /* ... */ },
    Enroll { /* ... */ },
    Doctor,  // Phase 6
}
```

## Common Pitfalls

- **Premature optimization inside the worker loop.** The worker runs at 31.25 Hz (32 ms frames). That is not a hot path for Rust -- ML inference dominates wall time. Prioritize readability; measure before optimizing.
- **`Box<dyn Trait>` for the virtual mic.** The `VirtualMic` trait is dyn-compat by design; `create_virtual_mic()` returns `Box<dyn VirtualMic>`. This keeps the platform split at the top of the module tree and out of every call site.
- **Forgetting to drop the cpal stream before teardown.** `cpal::Stream` stops its callback on drop. Teardown order in `main.rs` matters: drop streams first, then call `vmic.teardown()`, otherwise the virtual mic may be destroyed while the output callback is still writing to it.
- **Panic in a cpal callback.** Panics in the audio callback abort the process on some platforms and silently stop the stream on others. Every fallible call inside a callback must be handled via `if let Err(..)` / atomic counter, never `.unwrap()`.
