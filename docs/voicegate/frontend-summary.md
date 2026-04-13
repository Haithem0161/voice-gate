# VoiceGate Frontend Summary

**Purpose:** Cross-team handoff document. In a typical backend/frontend split this file lists every API change a frontend developer needs to absorb. VoiceGate has no web frontend — so here, "frontend" means **everything end users see and touch**: CLI flags, GUI screens, config keys, file paths, error messages, and install-time UX.

**Update discipline:** This file MUST be updated after EACH phase completes, never batched at the end. Each phase's section is appended when that phase's verification gate passes.

---

## How to read this file

- Each phase has its own `## Phase N Delivered` section that lists every user-visible surface-area change.
- A change is "user-visible" if a user can observe it via: a CLI invocation, a GUI widget, a log message, a file path, an install step, or a packaging artifact.
- Internal refactors, type changes, and library upgrades are **not** listed here — they belong in phase files and commit messages.

---

## Phase 0 — Current state (before the morph)

**What the user sees today:**
- This is a Rust backend template. End users never run it directly. The only surface is an HTTP server at port 3000 with a Swagger UI and a `/api/users` CRUD demo. Nothing about VoiceGate.

**After Phase 1 lands, the above is gone. The backend HTTP surface is replaced by a desktop binary.**

---

## Phase 1 — Foundation Morph + Audio Passthrough (not yet delivered)

### New CLI commands

| Command | Description |
|---------|-------------|
| `voicegate --version` | Prints `voicegate 0.1.0`. Standard `clap` behavior. |
| `voicegate --help` | Prints usage. |
| `voicegate devices` | Lists all cpal input and output devices with an `IN`/`OUT` prefix. Marks the defaults with `(default)`. |
| `voicegate run --passthrough` | Starts mic → virtual-mic loopback. No ML. Press Ctrl-C to stop. On Linux, creates `voicegate_sink` and `voicegate_mic` PipeWire nodes via `pw-cli`; on Windows, writes to `CABLE Input (VB-Audio Virtual Cable)`. |

### New file paths

| Path | Purpose |
|------|---------|
| `~/.config/voicegate/config.toml` (Linux) / `%APPDATA%\voicegate\config.toml` (Windows) | User config. Created on first save. Phase 1 only writes the `[audio]` section. |
| `models/silero_vad.onnx` | Expected location. Phase 1 does not load it but the directory exists as `models/.gitkeep`. |
| `assets/enrollment_passages.txt` | The PRD Appendix A passage, shipped as an asset. |

### New GUI surface

None. Phase 1 is CLI-only.

### New install steps

**Linux:**
1. `sudo apt install libasound2-dev libpipewire-0.3-dev pkg-config pipewire pipewire-audio-client-libraries libclang-dev`
2. Install ONNX Runtime shared library to `/usr/local/lib/` (documented in README).
3. `cargo build --release`
4. `./target/release/voicegate run --passthrough`

**Windows:**
1. Install VB-Audio Virtual Cable from https://vb-audio.com/Cable/ (reboot).
2. Place `onnxruntime.dll` next to the binary.
3. `cargo build --release`
4. `voicegate.exe run --passthrough`

### Error messages users may encounter

- `"pw-cli not found. VoiceGate requires PipeWire on Linux. See README.md."` — Linux without PipeWire (Phase 6 adds PulseAudio fallback).
- `"CABLE Input (VB-Audio Virtual Cable) not found. Install from https://vb-audio.com/Cable/ and reboot."` — Windows without VB-Cable.
- `"Microphone access denied. On Linux, check 'pactl list short sources' and PipeWire permissions. On Windows, Settings → Privacy → Microphone."` — OS-level mic permission missing.

### Breaking changes from the previous state

- **The backend HTTP API is gone.** Any previous reference to `http://localhost:3000/api/*` no longer exists.
- The Docker Compose stack is gone. PostgreSQL is not part of VoiceGate.
- `make db`, `make migrate`, `make backend` targets are gone. Replaced by `make models`, `make dev`, `make release`, `make package-linux`, `make package-windows`.
- `.env` files referencing `DATABASE_URL`/`JWT_SECRET` are removed and will not be respected.

---

## Phase 2 — ML Inference Primitives (not yet delivered)

### New CLI commands

None. Phase 2 is library-only.

### New file paths

