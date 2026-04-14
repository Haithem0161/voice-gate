---
paths:
  - "src/**/*.rs"
  - ".github/workflows/**"
  - "scripts/setup_pipewire.sh"
  - "Cargo.toml"
---

# Cross-Platform Rules (Windows + Linux)

VoiceGate ships on two targets: `x86_64-unknown-linux-gnu` (Ubuntu 22.04+) and `x86_64-pc-windows-msvc` (Windows 10/11). macOS is out of scope for v1 and behind `#[cfg(target_os = "macos")]` gates that produce `unimplemented!()` errors.

## Target Matrix

| Concern | Linux | Windows | macOS (v1) |
|---------|-------|---------|------------|
| Target triple | `x86_64-unknown-linux-gnu` | `x86_64-pc-windows-msvc` | out of scope |
| cpal backend | ALSA (works through PipeWire/PulseAudio) | WASAPI | CoreAudio |
| Virtual mic | `voicegate_sink` + `voicegate_mic` via PipeWire (`pw-cli`) | VB-Audio Virtual Cable (`CABLE Input` / `CABLE Output`) | n/a |
| ONNX Runtime lib | `libonnxruntime.so*` in `/usr/local/lib` | `onnxruntime.dll` next to exe or on PATH | n/a |
| Dev headers needed | `libasound2-dev`, `libpipewire-0.3-dev`, `libclang-dev` | none (MSVC toolchain) | n/a |
| Config path | `~/.config/voicegate/config.toml` | `%APPDATA%\voicegate\config.toml` | `~/Library/Application Support/voicegate/config.toml` |
| Profile path | `~/.local/share/voicegate/profile.bin` | `%APPDATA%\voicegate\profile.bin` | `~/Library/Application Support/voicegate/profile.bin` |
| Packaging | AppImage / tarball | MSI via `cargo-wix` | n/a |
| CI runner | `ubuntu-latest` | `windows-latest` | n/a |

## `#[cfg(target_os)]` Split Points

Keep the split as high in the module tree as possible. Prefer one file that branches at the top over many `#[cfg]` sprinkles.

| File | Split |
|------|-------|
| `src/audio/virtual_mic.rs` | `#[cfg(target_os = "linux")] mod pw_cli;` + `#[cfg(target_os = "windows")] mod vb_cable;`. `create_virtual_mic()` is the single `#[cfg]` branch. |
| `src/audio/audio_server.rs` (Phase 6) | Linux-only file, entirely behind `#[cfg(target_os = "linux")]`. Module is not declared at all on Windows. |
| `src/audio/capture.rs` + `output.rs` | No split — cpal handles WASAPI vs ALSA transparently. |
| `src/config/settings.rs` | No split. `dirs::config_dir()` handles platform differences. |

On macOS, `create_virtual_mic()` returns an error with `VoiceGateError::VirtualMic("macOS not supported in v1".into())`. Do NOT `unimplemented!()` — the binary must compile on macOS for contributors using the platform, even if runtime errors out.

## Path Resolution via `dirs`

- Config: `dirs::config_dir().expect("config dir").join("voicegate").join("config.toml")`
  - Linux: `$XDG_CONFIG_HOME` or `~/.config`
  - Windows: `%APPDATA%` (same as `dirs::data_dir()` on Windows — this is the intentional convention)
  - macOS: `~/Library/Application Support`
- Profile: `dirs::data_dir().expect("data dir").join("voicegate").join("profile.bin")`
  - Linux: `$XDG_DATA_HOME` or `~/.local/share`
  - Windows: `%APPDATA%`
  - macOS: `~/Library/Application Support`
- Models: relative to the executable (`models/silero_vad.onnx`) during dev. Phase 6 packaging embeds them in the AppImage / MSI.

Create directories before writing: `std::fs::create_dir_all(parent)?;`.

## ONNX Runtime Shared Library

- **`ort` crate feature `load-dynamic`**: the ONNX Runtime shared library is resolved at runtime, not link time. A missing shared library causes a runtime error at first `Session` creation, not a build failure. This is intentional — Phase 1 (audio passthrough) compiles without ONNX Runtime installed.
- **Linux install** (README prereqs, phase-01 §6.1 G-003):
  ```bash
  wget https://github.com/microsoft/onnxruntime/releases/download/v1.17.0/onnxruntime-linux-x64-1.17.0.tgz
  tar xzf onnxruntime-linux-x64-1.17.0.tgz
  sudo cp onnxruntime-linux-x64-1.17.0/lib/libonnxruntime.so* /usr/local/lib/
  sudo ldconfig
  ```
- **Windows install**: download `onnxruntime.dll` from the same release page; place next to `voicegate.exe` or on `%PATH%`.
- **Version pinning**: `1.17.0`. Later versions may work, but 1.17.x is the verified minimum.

## Linux Audio Stack

Ubuntu 22.04 ships PulseAudio by default on older setups and PipeWire on newer ones. 22.04 LTS with the HWE kernel + PipeWire is the target. To verify PipeWire is running: `pw-cli info 0` returns a node listing.

**Phase 1 audio-server assumption: PipeWire is present.** `src/audio/virtual_mic.rs`'s Linux impl checks `which::which("pw-cli")` at construction and errors out if missing. PulseAudio fallback is Phase 6 (Decision D-007 in `docs/voicegate/research.md`).

### `pw-cli` approach (Phase 1)

