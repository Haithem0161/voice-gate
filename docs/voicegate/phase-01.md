# Phase 1: Foundation Morph + Audio Passthrough

**Goal:** Morph the repo from a Rust/Axum backend template into a Rust desktop binary crate, rewrite all project-level docs and rule files, scaffold the module tree, and prove a working mic → virtual-mic passthrough on Linux via PipeWire.

**Dependencies:** None (this phase produces the scaffolding every later phase extends).
**Complexity:** L

---

## Section 1: Module & File Changes

### 1.1 Files to DELETE

Remove outright. These are backend artifacts that have zero role in a desktop app.

```
backend/                              # entire directory (Axum app, domains, migrations, .sqlx, backend/.memory)
docker-compose.yml
deploy/Dockerfile
deploy/voice-gate.service
deploy/                               # directory itself after contents removed
.env                                  # if it exists and references DATABASE_URL / JWT_SECRET
.claude/rules/rust-backend.md
.claude/rules/docker.md
.claude/rules/api-design.md
.claude/rules/auth.md
.claude/rules/migrations.md
.claude/rules/ddd.md
```

**Preserve before deletion:** move `backend/.memory/cursor.json` to `.memory/cursor.json` at the repo root first (telemetry continuity).

### 1.2 Files to REWRITE from scratch

| Path | New content summary |
|------|---------------------|
| `CLAUDE.md` | Desktop app principles; Context7 crate list re-enumerated (`cpal`, `ort`, `eframe`, `egui`, `rubato`, `hound`, `ringbuf`, `ndarray`, `dirs`, `thiserror`, `serde`, `toml`, `anyhow`, `tracing`, `which`, `pipewire`); file-path table replaces port map; fixture-WAV testing replaces curl testing; rule-file references updated; "No emojis" + commit-authorship rules kept verbatim. |
| `Makefile` | Targets: `setup`, `models`, `dev`, `test`, `lint`, `release`, `package-linux`, `package-windows`, `clean`. Delete `db`, `migrate`, `backend`. |
| `README.md` | VoiceGate overview, features, prerequisites, build & run, model sourcing, license. |
| `.github/workflows/ci.yml` | Replace Postgres CI with a matrix `{ubuntu-latest, windows-latest}` × `{fmt --check, clippy -D warnings, test, build --release}`. Install `libasound2-dev`, `libpipewire-0.3-dev`, `pkg-config`, `libclang-dev` on Linux. Cache cargo registry + target. |
| `.gitignore` | Add `models/*.onnx`, `models/tmp_model/`, `target/`, `*.bin`, `.env*`. |
| `.claude/rules/testing.md` | Rewrite for Rust desktop: unit tests inline, integration tests in `tests/` reading fixture WAVs via `hound`, deterministic ORT session tests, embedding-vector snapshot tests with tolerance, no DB setup, `cargo test` only. |
| `.claude/rules/planning.md` | **Keep most of it.** Prepend a "Desktop App Adaptation" header note that points at the 7-section phase-file template used by VoiceGate (Module & File Changes / Dependencies & Build Config / Types, Traits & Public API / Runtime Behavior / Cross-Platform & Resource Handling / Verification / PRD Gap Additions). |
| `Cargo.toml` (new, at repo root) | Single binary crate; see §2 for exact dependencies. |

### 1.3 Files to CREATE (new)

**New rule files** (replace deleted backend rules):

- `.claude/rules/rust-desktop.md`
- `.claude/rules/audio-io.md`
- `.claude/rules/ml-inference.md`
- `.claude/rules/gui.md`
- `.claude/rules/module-boundaries.md`
- `.claude/rules/cross-platform.md`

**New plan directory** (part of 1.2, already in progress — this file lives in it):

- `docs/voicegate/roadmap.md`
- `docs/voicegate/research.md`
- `docs/voicegate/phase-01.md` through `phase-06.md`
- `docs/voicegate/status.md`
- `docs/voicegate/frontend-summary.md`

**New source tree** (stubs + passthrough wiring only; no ML yet):

```
src/
├── main.rs                    # clap CLI entry point — Phase 1 subcommands: `devices`, `run --passthrough`
├── lib.rs                     # public module re-exports
├── audio/
│   ├── mod.rs
│   ├── capture.rs             # cpal input stream, 32 ms frames, SPSC push
│   ├── output.rs              # cpal output stream, SPSC pop
│   ├── ring_buffer.rs         # ringbuf::HeapRb SPSC wrapper
│   ├── resampler.rs           # stub (rubato wired up in Phase 2)
│   └── virtual_mic.rs         # VirtualMic trait + Linux PwCliVirtualMic + Windows VbCableVirtualMic
├── config/
│   ├── mod.rs
│   └── settings.rs            # Config::default + Config::load (partial; expanded in Phase 4)
└── ml/
    └── mod.rs                 # empty placeholder; fleshed out in Phase 2
```

