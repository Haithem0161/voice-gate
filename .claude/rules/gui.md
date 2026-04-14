---
paths:
  - "src/gui/**"
  - "src/app_controller.rs"
---

# GUI Rules (eframe + egui)

This file is the contract for `src/gui/` and `src/app_controller.rs`. Everything in this module runs on the eframe-owned main/UI thread. The GUI never touches cpal or ort directly.

## Lifecycle

- `main.rs` calls `eframe::run_native("VoiceGate", NativeOptions::default(), Box::new(|cc| Box::new(VoiceGateApp::new(cc, controller))))` for the default `run` subcommand.
- `VoiceGateApp` implements `eframe::App`. Its `update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame)` method is called every UI frame (vsync-paced, typically 60 Hz).
- `VoiceGateApp` owns:
  - `AppController` — the mediator that talks to the pipeline and enrollment handles
  - A copy of the config snapshot (cloned from `Arc<RwLock<Config>>` once per UI frame)
  - Local UI state (which screen is active, slider drag-in-progress flags, error toasts)
- `VoiceGateApp` does NOT own the cpal streams or the ort sessions. Those live inside the pipeline owned by `AppController`.

## AppController (the mediator)

`AppController` is in `src/app_controller.rs` at the top of the module tree so both `main.rs` and `src/gui/` can reach it. It owns the long-running audio/ML pipeline and exposes a small, thread-safe API:

```rust
pub struct AppController {
    config: Arc<RwLock<Config>>,
    pipeline_handle: Option<PipelineHandle>,     // None when gated off
    enrollment_handle: Option<EnrollmentHandle>, // None when not enrolling
    status: PipelineStatus,                      // atomics shared with the worker
    profile_path: PathBuf,
}

impl AppController {
    pub fn start_pipeline(&mut self, profile: Profile) -> Result<(), VoiceGateError>;
    pub fn stop_pipeline(&mut self);
    pub fn start_enrollment(&mut self, mode: EnrollmentMode) -> Result<(), VoiceGateError>;
    pub fn finish_enrollment(&mut self) -> Result<Profile, VoiceGateError>;
    pub fn cancel_enrollment(&mut self);
    pub fn set_threshold(&self, threshold: f32);  // writes through Arc<RwLock<Config>>
    pub fn set_hold_frames(&self, hold: u32);
    pub fn toggle_bypass(&self);
    pub fn status_snapshot(&self) -> StatusSnapshot;  // reads atomics
}
```

- The GUI NEVER imports `cpal::*` or `ort::*`. Only `AppController` does.
- All cross-thread communication uses atomics (for scalar status) or channels (for events like "enrollment finished").
- `status_snapshot()` reads `Arc<AtomicU32>` / `Arc<AtomicU8>` every UI frame. These are cheap (relaxed atomic loads).

## Update Loop Rules

Inside `eframe::App::update`:

- **Do NOT block.** No `thread::sleep`, no `channel.recv()` without timeout, no `Mutex::lock()` on contended locks. The update must return within one vsync frame (~16 ms) or the UI will stutter.
- **Do NOT allocate gratuitously.** Building a `Vec<String>` on every frame for a 100-item list is fine; cloning the entire config on every frame is wasteful. Snapshot once at the top of `update` and reuse.
- **Do NOT spawn threads from `update`.** Long-running work belongs in the pipeline, which is started once via `AppController::start_pipeline`.
- **Request repaint only when needed.** `ctx.request_repaint()` is free to call but `ctx.request_repaint_after(Duration::from_millis(50))` is preferred for status meters — it caps UI refresh at 20 Hz when audio is running, saving CPU.

```rust
impl eframe::App for VoiceGateApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let status = self.controller.status_snapshot();
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_main_screen(ui, &status);
        });
        // Keep the similarity meter live at 20 Hz without pegging CPU
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}
```

## Cross-Thread Repaint from the Worker

When the processing worker wants to notify the UI of a state change (e.g. gate flipped from Closed to Open), it calls `egui::Context::request_repaint()`. This is thread-safe — the only egui API that is safe to call from an arbitrary thread.

To give the worker access to the context:

1. During `VoiceGateApp::new`, store `cc.egui_ctx.clone()` in `AppController`.
2. `AppController` hands that clone to the pipeline when it starts.
3. The worker calls `egui_ctx.request_repaint()` after updating any atomic the UI reads.

Never call any other egui API from the worker. Building widgets, reading `egui::Context` state, or touching `ui: &mut egui::Ui` from the worker is undefined behavior.

## Widgets (Main Screen)

Per PRD §5.7. Layout from top to bottom:

