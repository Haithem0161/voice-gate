# Phase 5: GUI

**Goal:** Wrap the headless Phase 4 pipeline in an eframe/egui control panel so end users never have to touch a terminal. Ship a main screen (device pickers, similarity meter, sliders, bypass toggle, gate/VAD LEDs) and an enrollment wizard that drives Phase 3's enrollment path from a UI.

**Dependencies:** Phase 4 (full headless pipeline + `PipelineStatus` atomics + `Config` with all sections).
**Complexity:** M

---

## Section 1: Module & File Changes

### 1.1 Files to CREATE

```
src/gui/mod.rs              # pub mod app; pub mod enrollment_wizard;
src/gui/app.rs              # eframe::App impl with main screen
src/gui/enrollment_wizard.rs # Enrollment wizard screen (sub-state of the main app)
src/app_controller.rs       # AppController — mediates between GUI and audio threads
```

**No new tests.** GUI is verified manually. Phase 5 adds one smoke test (`test_app_controller_start_stop`) but otherwise relies on Phase 4's automated suite for correctness of the underlying pipeline.

### 1.2 Files to MODIFY

| Path | Change |
|------|--------|
| `src/main.rs` | `Run` without any flags (default) launches the GUI. `run --headless` and `run --passthrough` remain as escape hatches. |
| `src/lib.rs` | `pub mod gui; pub mod app_controller;`. Add `VoiceGateError::Gui(String)`. |
| `Cargo.toml` | `eframe` and `egui` were pinned in Phase 1 but not imported. Phase 5 imports them. |

---

## Section 2: Dependencies & Build Config

**New crate:**

```toml
rfd = "0.14"        # File dialogs (optional — only used in Phase 5 for the "choose profile path" dialog)
```

Added via `cargo add rfd`. Rationale: egui does not ship a native file-open dialog; `rfd` is the canonical rust-file-dialog crate, works on GTK (Linux) and Win32 (Windows).

**Existing crates used for the first time:** `eframe`, `egui` (both pinned in Phase 1).

**`eframe` feature flags:** default features are fine. We do **not** enable `wgpu` because the default GL backend works everywhere and we don't need GPU for a simple control panel. Document this choice in `.claude/rules/gui.md`.

---

## Section 3: Types, Traits & Public API

### 3.1 `src/app_controller.rs`

The `AppController` is the ONLY thing the GUI knows about. It never imports `cpal` or `ort` directly — that's enforced by `.claude/rules/module-boundaries.md`.

```rust
pub struct AppController {
    // Shared state (atomics and locks). Cloned into the GUI thread.
    pub status: Arc<PipelineStatus>,
    pub config: Arc<RwLock<Config>>,

    // Lifetime handles. None when not running.
    running: Option<RunningHandles>,
}

struct RunningHandles {
    shutdown: Arc<AtomicBool>,
    worker: JoinHandle<()>,
    _capture: CaptureStream,
    _output: OutputStream,
    vmic: Box<dyn VirtualMic>,
}

impl AppController {
    pub fn new(config: Config) -> Self;

    pub fn is_running(&self) -> bool;

    /// Start the headless pipeline with the currently loaded profile.
    pub fn start(&mut self, profile: Profile) -> anyhow::Result<()>;

    /// Stop the pipeline cleanly (tears down PipeWire nodes / VB-Cable detection).
    pub fn stop(&mut self) -> anyhow::Result<()>;

    /// Load the default profile, or return an error if not enrolled yet.
    pub fn load_default_profile(&self) -> anyhow::Result<Profile>;

    /// Run an enrollment session from the microphone for N seconds, save to default path.
    /// This is blocking and runs on a dedicated thread; the GUI calls it from a worker
    /// thread spawned by the enrollment wizard, not the egui update loop.
    pub fn enroll_from_mic(
        &self,
        seconds: u32,
        progress: Arc<AtomicU32>,  // seconds elapsed, 0..seconds
    ) -> anyhow::Result<Profile>;

    /// Wrap the Phase 4 config-to-pipeline construction.
    fn spawn_worker(
        config: &Config,
        profile: Profile,
        status: Arc<PipelineStatus>,
        input_consumer: AudioConsumer,
        output_producer: AudioProducer,
        shutdown: Arc<AtomicBool>,
    ) -> JoinHandle<()>;
}
```