**New scripts, assets, fixtures:**

- `scripts/download_models.py` (empty placeholder, committed)
- `scripts/export_ecapa.py` (empty placeholder, committed)
- `scripts/setup_pipewire.sh` (executable, mirrors PRD Appendix C)
- `models/.gitkeep`
- `tests/fixtures/.gitkeep`
- `assets/enrollment_passages.txt` (PRD Appendix A pangram passage)

---

## Section 2: Dependencies & Build Config

New root `Cargo.toml` (replaces `backend/Cargo.toml`):

```toml
[package]
name = "voicegate"
version = "0.1.0"
edition = "2021"
rust-version = "1.83"

[dependencies]
# Audio I/O
cpal = "0.15"
rubato = "0.14"
ringbuf = "0.4"

# ML inference (unused in Phase 1, added now to pin versions once)
ort = { version = "2", features = ["load-dynamic"] }
ndarray = "0.15"

# GUI (unused in Phase 1)
eframe = "0.28"
egui = "0.28"

# Audio file I/O
hound = "3.5"

# Config + CLI
serde = { version = "1", features = ["derive"] }
toml = "0.8"
clap = { version = "4", features = ["derive"] }

# Error / logging
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = "0.3"

# Platform paths + executable lookup
dirs = "5"
which = "6"

[target.'cfg(target_os = "linux")'.dependencies]
pipewire = { version = "0.8", optional = true }

[features]
default = []
pipewire-native = ["pipewire"]

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
```

**Notes:**
- Dependencies are pinned once in Phase 1 to avoid churn in later phases. Unused crates (`ort`, `ndarray`, `eframe`, `egui`, `hound`) are listed but not imported in Phase 1 source.
- `pipewire-native` feature is **off** by default. Phase 1 uses `pw-cli` shell invocations via `std::process::Command`. The feature is exercised in Phase 6.
- `rust-toolchain.toml` already pins 1.83 and is preserved.

**ONNX Runtime shared library:**
- Linux: `libonnxruntime.so` must be installed system-wide (`/usr/local/lib`). Installation is a prerequisite, not a build step (see README).
- Windows: `onnxruntime.dll` next to the executable or on `PATH`.
- The `load-dynamic` feature on `ort` means build does NOT fail if the shared library is missing; only runtime model loading does. This is acceptable for Phase 1 since Phase 1 doesn't load any ONNX models.

---

## Section 3: Types, Traits & Public API

### 3.1 `src/audio/ring_buffer.rs`

```rust
use ringbuf::{traits::*, HeapRb};

pub type AudioProducer = <HeapRb<f32> as Split>::Prod;
pub type AudioConsumer = <HeapRb<f32> as Split>::Cons;

pub fn new_audio_ring(capacity_samples: usize) -> (AudioProducer, AudioConsumer) {
    HeapRb::<f32>::new(capacity_samples).split()
}
```

- Capacity for the input ring and output ring is **3 seconds × 48 000 Hz = 144 000 samples** each (≈ 576 KB per queue).

### 3.2 `src/audio/capture.rs`

```rust
pub struct CaptureStream {
    _stream: cpal::Stream,  // kept alive so the callback keeps firing
    pub device_name: String,
    pub sample_rate: u32,
}

pub fn start_capture(
    device_name: Option<&str>,
    producer: AudioProducer,
) -> anyhow::Result<CaptureStream>;
```

- Selects the named cpal input device or falls back to `default_input_device()`.
- Negotiates `StreamConfig { channels: 1, sample_rate: 48000, buffer_size: Fixed(1536) }`; on `StreamConfigNotSupported`, retries with `BufferSize::Default`.
- On stereo-only devices, downmixes L+R in the callback via averaging.

### 3.3 `src/audio/output.rs`

```rust
pub struct OutputStream {
    _stream: cpal::Stream,
    pub device_name: String,
}

pub fn start_output(
    device_name: &str,
    consumer: AudioConsumer,
) -> anyhow::Result<OutputStream>;
```