```bash
# Create a null sink that we write to
pw-cli create-node adapter '{
  factory.name=support.null-audio-sink
  node.name=voicegate_sink
  media.class=Audio/Sink
  audio.channels=1
  audio.position=[MONO]
}'

# Create a virtual source that Discord reads from
pw-cli create-node adapter '{
  factory.name=support.null-audio-sink
  node.name=voicegate_mic
  media.class=Audio/Source/Virtual
  audio.channels=1
  audio.position=[MONO]
}'

# Link the sink monitor to the virtual source
pw-link voicegate_sink:monitor_MONO voicegate_mic:input_MONO
```

- VoiceGate writes to `voicegate_sink` via cpal output (appears as an output device to cpal).
- Discord reads from `voicegate_mic` (appears as an input device to Discord, NOT to cpal).
- Teardown: `pw-cli destroy-node voicegate_sink && pw-cli destroy-node voicegate_mic` (idempotent).

### `pipewire-rs` approach (Phase 6, optional)

Behind the `pipewire-native` cargo feature. Directly binds to the PipeWire C API via `pipewire = "0.8"`. Eliminates the `pw-cli` subprocess dependency but adds build complexity (requires `libpipewire-0.3-dev` at build time). Default feature set stays on `pw-cli` for simpler packaging.

### PulseAudio fallback (Phase 6)

For Ubuntu 22.04 installs that still use PulseAudio:

```bash
pactl load-module module-null-sink sink_name=voicegate_sink
pactl load-module module-remap-source master=voicegate_sink.monitor source_name=voicegate_mic
```

Track the module indices returned by `pactl load-module` to call `pactl unload-module <index>` on teardown.

## Windows: VB-Audio Virtual Cable

- User-installed, free, from https://vb-audio.com/Cable/ — requires a reboot after install.
- **Detection**: scan `cpal::Host::output_devices()?` for a device whose name matches `"CABLE Input (VB-Audio Virtual Cable)"`. That is the OUTPUT side that VoiceGate writes to.
- **Discord setup**: Discord's input device should be set to `"CABLE Output (VB-Audio Virtual Cable)"`. That is the INPUT side that Discord reads.
- The naming is confusing — "CABLE Input" is what VoiceGate writes to (an output from cpal's perspective), and "CABLE Output" is what Discord reads (an input from Discord's perspective). Document this in the README and the GUI error message.
- If detection fails, return `VoiceGateError::VirtualMic("VB-Audio Virtual Cable not found. Install from https://vb-audio.com/Cable/ and reboot.")`. The GUI should display this with a clickable link (`open::that("https://vb-audio.com/Cable/")`).

## CI Matrix

`.github/workflows/ci.yml`:

- `lint` job on `ubuntu-latest` — runs `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.
- `test` job matrix `{ubuntu-latest, windows-latest}` — runs `cargo check --all-targets`, `cargo test --all-targets`, and `cargo build --release`.
- Linux step installs `pkg-config libasound2-dev libpipewire-0.3-dev libclang-dev` before cargo.
- Windows step installs nothing extra — the MSVC toolchain + rust-cache suffices.
- ONNX Runtime shared library is NOT installed in CI for the initial phases. Phase 2+ will either download it in a setup step or vendor a lightweight test-only copy.

## Packaging (Phase 6)

### Linux: AppImage

- Tooling: `linuxdeploy` + `cargo appimage` (or a plain shell script that assembles the AppDir).
- Bundle contents: `voicegate` binary, `libonnxruntime.so.1.17.0`, `models/*.onnx`, `assets/enrollment_passages.txt`, `scripts/setup_pipewire.sh`, `.desktop` file, icon.
- Entry point: launch `voicegate` directly; no installer.

### Windows: MSI via `cargo-wix`

- `cargo install cargo-wix` then `cargo wix init` (Phase 6) to generate the WiX template.
- MSI contents: `voicegate.exe`, `onnxruntime.dll`, `models/*.onnx`, `assets/enrollment_passages.txt`, start-menu shortcut.
- Unsigned in v1. Users will see a SmartScreen warning. Code signing is deferred to v1.1.

## Pitfalls

- **Hard-coding `/home/user/...`** in `src/` — use `dirs` always. Same for `C:\Users\...`.
- **Assuming `/` path separators** — use `std::path::Path::join` and `PathBuf`. Never string-concatenate paths.
- **Linking against ONNX Runtime at build time** — the `load-dynamic` feature is the entire reason the build graph is portable. Do NOT enable any ort feature that pulls in a system library via `build.rs` unless you are prepared to handle the cross-OS install.
- **Calling `pw-cli` on Windows** — gate everything under `#[cfg(target_os = "linux")]`. The module file can still exist (to shrink the diff for cross-platform PRs); just wrap the body.
- **Assuming `pkg-config` exists on Windows** — it doesn't unless the user installs vcpkg. For VoiceGate that is not a problem because only Linux deps (`alsa-sys`, `pipewire-sys`) need pkg-config.
- **Shipping an AppImage that calls `/usr/local/lib/libonnxruntime.so`** — the AppImage must bundle its own copy and set `LD_LIBRARY_PATH` at launch, otherwise moving the binary to a machine without system ORT breaks it.
- **Forgetting to preserve environment on `sudo`** when installing ONNX Runtime — not a VoiceGate concern, but document in the README prereqs.