**Invariants:**
- `AppController` is `Send` but not `Sync`. GUI thread holds the only reference.
- `start()` → `stop()` → `start()` must work cleanly (restartable for config changes).
- `stop()` is called on app exit via `eframe::App::on_exit`.

### 3.2 `src/gui/app.rs`

```rust
pub struct VoiceGateApp {
    controller: AppController,
    screen: Screen,

    // Cached atomics snapshots (refreshed each frame).
    latest_similarity: f32,
    latest_gate_state: GateState,
    latest_vad_active: bool,

    // Device lists (refreshed on demand).
    input_devices: Vec<String>,
    output_devices: Vec<String>,

    // Transient UI state.
    error_banner: Option<String>,
}

enum Screen {
    Main,
    EnrollmentWizard(EnrollmentWizardState),
}

impl eframe::App for VoiceGateApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame);
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>);
}

pub fn run() -> eframe::Result<()>;   // called from main.rs
```

**Main screen layout** (from PRD §5.10):

```
┌─────────────────────────────────────────────┐
│  VoiceGate                           [─][□][×]│
├─────────────────────────────────────────────┤
│  Status: [● Active | ○ Inactive]            │
│  Input:  [dropdown: device pickers]         │
│  Output: [dropdown: auto / specific name]   │
│                                             │
│  Similarity: [====●========]  0.82          │
│                                             │
│  Threshold:  [=======●====] 0.70            │
│  Hold time:  [===●========] 160 ms          │
│                                             │
│  [Re-enroll Voice] [Bypass: Off ▼]          │
│                                             │
│  Gate: ● OPEN   VAD: ● Speech               │
└─────────────────────────────────────────────┘
```

**Dropdowns / widgets:**
- `egui::ComboBox` for input device, output device, bypass mode.
- `egui::Slider::new(&mut threshold, 0.50..=0.95)` with step 0.01.
- `egui::Slider::new(&mut hold_ms, 40..=500).step_by(20.0)`.
- `egui::ProgressBar::new(similarity)` for the meter.
- Two small colored disks for Gate/VAD LEDs.

### 3.3 `src/gui/enrollment_wizard.rs`

```rust
pub struct EnrollmentWizardState {
    seconds_target: u32,         // default 30
    progress: Arc<AtomicU32>,    // written by the enrollment thread
    status: WizardStatus,
    passage: String,             // loaded from assets/enrollment_passages.txt
}

enum WizardStatus {
    ReadyToStart,
    Recording { started_at: Instant },
    Processing,
    Done(PathBuf),
    Failed(String),
}

impl EnrollmentWizardState {
    pub fn new() -> anyhow::Result<Self>;  // reads passage file
    pub fn render(&mut self, ui: &mut egui::Ui, controller: &mut AppController);
}
```

**Wizard screen** (from PRD §5.10 screen 2):

```
┌─────────────────────────────────────────────┐
│  Voice Enrollment                           │
├─────────────────────────────────────────────┤
│  Please read the following text aloud:      │
│                                             │
│  "The quick brown fox jumps over the lazy   │
│   dog. Pack my box with five dozen liquor   │
│   jugs. How vexingly quick daft zebras ..." │
│                                             │
│  Recording: [●●●●●●●○○○○○] 15 s / 30 s      │
│                                             │
│  [Cancel]             [Finish Early]        │
└─────────────────────────────────────────────┘
```

### 3.4 Error additions

```rust
#[error("GUI error: {0}")]
Gui(String),
```

---

## Section 4: Runtime Behavior

### 4.1 GUI update loop (per egui frame)

Called at ~60 Hz by eframe. Must not block, must not allocate large buffers.

1. Snapshot `self.controller.status` atomics into `self.latest_*` fields (cheap: 4 `.load(Relaxed)` calls).
2. `egui::CentralPanel::default().show(ctx, |ui| { ... })` — draws based on `self.screen`.
3. If any slider changed:
    - Acquire `self.controller.config.write()` (blocking; fast because writes are rare).
    - Update the affected field.
    - Release.
    - The running worker thread sees the new value on its next iteration via `config.read()`.
4. If the bypass dropdown changed:
    - Write `PipelineStatus::bypass_mode` atomic.