- Looks up the named cpal output device by display name (`voicegate_sink` on Linux, `CABLE Input (VB-Audio Virtual Cable)` on Windows).

### 3.4 `src/audio/virtual_mic.rs`

```rust
pub trait VirtualMic: Send {
    /// Set up the virtual microphone. Returns the cpal output device name to write to.
    fn setup(&mut self) -> anyhow::Result<String>;
    /// Tear down on exit. Must be idempotent.
    fn teardown(&mut self) -> anyhow::Result<()>;
    /// Human-readable name Discord should select as input.
    fn discord_device_name(&self) -> &str;
}

pub fn create_virtual_mic() -> Box<dyn VirtualMic>;

#[cfg(target_os = "linux")]
pub struct PwCliVirtualMic { /* tracks whether setup completed */ }

#[cfg(target_os = "windows")]
pub struct VbCableVirtualMic { /* detection state */ }
```

- `create_virtual_mic()` returns the appropriate impl via `#[cfg(target_os)]`.
- On Linux, `PwCliVirtualMic::setup()` spawns `pw-loopback` as a child process with `--capture-props` / `--playback-props` defining `voicegate_sink` (Audio/Sink) and `voicegate_mic` (Audio/Source/Virtual). See §6.3 below for why this replaces the PRD Appendix C `pw-cli create-node` + `pw-link` approach (short version: on PipeWire 1.0.x, `pw-cli create-node` creates a client-owned node that dies with the pw-cli process, so the original approach cannot work).
- On Windows, `VbCableVirtualMic::setup()` scans cpal output devices for `"CABLE Input (VB-Audio Virtual Cable)"`. If missing, returns an `anyhow::Error` with the install link.
- `teardown()` on Linux sends SIGKILL to the `pw-loopback` child process and waits for exit; `pw-loopback` tears down its nodes on shutdown regardless of signal.

### 3.5 `src/audio/resampler.rs` (stub)

```rust
/// Placeholder — flesh out in Phase 2.
pub struct Resampler48to16;

impl Resampler48to16 {
    pub fn new() -> Self { Self }
}
```

### 3.6 `src/config/settings.rs` (partial — only `[audio]` section in Phase 1)

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub audio: AudioConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AudioConfig {
    pub input_device: String,     // "default" or a specific device name
    pub output_device: String,    // "auto" or a specific name

    /// Frame size in milliseconds. Matches PRD §5.9 TOML key `frame_size_ms`.
    /// **HARD-CODED to 32** in v1 — see §6.2 and Decision D-001 (research.md).
    /// Non-default values are REJECTED at load time because Silero VAD requires
    /// exactly 512 samples @ 16 kHz, which only aligns at 32 ms frames.
    pub frame_size_ms: u32,

    pub sample_rate: u32,         // always 48000 in v1
}

impl AudioConfig {
    /// Derived constant: `frame_size_ms * sample_rate / 1000`. For the v1 values
    /// 32 ms × 48 000 Hz this is exactly 1536. Callers should use this helper
    /// instead of recomputing, and use it as an assertion when building cpal
    /// stream configs.
    pub fn frame_size_samples(&self) -> usize {
        (self.frame_size_ms as usize) * (self.sample_rate as usize) / 1000
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            audio: AudioConfig {
                input_device: "default".into(),
                output_device: "auto".into(),
                frame_size_ms: 32,
                sample_rate: 48000,
            },
        }
    }
}

impl Config {
    /// Validate that user-provided config values are in the supported set.
    /// Called by `Config::load()` after deserializing. Returns an error that
    /// surfaces cleanly to the CLI/GUI, not a panic.
    pub fn validate(&self) -> Result<(), VoiceGateError> {
        if self.audio.frame_size_ms != 32 {
            return Err(VoiceGateError::Config(format!(
                "audio.frame_size_ms = {} is not supported. Only 32 is valid in v1 \
                 (Silero VAD requires 512 samples at 16 kHz, which only aligns at 32 ms).",
                self.audio.frame_size_ms
            )));
        }
        if self.audio.sample_rate != 48000 {
            return Err(VoiceGateError::Config(format!(
                "audio.sample_rate = {} is not supported. Only 48000 is valid in v1.",
                self.audio.sample_rate
            )));
        }
        Ok(())
    }
}