| Path | Purpose |
|------|---------|
| `models/silero_vad.onnx` | Downloaded by `make models`. ~2 MB. |
| `models/ecapa_tdnn.onnx` | Exported by `make models`. ~80 MB. |
| `tests/fixtures/*.wav` | Test fixtures. Downloaded by `scripts/download_fixtures.sh`. |

### New GUI surface

None.

### New install steps

- `pip install speechbrain torch onnx onnxruntime numpy` (only needed if a contributor regenerates models; end users consume pre-exported ONNX files).
- `make models` (runs the two Python scripts).

### New error messages

- `"ML model file not found: models/ecapa_tdnn.onnx. Run 'make models' or set VOICEGATE_MODELS_DIR."`
- `"ONNX Runtime not available. Install libonnxruntime.so (Linux) or onnxruntime.dll (Windows)."`

### Internal additions visible to developers (not end users)

- `src/ml/vad.rs`, `src/ml/embedding.rs`, `src/ml/similarity.rs` are now importable.
- `Resampler48to16` is fleshed out.

---

## Phase 3 — Enrollment + CLI (not yet delivered)

### New CLI commands

| Command | Description |
|---------|-------------|
| `voicegate enroll --wav <path>` | Read audio from a WAV file (any rate/channels), extract a centroid embedding, save to profile.bin. |
| `voicegate enroll --mic <seconds>` | Record live from the mic for N seconds (30 recommended), extract a centroid embedding, save. Prints the enrollment passage before recording starts. |
| `voicegate enroll --list-passages` | Print the enrollment passage from Appendix A and exit. |
| `voicegate enroll --output <path>` | Override the default profile location. |
| `voicegate enroll --device <name>` | Override the input device (only applies with `--mic`). |

### New file paths

| Path | Purpose |
|------|---------|
| `~/.local/share/voicegate/profile.bin` (Linux) / `%APPDATA%\voicegate\profile.bin` (Windows) | Saved speaker profile. 784 bytes for v1 (magic + version + dim + 192 floats + CRC32). |

### New error messages

- `"enrollment produced only N segments, need at least 5. Speak for longer or more clearly."`
- `"profile format error: invalid magic bytes (expected VGPR, found [0x58, 0x58, 0x58, 0x58])"`
- `"profile format error: checksum mismatch"`
- `"profile format error: unsupported profile version 99"`

---

## Phase 4 — Gate + Pipeline Integration (not yet delivered)

### New CLI commands

| Command | Description |
|---------|-------------|
| `voicegate run --headless --profile <path>` | Runs the full speaker-gated pipeline with an enrolled profile. No GUI. Press Ctrl-C to stop. |
| `voicegate run --headless` | Same as above but uses the default profile location. |

### New config keys (`config.toml` now supports all PRD §5.9 sections)

```toml
[audio]
input_device = "default"
output_device = "auto"
frame_size_ms = 32
sample_rate = 48000

[vad]
threshold = 0.5
model_path = "models/silero_vad.onnx"

[verification]
threshold = 0.70
embedding_window_sec = 1.5
embedding_interval_ms = 200
ema_alpha = 0.3
model_path = "models/ecapa_tdnn.onnx"

[gate]
hold_frames = 5
crossfade_ms = 5

[enrollment]
profile_path = "auto"
min_duration_sec = 20
segment_duration_sec = 3

[gui]
show_similarity_meter = true
show_waveform = false
```

### New error messages

- `"pipeline error: <details>"` — non-fatal, frame is zeroed and pipeline continues.
- `"No profile found at <path>. Run 'voicegate enroll' first."`

### Performance expectations users will observe

- Startup time: <2 seconds (model loading + device init).
- End-to-end latency: <50 ms (imperceptible).
- CPU: <10% of one core.

---

## Phase 5 — GUI (not yet delivered)

### New CLI commands

| Command | Description |
|---------|-------------|
| `voicegate` (no subcommand) | Launches the GUI. Default behavior from Phase 5 onwards. |
| `voicegate run` (no flags) | Also launches the GUI. `--headless` and `--passthrough` remain as escape hatches. |

### New GUI screens