1. **Status indicator**: "Listening" / "Speaker verified" / "Gated" / "Bypassed". Big, colored text.
2. **Device pickers**: input and output `ComboBox`es populated from `cpal::Host::{input,output}_devices()`. Display the device name; value is the same string.
3. **Refresh button**: `ui.small_button("↻")` next to each ComboBox. (Uses a text character, not an emoji. A geometric arrow is acceptable; a colored emoji is not.) Clicking re-enumerates cpal devices. Manual only — never auto-refresh, cpal enumeration takes 50-200 ms. This is Pass 1 gap G-010.
4. **Similarity meter**: horizontal bar showing `status.similarity` (0.0 to 1.0), tinted green above threshold, red below.
5. **Threshold slider**: `egui::Slider::new(&mut threshold, 0.5..=0.95).text("Threshold")`. On change, calls `controller.set_threshold(threshold)` (debounced: only write when the user releases the slider).
6. **Hold-time slider**: `0..=20` frames. Same pattern.
7. **Bypass toggle**: `ui.checkbox(&mut bypass, "Bypass (pass audio through unchanged)")`. Takes effect in <50 ms via an atomic on the pipeline.
8. **Gate LED**: green / red square reflecting `PipelineStatus::gate_state`.
9. **VAD LED**: green / grey square reflecting `PipelineStatus::vad_active`.
10. **Enrollment button**: opens the enrollment wizard screen.

## Enrollment Wizard

Per PRD §5.6. This is a separate screen (not a modal) because it needs to live for 30+ seconds and the user should still see the main status meters.

State machine:

```
Welcome → Countdown → Recording → Finish
    ↑                                ↓
    └── Cancel ──────────────────────┘
```

- **Welcome**: shows the passage from `assets/enrollment_passages.txt`. A "Start" button transitions to Countdown.
- **Countdown**: 3-second visual countdown before recording starts. Gives the user time to get ready.
- **Recording**: shows a waveform (optional, v1.1), elapsed time, estimated time remaining, VAD-active indicator. A "Finish early" button is enabled after 15 seconds of VAD-active time.
- **Finish**: saves the profile, shows "Saved to ~/.local/share/voicegate/profile.bin", offers "Start gating" or "Re-record".

State transitions are driven by events from `AppController::enrollment_rx` (a channel). The wizard NEVER polls `enrollment_handle` directly from `update`.

## File Dialogs (rfd)

`rfd = "0.14"` (add via `cargo add rfd` in Phase 5; not in the Phase 1 Cargo.toml). Used for:

- "Load profile..." to pick a `profile.bin` from anywhere on disk (optional, v1.1).
- "Save enrollment as..." to save a profile to a custom location.

```rust
if ui.button("Load profile...").clicked() {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("VoiceGate Profile", &["bin"])
        .pick_file()
    {
        self.controller.load_profile(path);
    }
}
```

`rfd::FileDialog::pick_file()` is blocking on the main thread. That is acceptable for a file picker because the user expects the UI to freeze briefly while picking. For longer operations (actual profile load), spawn a thread via `AppController`.

## Error Display

- Errors from `AppController` methods that return `Result` are stored in `self.last_error: Option<String>` and displayed as a dismissible `egui::Window` titled "Error".
- Never display a raw `VoiceGateError::Debug` stringification to the user. Use the `Display` impl which is set up via `thiserror`'s `#[error("...")]` annotations to be user-friendly.
- Common errors have dedicated recovery actions:
  - "CABLE Input not found" → show an "Install VB-Cable" button that opens https://vb-audio.com/Cable/ via `open::that`
  - "pw-cli not found" → show a "PipeWire required" dialog with Linux install instructions

## Pitfalls

- **Blocking `update` on a channel receive**. Use `try_recv()` + state persistence. The next `update` will check again.
- **Holding a `RwLock` write guard across a UI operation**. Take the write lock, apply the change, drop the guard, then render.
- **Spawning cpal streams from the UI thread**. That is `AppController::start_pipeline`'s job. `update` only calls `start_pipeline` once in response to a button click.
- **Importing `cpal` or `ort` in any file under `src/gui/`**. This violates module boundaries (see `module-boundaries.md`). The GUI knows only about `AppController`, `Config`, `Profile`, `StatusSnapshot`, and pure data types.
- **Rendering on every state tick of the worker**. The worker runs at 31.25 Hz (every 32 ms). If it called `request_repaint()` on every frame, the UI would render at 31 Hz minimum even when idle. Gate the repaint: only call it when a status value actually changes, or use `request_repaint_after(50ms)` from `update` itself.
- **egui window state lost on config reload**. If the user reloads config, do not recreate `VoiceGateApp` — update the in-place fields. `eframe` is designed for single long-lived app instances.

## Context7

Before writing eframe/egui code:

1. `resolve-library-id "eframe"` and `"egui"`.
2. `query-docs` for the specific widget you need. `egui::Slider::new`, `egui::ComboBox::from_label`, `egui::Grid::new`, `egui::CollapsingHeader::new`, etc. The API has evolved across 0.26, 0.27, 0.28, 0.29, 0.30.
3. The "immediate mode" pattern means widget state is passed in and out by reference every frame. Do not cache widget return values outside `update`.