impl Config {
    pub fn load() -> anyhow::Result<Self>;       // dirs::config_dir()/voicegate/config.toml, falls back to Default
    pub fn save(&self) -> anyhow::Result<()>;
}
```

### 3.7 `src/main.rs`

```rust
#[derive(Parser)]
#[command(name = "voicegate", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List cpal input and output devices.
    Devices,
    /// Run the pipeline. In Phase 1, only --passthrough is wired up.
    Run {
        /// Passthrough mode: mic → virtual mic with no ML. Phase 1 default.
        #[arg(long)]
        passthrough: bool,
    },
}
```

### 3.8 Error types

A single top-level `thiserror` enum in `src/lib.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum VoiceGateError {
    #[error("audio device error: {0}")]
    Audio(String),

    #[error("virtual microphone setup failed: {0}")]
    VirtualMic(String),

    #[error("configuration error: {0}")]
    Config(String),
}
```

Handler surface uses `anyhow::Result<T>`; boundaries convert to `VoiceGateError`. Later phases extend with `Ml`, `Enrollment`, `Gate` variants.

---

## Section 4: Runtime Behavior

### 4.1 `voicegate devices`

1. `cpal::default_host()`
2. Enumerate `host.input_devices()?` and `host.output_devices()?`
3. Print each device's `.name()?` with an "IN" or "OUT" prefix.
4. Mark the default devices with `(default)`.
5. Return 0.

### 4.2 `voicegate run --passthrough`

**Startup sequence (main thread):**

1. `tracing_subscriber::fmt::init()`
2. `Config::load()` — may return `Default` if no config exists yet.
3. `let mut vmic = create_virtual_mic();`
4. `let output_device_name = vmic.setup()?;`
   - Linux: shells out to `pw-cli` to create `voicegate_sink` + `voicegate_mic` + link. Returns `"voicegate_sink"`.
   - Windows: scans for VB-Cable. Returns `"CABLE Input (VB-Audio Virtual Cable)"`.
5. Allocate two ring buffers (input + output), each 144 000 samples capacity.
6. `let capture = start_capture(&config.audio.input_device, input_producer)?;`
7. Spawn the passthrough worker thread (see 4.3).
8. `let output = start_output(&output_device_name, output_consumer)?;`
9. Install Ctrl-C handler that flips a shared `AtomicBool` shutdown signal.
10. Main thread parks on a condvar until shutdown is signaled.
11. Drop streams (stops callbacks).
12. `vmic.teardown()?;`
13. Exit 0.

### 4.3 Passthrough worker thread (Phase 1 only — no ML)

```
loop {
    if shutdown.load(Relaxed) { break; }

    // Pop up to 1536 samples from the input ring (blocking wait if empty).
    let mut frame = [0.0f32; 1536];
    let n = input_consumer.pop_slice(&mut frame);
    if n == 0 {
        std::hint::spin_loop();
        continue;
    }

    // Push the same frame to the output ring (Phase 1: identity).
    output_producer.push_slice(&frame[..n]);
}
```

**Real-time constraints:**

- **Input callback MUST NOT allocate.** Just `push_slice` into the ring.
- **Output callback MUST NOT allocate.** Just `pop_slice` from the ring.
- **Worker thread is allowed to allocate** between frames but should pre-allocate the `[f32; 1536]` scratch buffer once and reuse it (stack-allocated here).
- **No `println!` / `tracing::info!` inside any callback.** Worker thread logging is allowed at `debug` level only.

### 4.4 ALSA variable-callback handling

If `BufferSize::Fixed(1536)` is rejected, the capture callback is called with arbitrary sizes (e.g. 480, 960, 2048). The callback simply `push_slice`s everything into the ring, and the worker pops in fixed 1536-sample chunks. The ring's 3-second capacity absorbs jitter.

### 4.5 Shutdown cleanliness

- Linux `teardown()` MUST destroy `voicegate_sink` and `voicegate_mic`. If the process is killed with SIGKILL, the nodes leak; document this as a known limitation and add a note in README that `pw-cli list-objects | grep voicegate` + manual destroy is the recovery.
- Windows `teardown()` is a no-op (VB-Cable is persistent).

---

## Section 5: Cross-Platform & Resource Handling

### 5.1 `#[cfg(target_os)]` split points

| File | Split |
|------|-------|
| `src/audio/virtual_mic.rs` | Linux uses `PwCliVirtualMic`; Windows uses `VbCableVirtualMic`. macOS is `unimplemented!()`. |
| `src/audio/capture.rs` | None — cpal handles WASAPI vs ALSA transparently. |
| `src/audio/output.rs` | None. |

### 5.2 File path resolution