**Main screen:**
- Status indicator (Active / Inactive).
- Input device dropdown (refreshable).
- Output device dropdown (`auto` by default, resolves to `voicegate_sink` on Linux / `CABLE Input` on Windows).
- Real-time similarity meter (0.0–1.0 progress bar).
- Threshold slider (0.50–0.95, default 0.70).
- Hold-time slider (40–500 ms, default 160 ms).
- Bypass dropdown: Normal / Force Open / Force Closed.
- Gate LED (open = green, closed = gray).
- VAD LED (speech = green, silence = off).
- "Start / Stop" button.
- "Re-enroll Voice" button.

**Enrollment wizard:**
- Passage display (PRD Appendix A).
- Progress bar (0 / 30 seconds).
- "Start" button.
- "Cancel" button (works mid-recording).
- "Finish Early" button (works after 20 s).
- "Close" button (after success or failure).

### New file paths

No new paths. The GUI reads and writes the same `config.toml` and `profile.bin` as the CLI.

### New error banners (shown at the top of the main screen)

- `"No profile found. Please enroll your voice first."`
- `"Microphone disconnected. Reconnecting..."` (Phase 6 improves this)
- `"Failed to start virtual microphone: <reason>"`

### Window title and icon

- Title: `"VoiceGate"`.
- Default size: 480 × 360.
- Icon: placeholder in Phase 5; real icon in Phase 6.

---

## Phase 6 — Cross-Platform Hardening + Release (not yet delivered)

### New CLI commands

| Command | Description |
|---------|-------------|
| `voicegate doctor` | Environment diagnostics: platform, audio server, virtual-mic availability, ONNX runtime, model presence, profile presence. Exits 0 if healthy, 1 if any check fails. |
| `voicegate enroll --anti-target <name> --wav <path>` | Add a "not me" voice (e.g. a brother) to improve discrimination. |
| `voicegate enroll --anti-target <name> --mic <seconds>` | Same but from the mic. |

### New packaging artifacts

| Platform | Artifact | Notes |
|----------|----------|-------|
| Linux | `voicegate-<tag>-linux-x86_64.AppImage` | Bundles binary, onnxruntime.so, silero_vad.onnx, assets. ECAPA-TDNN downloaded on first run. |
| Windows | `voicegate-<tag>-windows-x86_64.msi` | Bundles binary, onnxruntime.dll, silero_vad.onnx, assets. ECAPA-TDNN downloaded on first run. MSI is unsigned in v1. |

### New install steps

**Linux AppImage:**
1. `sudo apt install libasound2 libpipewire-0.3-0` (runtime libs only, not `-dev`).
2. Download the AppImage; `chmod +x`; run.
3. On first run, consent to ECAPA-TDNN download (~80 MB).

**Windows MSI:**
1. Install VB-Audio Virtual Cable (prerequisite, not bundled).
2. Install the MSI. Start Menu shortcut is created.
3. On first run, consent to ECAPA-TDNN download.

### New file format

- `profile.bin` version **2**: adds up to 8 anti-target embeddings with UTF-8 names. v1 profiles are auto-upgraded in memory on load; saves always write v2.

### Resilience improvements visible to users

- Mic disconnect during active operation → reconnect attempts (1, 2, 4, 8 s backoff) with a persistent GUI banner.
- PipeWire/PulseAudio autodetect: VoiceGate picks whichever is running. Clear error if neither is present.
- `voicegate doctor` gives a single-glance health check for support situations.

### Updated config.toml (no breaking changes)

Missing new keys are filled with defaults automatically on load (`#[serde(default)]`).

### New docs

- `README.md` — full install + quick-start for both platforms.
- `TROUBLESHOOTING.md` — common problems: VB-Cable not detected, PipeWire perm denied, model download failed, enrollment too short, similar-sounding siblings.

---

## End-of-project surface summary (target)

After Phase 6 lands, a new user can:

1. Download the AppImage (Linux) or MSI (Windows).
2. Install prerequisites (runtime libs on Linux, VB-Cable on Windows).
3. Launch VoiceGate → GUI opens.
4. Click "Re-enroll Voice" → read the passage for 30 s → profile saved.
5. (Optional) Add anti-targets for similar-sounding people.
6. In Discord → Settings → Voice → Input Device → select `VoiceGate Virtual Microphone` (Linux) or `CABLE Output (VB-Audio Virtual Cable)` (Windows).
7. Speak on Discord. Only their own voice gets through.

Total CLI subcommand count: **4** (`devices`, `run`, `enroll`, `doctor`).
Total GUI screens: **2** (main, enrollment wizard).
Total installable artifacts: **2** (AppImage, MSI).