5. If the "Re-enroll Voice" button was clicked:
    - Transition `self.screen` to `Screen::EnrollmentWizard(EnrollmentWizardState::new()?)`.
6. Return.

### 4.2 Cross-thread repaint signaling

- egui auto-repaints on input events and after short intervals, but the similarity meter should update continuously while the pipeline runs.
- Solution: the worker thread calls `egui_ctx.request_repaint()` via a cloned `egui::Context` held in the app controller. This is a no-op if there is no pending update.
- `egui::Context` is `Clone + Send + Sync`, designed for this.
- Hook: pass `ctx.clone()` to `AppController::start` when the worker is spawned. The worker calls `ctx.request_repaint()` at the end of each `process_frame` (cheap).

### 4.3 Enrollment wizard lifecycle

1. User clicks "Re-enroll Voice" on main screen. App controller's `stop()` is called if the pipeline is running.
2. `Screen::EnrollmentWizard(state)` is set.
3. `EnrollmentWizardState::new()` reads the passage file, sets `seconds_target = 30`, `status = ReadyToStart`.
4. User clicks "Start". The wizard:
    - Creates an `Arc<AtomicU32>` progress counter.
    - Spawns a background thread: `controller.enroll_from_mic(30, progress.clone())`.
    - Sets `status = Recording { started_at: Instant::now() }`.
5. Each frame, the wizard reads `progress.load(Relaxed)`, renders a progress bar `progress / seconds_target`.
6. When the background thread finishes:
    - On success: `status = Done(profile_path)`; show a "Enrollment saved. Close wizard." button.
    - On failure: `status = Failed(msg)`; show error + retry button.
7. On "Close", `self.screen = Screen::Main`, and the user presses "Start" on the main screen to re-launch the pipeline.

**Finish Early button:** sets a separate `Arc<AtomicBool>` that the enrollment worker checks; when set, the worker stops capturing and finalizes with whatever has been recorded so far. If fewer than `MIN_SEGMENTS` speech segments are present, the enrollment fails gracefully.

### 4.4 Device list refresh

- Device lists are populated at app startup by calling `cpal::default_host().input_devices()` / `output_devices()`.
- Refresh triggered manually by a small refresh icon next to each dropdown.
- NOT refreshed every frame — that would be expensive.

### 4.5 Config persistence

- On `eframe::App::on_exit`, call `self.controller.config.read().save()?`. Errors are logged but don't prevent exit.
- On app startup, `Config::load()` is called; if the config file doesn't exist, `Default` is used.

### 4.6 Error banners

- The `error_banner` field is set on any `AppController` call that returns `Err`.
- Rendered as a red box at the top of the main screen with a "Dismiss" button.
- Auto-dismisses after 10 seconds (tracked via an `Option<Instant>`).

---

## Section 5: Cross-Platform & Resource Handling

### 5.1 Window title, icon, default size

- Title: `"VoiceGate"`.
- Default size: `480 × 360` (roughly matches the PRD mockup).
- Icon: embed `assets/icon.png` via `include_bytes!`; use `egui::IconData` in `NativeOptions`.
- Phase 5 ships **without a custom icon**; a placeholder is acceptable. Phase 6 (or a cosmetic follow-up) adds a real one.

### 5.2 High-DPI scaling

- egui's default DPI handling is correct on both Windows and Linux (reads from the display).
- Test on a 4K display: text should remain legible without manual scaling.

### 5.3 Wayland vs X11 on Linux

- `eframe` uses `winit` which supports both. Default feature set is fine.
- If input latency is visibly worse on Wayland, document it but don't fix in v1.

### 5.4 GUI hangs during enrollment

- Because `enroll_from_mic` runs on a worker thread, the GUI remains responsive during enrollment.
- The cancel button works at any time: it flips the cancellation `AtomicBool` and the worker returns within one audio chunk (~20 ms).

### 5.5 RWLock contention on config

- The worker thread reads `config.read()` once per frame (32 ms). The GUI writes `config.write()` only on user interaction (rare).
- Lock hold times are microseconds. No measurable contention.
- If contention becomes a problem, migrate to `arc-swap` — but don't premature-optimize.

---

## Section 6: Verification

### Automated