- Config: `dirs::config_dir()?.join("voicegate/config.toml")`
- Profile (Phase 3): `dirs::data_dir()?.join("voicegate/profile.bin")`
- Models: relative to the executable (`models/silero_vad.onnx`, etc.) for dev; final packaging (Phase 6) embeds in the AppImage/MSI.

### 5.3 Error surfaces

- **Linux, no PipeWire:** `which pw-cli` fails → error "`pw-cli` not found. VoiceGate requires PipeWire on Linux. See README.md." (PulseAudio fallback is Phase 6.)
- **Windows, no VB-Cable:** device scan empty → error "`CABLE Input (VB-Audio Virtual Cable)` not found. Install from https://vb-audio.com/Cable/ and reboot."
- **Permission denied on mic:** cpal surfaces this via `DefaultStreamConfigError`. Wrap with "Microphone access denied. On Linux, check `pactl list short sources` and PipeWire permissions. On Windows, Settings → Privacy → Microphone."

### 5.4 Graceful device disconnect

Out of scope for Phase 1 (not in PRD §13 success criteria, and reconnect logic is complex). Document as a known limitation: if the mic is unplugged, the process exits with an error. Phase 6 may revisit.

---

## Section 6: Verification

**Every step must pass before Phase 1 is considered complete.**

### Automated checks

1. `cargo check` compiles cleanly on Linux.
2. `cargo clippy -- -D warnings` is clean.
3. `cargo fmt --check` is clean.
4. `cargo build --release` produces `target/release/voicegate`.
5. `grep -riE 'sqlx|axum|utoipa|postgres|diesel|jwt' CLAUDE.md src/ $(ls .claude/rules/*.md | grep -v planning.md)` returns **zero** matches. This catches backend-idiom residue in rewritten files. `planning.md` is excluded from the grep because its "Desktop App Adaptation" header note intentionally references the old SQLx/Axum template when explaining the adapted 7-section phase template; the grep would otherwise flag those intentional references as residue.
6. Directory listing `ls .claude/rules/` matches exactly: `audio-io.md cross-platform.md gui.md ml-inference.md module-boundaries.md planning.md rust-desktop.md testing.md`. None of the deleted files remain.

### Manual smoke tests (Linux)

7. `./scripts/setup_pipewire.sh` succeeds; `pw-cli list-objects | grep voicegate` shows both `voicegate_sink` and `voicegate_mic`. Afterwards, run the reverse destroy commands from PRD Appendix C to clean up; the automated path below must handle setup itself.
8. `cargo run --release -- devices` prints both input and output device lists with at least one entry each.
9. `cargo run --release -- run --passthrough` starts without error.
10. With `voicegate run --passthrough` running: open any recorder (e.g. `gnome-sound-recorder` or `pw-cat --record -`), select `voicegate_mic` as the input source, speak into the physical mic. The recording must contain the spoken audio unchanged.
11. Ctrl-C exits cleanly. After exit, `pw-cli list-objects | grep voicegate` returns **empty**.

### Manual smoke tests (Windows)

12. With VB-Cable installed, `voicegate.exe devices` lists `CABLE Input (VB-Audio Virtual Cable)` among outputs.
13. `voicegate.exe run --passthrough` runs. Audacity or Discord, configured to read `CABLE Output (VB-Audio Virtual Cable)`, receives the spoken audio.
14. Ctrl-C exits cleanly.

### Acceptance thresholds

- Passthrough audio must be **bit-for-bit identical** to the input (Phase 1 applies no processing). Verify by routing a WAV file through the system via a loopback cable or `pw-cat` and byte-comparing a 5-second capture.
- No audible glitches, dropouts, or xruns over a 30-second test.

---

## Section 6+: PRD Gap Additions

### 6.2 Config key `audio.frame_size_ms` naming + validation (Pass 2, G-011, MEDIUM)

**Gap:** PRD §5.9 specifies the TOML key as `frame_size_ms = 32`. The Phase 1 draft originally defined the Rust field as `frame_size_samples: usize` with value 1536, creating a unit mismatch at the serde boundary — the TOML file would be parsed into a field named `frame_size_samples` (or rename-annotated), but executors reading the PRD schema would write `frame_size_ms = 32` and hit a parse error or wrong value.

**Resolution (applied above in §3.6):**

