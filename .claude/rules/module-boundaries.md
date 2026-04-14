---
paths:
  - "src/**/*.rs"
---

# Module Boundary Rules (VoiceGate)

This file is the architecture contract. Every Rust file under `src/` obeys it. Violations are caught in code review, not by tooling, so read this before adding any `use crate::...` line.

## The Layer Diagram

```
                           main.rs  (binary entry + clap)
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
           config/        app_controller     gui/   (uses app_controller only)
              │                │                │
              │                ▼                │
              │           pipeline/             │
              │                │                │
              │        ┌───────┼───────┐        │
              │        ▼       ▼       ▼        │
              │     audio/    ml/    gate/      │
              │        │       │       │        │
              │        │       │    enrollment/ │
              │        │       │                │
              └────────┴───────┴────────────────┘
                       (all depend on lib.rs error types)
                              │
                              ▼
                            lib.rs  (VoiceGateError + re-exports)
```

**Arrows mean "depends on" (imports from).** No arrow can point upward. No arrow can skip layers sideways except where shown.

## Layer Rules

### `lib.rs` — foundation

- Defines `VoiceGateError` and re-exports the public module surface.
- Imports from: `std`, `thiserror`, `serde`.
- Imported by: everything.

### `config/`

- Pure data + TOML serde. `Config`, `AudioConfig`, `VadConfig`, `VerificationConfig`, `GateConfig`, `EnrollmentConfig`, `GuiConfig`.
- Imports from: `std`, `serde`, `toml`, `dirs`, `lib.rs` (for `VoiceGateError`).
- Imported by: every other module that needs to read config.
- **May NOT** import from `audio/`, `ml/`, `gate/`, `enrollment/`, `pipeline/`, `gui/`, `app_controller`. Config is a pure data layer.

### `audio/`

- cpal + ringbuf + rubato + hound. `VirtualMic` trait and its platform impls. Capture / output streams. Frame ring buffers.
- Imports from: `cpal`, `ringbuf`, `rubato`, `hound`, `std`, `anyhow`, `lib.rs`, `config/` (for `AudioConfig`).
- Imported by: `pipeline/`, `app_controller`, `main.rs` (for `devices` subcommand).
- **May NOT** import from `ml/`, `gate/`, `enrollment/`, `pipeline/`, `gui/`, `app_controller`.

### `ml/`

- ort + ndarray. `SileroVad`, `EcapaTdnn`, `SpeakerVerifier`, similarity functions, L2 normalization.
- Imports from: `ort`, `ndarray`, `std`, `lib.rs`, `config/` (for `VerificationConfig`).
- Imported by: `pipeline/`, `enrollment/`, `app_controller`.
- **May NOT** import from `audio/`, `gate/`, `enrollment/`, `pipeline/`, `gui/`, `app_controller`.

### `gate/`

- Pure-logic state machine over `&[f32]` buffers. `AudioGate`, crossfade math, hysteresis.
- Imports from: `std`, `lib.rs`, `config/` (for `GateConfig`).
- Imported by: `pipeline/`.
- **May NOT** import from `audio/`, `ml/`, `enrollment/`, `gui/`, `app_controller`.

### `enrollment/`

- `EnrollmentSession`, `Profile`, profile.bin format. Builds on `ml/` for embeddings.
- Imports from: `std`, `lib.rs`, `config/` (for `EnrollmentConfig`), `ml/` (for embedding extraction).
- Imported by: `app_controller`, `main.rs` (for `enroll` subcommand).
- **May NOT** import from `audio/`, `gate/`, `pipeline/`, `gui/`.

### `pipeline/`

- Orchestrates the processing worker thread: audio → resample → VAD → embedding → verify → gate → audio out.
- Imports from: `std`, `lib.rs`, `config/`, `audio/`, `ml/`, `gate/`.
- Imported by: `app_controller`.
- **May NOT** import from `enrollment/`, `gui/`, `app_controller`.
- Does NOT own the cpal streams directly — the pipeline asks `audio/` to start them and holds handles. This keeps the cpal dependency inside `audio/`.

### `app_controller.rs`