1. **`test_app_controller_start_stop`** — Create an `AppController` with default config and a test profile (synthesized in-memory, not loaded from disk). Call `start()`, wait 100 ms, call `stop()`. Assert no panics, no leaked threads. Does NOT actually open an audio device (uses a mock `VirtualMic` that returns `"null"` and a capture stream that reads from an in-memory sine wave — add a `#[cfg(test)]` module with this mock).
2. `cargo clippy -- -D warnings` clean.
3. `cargo fmt --check` clean.
4. `cargo build --release` produces `target/release/voicegate`.

### Manual smoke tests

5. **First launch** — `cargo run --release` (no subcommand) opens the GUI. Main screen is shown. Status = Inactive because no profile exists. Clicking "Start" shows an error banner: "No profile found. Please enroll your voice first."
6. **Enrollment flow:**
    - Click "Re-enroll Voice" → wizard screen.
    - Passage is displayed.
    - Click "Start" → progress bar animates.
    - Read the passage aloud for 30 s.
    - Wizard shows "Enrollment saved: /home/haithem/.local/share/voicegate/profile.bin".
    - Click "Close" → back to main screen.
7. **Start pipeline** — Click "Start" on main screen. Status flips to Active. Similarity meter updates in real time while speaking.
8. **Threshold slider** — Move the threshold slider. The gate behavior changes immediately (verified by speaking at the margin and observing the gate LED).
9. **Hold time slider** — Move from 160 ms to 400 ms. Brief pauses during speech no longer close the gate.
10. **Bypass toggle:**
    - Set to "On (pass all)": everything passes through regardless of speaker.
    - Set to "Off (mute)": output is silent.
    - Set to "Normal": gating resumes.
    - Each transition is audible in <50 ms.
11. **Device picker** — Change input device to a different mic. Pipeline restarts cleanly.
12. **Gate/VAD LEDs** — While speaking: VAD LED green, Gate LED green (open). During silence: VAD LED off, Gate LED green (still in hold window), then eventually Gate LED gray (closed).
13. **Error handling** — Unplug the mic during active operation. An error banner appears within 1 second. Click "Dismiss".
14. **Exit** — Close the window. Config is saved. PipeWire nodes torn down (`pw-cli list-objects | grep voicegate` empty).
15. **Re-launch** — Config and profile are loaded from disk. Device pickers show the last-selected devices. Clicking "Start" resumes where we left off.

### Windows-specific manual tests

16. On Windows 11 with VB-Cable installed, all of 5–15 work identically. The output device dropdown defaults to `"CABLE Input (VB-Audio Virtual Cable)"`.
17. If VB-Cable is NOT installed, the app launches but "Start" shows an error banner with the install link. Clicking the link opens the browser.

### Acceptance thresholds

- Similarity meter lag during active speech: <100 ms.
- Threshold slider effect: live, no restart required.
- Bypass toggle: audible change <50 ms.
- No GUI freezes during enrollment (responsive cancel button).
- 30-minute GUI session: no visual glitches, no leaked memory.

---

## Section 6+: PRD Gap Additions

### 6.1 Device picker refresh widget detail (Pass 1, G-010, LOW)

**Gap:** Section 4.4 mentions "small refresh icon next to each dropdown" but doesn't specify the exact widget, click behavior, or error handling.

**Addition (clarification to §3.2 main screen layout and §4.4):**

Each of the two device dropdowns (Input, Output) is rendered as a horizontal `egui::Ui::horizontal` containing:

1. An `egui::ComboBox` populated from the cached `input_devices` / `output_devices` vec.
2. An `egui::Button::new("⟳").small()` immediately to the right.

Clicking the refresh button:

1. Calls `cpal::default_host().input_devices()` (or `output_devices()`).
2. Collects the device names into a fresh `Vec<String>`.
3. Replaces the cached vec.
4. If the currently-selected device name is no longer present, sets `error_banner = Some("Selected device '{name}' is no longer available. Please pick another.")` and clears the selection.

Refresh is **manual only** — never called from `update()` directly, because cpal enumeration can take 50–200 ms on some backends and would stutter the 60 Hz UI.

On app startup, each device list is populated exactly once in `VoiceGateApp::new()` via the same call. The GUI does not poll for device changes.