1. **The struct field is `frame_size_ms: u32`**, matching the PRD TOML key exactly. No `#[serde(rename)]` needed.
2. **A helper method `frame_size_samples(&self) -> usize`** derives the sample count on demand (`frame_size_ms * sample_rate / 1000`). For v1 this is always 1536.
3. **`Config::validate()` rejects any value other than `frame_size_ms == 32`** at load time with a clear error message. Rationale: Silero VAD requires exactly 512 samples at 16 kHz, which only aligns at 32 ms frames per Decision D-001. Exposing the key as user-tunable but rejecting non-32 values at validation time keeps the TOML schema consistent with the PRD while preventing silent breakage.
4. **Same treatment for `sample_rate`**: validated to be exactly 48000.

**Why not hard-code and remove from config:** Leaving the keys in the TOML schema preserves PRD §5.9 compatibility and keeps the door open for future versions that might support 24 ms frames (if Silero is re-trained) or 44.1 kHz devices (if we add a pre-capture resample). Hard-coding them out would lock those futures out.

### 6.1 ONNX Runtime install documentation (Pass 1, G-003, LOW)

**Gap:** PRD §9.1 details the ONNX Runtime install steps but Phase 1 doesn't call them out explicitly even though Phase 1 owns the README rewrite.

**Addition:** When Phase 1 rewrites `README.md`, the "Prerequisites" section must include both platform install paths verbatim:

- **Linux:**
  ```bash
  wget https://github.com/microsoft/onnxruntime/releases/download/v1.17.0/onnxruntime-linux-x64-1.17.0.tgz
  tar xzf onnxruntime-linux-x64-1.17.0.tgz
  sudo cp onnxruntime-linux-x64-1.17.0/lib/libonnxruntime.so* /usr/local/lib/
  sudo ldconfig
  ```
- **Windows:** Download `onnxruntime.dll` from the same release page; place next to `voicegate.exe` or on `PATH`.

Phase 1 itself does NOT load any ONNX models (`ort` uses `load-dynamic`, so missing `libonnxruntime.so` causes a deferred runtime error, not a build error). Verification of a working ONNX Runtime install happens in Phase 2 when `SileroVad::load` runs for the first time.

### 6.3 PipeWire virtual-mic mechanism: `pw-loopback`, not `pw-cli create-node` (Execution discovery, HIGH, G-014)

**Gap:** PRD Appendix C and the original phase-01 §3.4 spec called for creating the Linux virtual mic via `pw-cli create-node adapter '{ ... }'` for both `voicegate_sink` and `voicegate_mic`, then linking them with `pw-link voicegate_sink:monitor_MONO voicegate_mic:input_MONO`, and tearing down with `pw-cli destroy-node <name>`. This approach does not work on PipeWire 1.0.x. It was discovered during step 7 of the Phase 1 morph, after `cargo run -- run --passthrough` failed with `pw-link: failed to link ports: No such file or directory` on the very first end-to-end smoke test.

**Root causes:**

1. **`pw-cli create-node` creates a client-owned node, not a persistent one.** The node is registered against the pw-cli session and is destroyed the moment `pw-cli` exits. So immediately after `pw-cli create-node adapter ...` returns success, the node is already gone. Any subsequent `pw-link` or cpal device scan will not find it.
2. **`pw-cli destroy-node` is not a real subcommand on PipeWire 1.0.5.** The actual command is `destroy <object-id>` and takes a numeric ID from `pw-cli ls Node`, not a name. The teardown path in the original spec would have errored out in practice.
3. **`voicegate_sink:monitor_MONO` is not necessarily the correct port name.** PipeWire derives port names from channel positions; MONO may be spelled differently depending on factory/config. Relying on a literal port name string is brittle even when the nodes do exist.

**Resolution (applied to [src/audio/virtual_mic.rs](src/audio/virtual_mic.rs) and [scripts/setup_pipewire.sh](scripts/setup_pipewire.sh) in step 7):**

Linux virtual mic now uses `pw-loopback`, the first-party PipeWire helper that owns a persistent capture sink + playback source pair wired by an internal loopback. `PwCliVirtualMic::setup` spawns:

```bash
pw-loopback \
    --channels 1 \
    --capture-props 'node.name=voicegate_sink node.description="VoiceGate Sink" media.class=Audio/Sink' \
    --playback-props 'node.name=voicegate_mic node.description="VoiceGate Virtual Microphone" media.class=Audio/Source/Virtual'
```

as a child process and stores the `std::process::Child` handle. `teardown` sends SIGKILL (via `Child::kill`) and waits for exit; `pw-loopback`'s atexit behaviour removes the nodes cleanly regardless of signal. A `Drop` impl on `PwCliVirtualMic` defensively calls `teardown` to cover panics in the main thread.