- Mediator between the GUI (or the headless CLI) and the pipeline / enrollment subsystems. Owns the pipeline handle and the enrollment handle. Exposes a small thread-safe API (see `gui.md`).
- Imports from: `std`, `lib.rs`, `config/`, `pipeline/`, `enrollment/`, `audio/` (for device enumeration and virtual mic setup).
- Imported by: `gui/`, `main.rs`.
- **May NOT** import from `gui/`. This is the one directional rule that makes the GUI an interchangeable frontend: today it is eframe, tomorrow it could be a web view, and `AppController` does not care.

### `gui/`

- eframe + egui + rfd. `VoiceGateApp`, main screen, enrollment wizard.
- Imports from: `eframe`, `egui`, `rfd`, `std`, `lib.rs`, `config/`, `app_controller`.
- Imported by: `main.rs` (for the `run` default subcommand).
- **May NOT** import from `cpal`, `ort`, `ringbuf`, `rubato`, `hound`, `audio/`, `ml/`, `gate/`, `enrollment/`, `pipeline/`. If the GUI needs something from those modules, it goes through `AppController` or through a pure data type re-exported from `lib.rs`.

### `main.rs`

- clap CLI + eframe launch. Can import anything because it wires the app together.
- Imports from: everything.
- Imported by: nothing (it is the binary entry).

## Why These Rules Exist

1. **Test isolation.** Pure-logic modules (`gate/`, `enrollment/`, `ml/similarity.rs`) can be unit-tested without cpal or ort. Anything under `audio/` or `ml/` can be tested without a GUI.
2. **Swap the frontend.** If v2 adds a web UI, only `gui/` changes. `AppController` is the contract.
3. **Context7 discipline.** Each boundary has a small, explicit set of external crates. That makes "query Context7 before editing this module" a concrete checklist, not a vague aspiration.
4. **Real-time safety.** `audio/` callbacks cannot accidentally `use crate::gui::...` and pull in allocating egui code. The boundary is enforced at module-definition time.
5. **Build time + binary size.** Modules that do not depend on `ort` compile in seconds instead of minutes. Tests in `tests/test_gate.rs` or `tests/test_profile.rs` link a much smaller dep graph.

## Enforcement

No automated tool checks these rules today. Enforcement is:

1. **Code review.** Every PR's diff is inspected for new `use crate::...` lines. A line like `use crate::gui::app::VoiceGateApp;` inside `src/audio/capture.rs` is an immediate reject.
2. **Grep sweep.** Before a phase is marked complete:
   ```bash
   grep -rn "use crate::gui" src/audio src/ml src/gate src/enrollment src/pipeline src/config
   # -> must be empty
   grep -rn "use cpal\|use ort" src/gui
   # -> must be empty
   grep -rn "use crate::app_controller" src/audio src/ml src/gate src/enrollment src/pipeline src/config
   # -> must be empty
   ```
3. **Tests.** Integration tests that target `src/gate/` or `src/ml/similarity.rs` import only from those modules. If the test file needs `cpal` or `eframe` to compile, the boundary has been violated somewhere upstream.

## Acceptable Cross-Module Types

These types are imported freely across module boundaries because they are pure data and have no behavioral coupling:

- `VoiceGateError` (lib.rs)
- `Config` and its sub-structs (config/)
- `Profile` (enrollment/profile.rs) — but only pure data access, not `EnrollmentSession`
- `StatusSnapshot` (pipeline/) — a `Copy` struct of atomics-snapshotted values
- Primitive `&[f32]` / `Vec<f32>` / `Box<[f32]>` slices of audio samples

Anything trait-object or closure-based crosses at `AppController` only.

## Pitfalls

- **`gui/` needing a device list.** Do not `use cpal::*` in gui. Add an `AppController::list_devices() -> DeviceList` method that internally calls into `audio/` and returns a pure data `DeviceList`.
- **`pipeline/` needing to signal the UI.** Do not `use egui::*` in pipeline. Update an atomic and call `egui_ctx.request_repaint()` via a stored `Arc<egui::Context>` that was handed in at pipeline-start time.
- **`audio/` needing config.** That is fine — it imports `config::AudioConfig`. Config is the one pure-data layer every other module may depend on.
- **A helper needed by both `enrollment/` and `pipeline/`.** Put it in `ml/` if it is ML-related, in `config/` if it is config-related, or in a new pure-logic module if it is something else. Do NOT put it in `pipeline/` and then have `enrollment/` import from `pipeline/` — that creates an upward dependency.