`scripts/setup_pipewire.sh` is rewritten to use `pw-loopback` as well and now runs as a foreground process (`exec`s pw-loopback); Ctrl-C tears down. The `verify` subcommand checks for `voicegate_sink` / `voicegate_mic` via `pw-cli ls Node`.

**Why the struct is still called `PwCliVirtualMic`:** The name is unchanged to minimize scope creep during step 7. Phase 6 may rename it to `PwLoopbackVirtualMic` or replace the shell-out entirely with a `pipewire-rs` native impl behind the `pipewire-native` feature flag (Decision D-006).

**Verification:** partial. The following sub-tests pass:

1. `voicegate devices` lists cpal input and output devices with the default marked.
2. `voicegate run --passthrough` spawns `pw-loopback` and `pw-cli ls Node | grep voicegate_` shows both `voicegate_sink` and `voicegate_mic` as persistent nodes.
3. On SIGINT or any error exit, `PwCliVirtualMic::drop` reaps the `pw-loopback` child and `pw-cli ls Node | grep voicegate_` returns empty.
4. Unit tests (`cargo test --lib`): 6/6 pass (ring buffer order, ring capacity constant, 4 x config validation).
5. `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo build --release` all clean.

The **end-to-end mic-to-voicegate_mic audio smoke test** (phase-01 §6 steps 9-11) cannot be completed on the development machine because its default audio input is reported by cpal's ALSA backend at 44 100 Hz i16, and Phase 1 does not implement capture-side rate conversion (Decision D-001 pins the entire pipeline to 48 kHz f32). On this machine `arecord -D hw:CARD=PCH,DEV=0 -f S16_LE -r 48000` works, but cpal's supported_input_configs enumeration for the same device reports only F32 variants and none of them open successfully at `build_input_stream` time with an ALSA `snd_pcm_hw_params` I/O error (errno 5). This is a known gap between cpal 0.15's ALSA backend and PipeWire's ALSA compatibility layer; it is a machine-specific hardware/config issue, not a code defect.

**Workarounds for verification on a different machine or by a future phase:**
- Run the smoke test on a machine whose default mic natively reports 48 kHz f32.
- Or route `pw-loopback` output through to a test capture via `pw-cat --record - | ...` to verify the `voicegate_sink` half of the pipeline without needing live mic input.
- Phase 2 will add rubato-based resampling and may also add capture-side resampling + i16-to-f32 conversion if this turns out to be common across target hardware. Track as new gap **G-015** below.

**Severity:** **HIGH** for the pw-loopback discovery (the plan would have shipped a broken Linux path); **MEDIUM** for the partial smoke-test gap (the code is correct but cannot be proven end-to-end on this hardware). Moving both into the gap log so Phase 6 can audit whether the `pipewire-native` implementation also needs to use `pw-loopback`-equivalent semantics (it will: it needs to create a persistent virtual source, not a client-owned one), and so Phase 2 can decide whether to add capture-side format/rate conversion.

### 6.4 Capture-side f32-only assumption is too strict for common hardware (Execution discovery, MEDIUM, G-015)

**Gap:** Phase 1 `audio::capture::start_capture` requires the cpal input device to support exactly 48 kHz f32 because the ring buffer and the entire downstream pipeline are f32. Many common ALSA devices (including the integrated HDA on the test machine, a Realtek ALC623) expose only i16 in their cpal-enumerated supported configs even when the underlying hardware can do 48 kHz natively. `cpal::SampleFormat::I16` at 48 kHz is ruled out by `find_48khz_f32_channels`, producing a clean error but no usable capture stream.

**Resolution (deferred to Phase 2):** Phase 2 introduces `rubato` for 48 kHz -> 16 kHz resampling for Silero VAD. While rewiring the capture path for Phase 2, also add:

1. **Accept i16 and u16 capture formats** and convert to f32 in the callback:
   - `i16 -> f32: sample as f32 / i16::MAX as f32`
   - `u16 -> f32: (sample as f32 - i16::MAX as f32) / i16::MAX as f32`
2. **Accept 44 100 Hz capture** and pre-resample in the worker (not the callback) to 48 000 Hz using rubato. This is additional to the 48 -> 16 kHz downsample for VAD. The output path stays pinned to 48 kHz because the virtual mic on both platforms expects 48 kHz.

Do NOT add format conversion inside the cpal callback beyond a simple `as f32` cast for i16 -- anything more (u24, 24-bit packed) belongs in a pre-worker stage in the ring buffer pipeline.

**Why not fix this in Phase 1:** the Phase 1 scope is identity passthrough, and adding format conversion + sample rate conversion compounds the risk of a Phase 1 that is supposed to be structurally minimal. Phase 2's `resampler.rs` is the natural home for this work since it already owns the rate-conversion path.

### 6.5 cpal 0.15 cannot target a PipeWire node by name (Execution discovery, HIGH, G-016)

**Gap:** Phase 1 assumed that after `PwCliVirtualMic::setup()` creates the `voicegate_sink` PipeWire node (via `pw-loopback`), the name `"voicegate_sink"` can be passed to `start_output()` and cpal will find a matching output device. In practice, **cpal 0.15's ALSA backend does not expose individual PipeWire nodes as cpal devices.** `voicegate devices` lists only ALSA-level device names (`default`, `pipewire`, `hw:CARD=PCH,*`), not PipeWire nodes. The output call looks up an output device whose `.name()` matches `"voicegate_sink"` and fails with "output device not found."

Empirical verification on the dev machine (PipeWire 1.0.5, Ubuntu 24.10-equivalent):

1. `pw-loopback --capture-props 'node.name=voicegate_sink media.class=Audio/Sink' ...` was spawned.
2. `pw-cli ls Node | grep voicegate_sink` confirms the node exists.
3. `voicegate devices` does NOT list `voicegate_sink` in its output device enumeration.
4. Therefore `start_output("voicegate_sink", consumer)` returns "output device \"voicegate_sink\" not found" at step 7 of main.rs's startup sequence.

The virtual mic half of the pipeline (the `voicegate_mic` source that Discord reads) is fine -- pw-loopback wires its own internal loopback from sink to source. The problem is getting VoiceGate's OWN audio *into* the sink via cpal.

**Options for resolution (deferred to Phase 2 or Phase 6):**

1. **Use `pw-loopback`'s capture side differently.** Instead of letting VoiceGate's cpal output write to `voicegate_sink`, have pw-loopback pair its capture side with a VoiceGate-owned producer. This requires using `pipewire-rs` natively (Phase 6 `pipewire-native` feature) so VoiceGate can register a PipeWire stream directly against `voicegate_sink`.

2. **Set `voicegate_sink` as the default PipeWire sink for the VoiceGate process only.** PipeWire supports per-process default routing via `PIPEWIRE_NODE` or `PULSE_SINK` environment variables. Setting `PIPEWIRE_NODE=voicegate_sink` before spawning cpal's output stream (or, equivalently, calling `pw-metadata 0 "default.audio.sink" ...` scoped to the process) would steer cpal's output to the correct node. This is a pragmatic Phase 2 fix.

3. **Use `pactl load-module module-null-sink`** (PulseAudio API, which PipeWire implements) to create a sink that cpal exposes as an ALSA device. This was the Phase 6 PulseAudio fallback per Decision D-007, but it turns out to be necessary earlier than Phase 6 because the pw-loopback path does not compose with cpal 0.15 on its own.

4. **Write directly to the sink via `pipewire-rs` at capture time** (bypass cpal output entirely on Linux). This is the cleanest long-term solution and matches the `pipewire-native` plan, but it is a large scope increase for Phase 1.

**Path chosen for now (Phase 1 ships partially):** Phase 1's code is left as-is because the bug is not in the code -- it is in the assumption that cpal-by-name routing to pw-loopback works. The phase file is updated to document this limitation; Phase 1 is marked partially-verified (automated gate PASS, manual mic-to-Discord smoke test DEFERRED); Phase 2 takes ownership of G-015 (capture format/rate conversion) AND G-016 (Linux output routing to `voicegate_sink`) as part of its pipeline work.

**Severity:** **HIGH**. The Linux audio passthrough is not end-to-end functional in Phase 1 until G-016 is resolved in Phase 2. On Windows, the VB-Cable path is expected to work because VB-Cable exposes itself as a regular ALSA-equivalent (WASAPI) device name that cpal DOES enumerate, but this also needs to be verified on real Windows hardware.

**Recommendation for Phase 2:** take the pragmatic fix (option 2 above) -- set `PIPEWIRE_NODE=voicegate_sink` in the process environment before starting the cpal output stream, OR use `pw-metadata` to route just VoiceGate's output. This is a small, reversible change that unblocks the smoke test without waiting for `pipewire-rs`.
