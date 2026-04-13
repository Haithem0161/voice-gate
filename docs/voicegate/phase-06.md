# Phase 6: Cross-Platform Hardening + Release

**Goal:** Turn the working Phase 5 app into something that ships — autodetect audio servers on Linux, detect VB-Cable on Windows with clear errors, add negative-enrollment for similar-sounding siblings, profile and optimize the pipeline, set up cross-platform CI, package for both platforms, and write install/troubleshooting docs.

**Dependencies:** Phases 1–5 (everything). This phase does not add new product functionality so much as make the existing app robust, shippable, and observable.

**Complexity:** M

---

## Section 1: Module & File Changes

### 1.1 Files to CREATE

**Rust source:**

```
src/audio/audio_server.rs        # AudioServer enum + detect_audio_server()
src/enrollment/anti_target.rs    # anti-target embeddings for negative enrollment
```

**Packaging + CI:**

```
packaging/linux/voicegate.desktop        # Freedesktop .desktop entry
packaging/linux/AppImageBuilder.yml      # appimage-builder config
packaging/windows/wix.toml               # cargo-wix config (MSI)
.github/workflows/release.yml            # release pipeline (tag-triggered)
```

**Docs (additions to README.md and a new TROUBLESHOOTING.md):**

```
TROUBLESHOOTING.md      # Common problems + fixes (e.g. VB-Cable not found, PipeWire perm denied)
```

### 1.2 Files to MODIFY

| Path | Change |
|------|--------|
| `src/audio/virtual_mic.rs` | Add `PwNativeVirtualMic` (behind `pipewire-native` feature) and `PulseVirtualMic` (always compiled on Linux). The `create_virtual_mic()` factory calls `detect_audio_server()` and picks the right impl. |
| `src/enrollment/profile.rs` | Add `PROFILE_VERSION = 2` that stores anti-target embeddings. `load()` accepts both v1 and v2 (up-converting v1 to in-memory v2 with zero anti-targets). |
| `src/ml/similarity.rs` | Add `SpeakerVerifier::update_with_anti_targets` that computes `score = sim(self) - max(sim(anti_targets))` and uses that as the smoothed value. |
| `src/main.rs` | Add `enroll --anti-target <name>` flag, `doctor` subcommand (audio-server + VB-Cable diagnostics). |
| `src/gui/app.rs` | Show a "Manage anti-targets" button that opens a small dialog listing stored anti-targets. |
| `.github/workflows/ci.yml` | Finalize matrix CI with cache, artifact upload on tags, and PipeWire/ALSA install step. |
| `Makefile` | `package-linux` → builds AppImage via `appimage-builder`. `package-windows` → runs `cargo wix` to produce an MSI. |
| `README.md` | Expand install section with both platforms, add a "Similar-sounding siblings" section documenting negative enrollment. |
| `docs/voicegate/research.md` | Fill in §9 Open Questions with measured answers from Phase 4's latency/CPU tests. |
| `Cargo.toml` | Enable `pipewire-native` feature as part of release builds **if** Q-004 from research.md resolves positive. Otherwise keep default. |

---

## Section 2: Dependencies & Build Config

**Potential new crates:**

```toml
# (Only if Q-004 from research.md resolves positive)
# pipewire = "0.8"   (already listed under [target.'cfg(target_os = "linux")'] in Phase 1)

# Packaging tooling (NOT dependencies — installed via cargo install):
# cargo install cargo-wix         # Windows MSI
# cargo install cargo-bundle      # Alternative to appimage-builder
```

**`appimage-builder`** is a Python tool (`pip install appimage-builder`), not a Rust crate. Documented in Phase 6 README install section.

**`cargo-wix`** is a cargo subcommand; install via `cargo install cargo-wix` in the CI workflow.

**No new runtime dependencies.** All new Rust code uses crates already pinned.

**CI caching:**
- `~/.cargo/registry/`, `~/.cargo/git/`, `target/` cached via `actions/cache@v4`.
- Key: `${{ runner.os }}-cargo-${{ hashFiles('Cargo.lock') }}`.

---

## Section 3: Types, Traits & Public API

### 3.1 `src/audio/audio_server.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioServer {
    PipeWire,
    PulseAudio,
    Unknown,
}

pub fn detect_audio_server() -> AudioServer {
    if which::which("pw-cli").is_ok()
        && std::process::Command::new("pw-cli").arg("info").output()
            .map(|o| o.status.success()).unwrap_or(false)
    {
        AudioServer::PipeWire
    } else if which::which("pactl").is_ok()
        && std::process::Command::new("pactl").arg("info").output()
            .map(|o| o.status.success()).unwrap_or(false)
    {
        AudioServer::PulseAudio
    } else {
        AudioServer::Unknown
    }
}
```

### 3.2 `src/audio/virtual_mic.rs` — Linux fallback chain

```rust
#[cfg(target_os = "linux")]
pub fn create_virtual_mic() -> Box<dyn VirtualMic> {
    match detect_audio_server() {
        AudioServer::PipeWire => {
            #[cfg(feature = "pipewire-native")]
            { Box::new(PwNativeVirtualMic::new()) }
            #[cfg(not(feature = "pipewire-native"))]
            { Box::new(PwCliVirtualMic::new()) }
        }
        AudioServer::PulseAudio => Box::new(PulseVirtualMic::new()),
        AudioServer::Unknown => Box::new(NullVirtualMic::new()),
    }
}

#[cfg(target_os = "linux")]
pub struct PulseVirtualMic {
    loaded_modules: Vec<u32>,  // module indices from `pactl load-module`
}

impl VirtualMic for PulseVirtualMic {
    fn setup(&mut self) -> anyhow::Result<String> {
        // pactl load-module module-null-sink sink_name=voicegate_sink ...
        // pactl load-module module-remap-source master=voicegate_sink.monitor source_name=voicegate_mic ...
        // Parse each output for the module index and push onto loaded_modules.
        // Return "voicegate_sink" (the cpal output device name).
    }

    fn teardown(&mut self) -> anyhow::Result<()> {
        // For each index in loaded_modules, pactl unload-module <index>.
    }

    fn discord_device_name(&self) -> &str { "VoiceGate_Virtual_Microphone" }
}
```

### 3.3 `src/enrollment/anti_target.rs`

```rust
pub const MAX_ANTI_TARGETS: usize = 8;

#[derive(Debug, Clone)]
pub struct AntiTarget {
    pub name: String,          // user-provided label, max 32 bytes UTF-8
    pub embedding: Vec<f32>,   // EMBEDDING_DIM, L2-normalized
}
```

Stored in `Profile` (v2) as:

```rust
pub struct Profile {
    pub version: u32,                    // 1 or 2
    pub embedding: Vec<f32>,             // the target (self) centroid
    pub anti_targets: Vec<AntiTarget>,   // empty for v1 profiles, 0..MAX for v2
}
```

### 3.4 `src/enrollment/profile.rs` — v2 format

```
Offset                      Size            Field
-------                     ------          -----
  0                           4             magic "VGPR"
  4                           4             version = 2
  8                           4             embedding_dim (same as v1)
 12                          4*D            target embedding (same as v1)
 12+4D                         1            anti_target_count u8 (0..MAX_ANTI_TARGETS)
 13+4D                     ...per target... (see below)
 final                        4             CRC32

Each anti-target entry:
   1 byte    name_len u8
   N bytes   UTF-8 name (name_len)
  4*D bytes  embedding
```

`load()` dispatches on version:
- `1`: reads the old format, sets `anti_targets = vec![]`, returns a v2-in-memory struct.
- `2`: reads the full format.

`save()` always writes v2 (even if `anti_targets.is_empty()`).

### 3.5 `src/ml/similarity.rs` — margin-based verification

```rust
impl SpeakerVerifier {
    /// Used when anti-targets are present. Computes:
    ///   raw = sim(live, self) - max(sim(live, anti_i))
    /// then applies the same EMA smoothing. Raw scores can be negative.
    /// Match decision: smoothed > (threshold - 0.5) to account for the shifted range.
    pub fn update_with_anti_targets(
        &mut self,
        live: &[f32],
        anti_targets: &[AntiTarget],
    ) -> VerifyResult;
}
```

The threshold shift (`threshold - 0.5`) is because margin scores typically range in [-0.5, +0.5] rather than [0, 1]. Phase 6 verification tunes the exact shift with real data.

### 3.6 `src/main.rs` — new commands

```rust
#[derive(Subcommand)]
enum Commands {
    // Existing: Devices, Run { .. }, Enroll { .. } (extended)

    /// Diagnose the environment. Prints detected audio server, virtual-mic
    /// availability, ONNX Runtime status, model file presence, profile presence.
    Doctor,
}

// Enroll gains:
// --anti-target <name>    Record as an anti-target instead of the primary target.
//                         Appends to existing profile.bin (creates anti_targets array).
```

---

## Section 4: Runtime Behavior

### 4.1 `voicegate doctor`

Prints a diagnostic report. Exit code 0 if everything is OK, 1 if any check fails.

```
VoiceGate Diagnostics
=====================
Platform:         linux (x86_64)
Audio server:     PipeWire (pw-cli 1.0.5)
Virtual mic:      Available (PwCliVirtualMic)
ONNX Runtime:     Found (/usr/local/lib/libonnxruntime.so.1.17.0)
Silero VAD:       Found (models/silero_vad.onnx, 2.1 MB)
ECAPA-TDNN:       Found (models/ecapa_tdnn.onnx, 79.8 MB)
Profile:          Found (~/.local/share/voicegate/profile.bin, version 2, 3 anti-targets)
Config:           Found (~/.config/voicegate/config.toml)
Default input:    Microphone (Logitech Webcam)
Default output:   voicegate_sink

All checks passed.
```

On Windows, the same report with VB-Cable detection replacing the audio-server line.

### 4.2 Audio-server autodetect at startup (main app, not just `doctor`)

- `main()` calls `detect_audio_server()` once on Linux before creating the virtual mic.
- If `AudioServer::Unknown`, exit with a clear error: "No supported audio server detected. VoiceGate requires PipeWire or PulseAudio. See TROUBLESHOOTING.md."
- If `PipeWire`, proceed with `PwCliVirtualMic` (or `PwNativeVirtualMic` if feature enabled).
- If `PulseAudio`, proceed with `PulseVirtualMic`.

### 4.3 Negative enrollment flow (`enroll --anti-target <name>`)

1. Load existing profile from `Profile::default_path()`. If none, error: "Enroll your own voice first with `voicegate enroll --wav ...`".
2. If already at `MAX_ANTI_TARGETS`, error: "Maximum anti-targets reached. Remove one first."
3. If `name` is already in use, error: "Anti-target '{name}' already exists. Use a different name or remove the old one."
4. Run the same enrollment flow as primary enrollment (15 seconds minimum instead of 20, per PRD §5.8).
5. Append to `profile.anti_targets`.
6. Save (always writes v2).
7. Print "Anti-target '{name}' added. Profile now has {N} anti-targets."

### 4.4 Pipeline uses anti-targets when present

In Phase 4's `process_frame`:

```rust
// Step 3.2 becomes:
let result = if self.profile.anti_targets.is_empty() {
    self.verifier.update(&live)
} else {
    self.verifier.update_with_anti_targets(&live, &self.profile.anti_targets)
};
```

The GUI threshold slider remains a plain [0.50, 0.95] value; internally, when anti-targets are active, the margin shift is applied automatically.

### 4.5 Profiling + optimization

1. Run `cargo flamegraph --bin voicegate -- run --headless --profile ~/.local/share/voicegate/profile.bin` for 60 seconds of real audio.
2. Identify the top 3 hot spots in `process_frame`. Expected order: ECAPA-TDNN session.run > Silero VAD session.run > rubato resample.
3. Optimizations to consider only if Phase 4's <10% CPU target was missed:
    - Pre-allocate ort `Value` handles instead of rebuilding per call.
    - Reduce `REEXTRACT_INTERVAL_SAMPLES_16K` from 200 ms to 300 ms (cuts embedding calls by 33%).
    - Enable `ort` DirectML (Windows) or CUDA (Linux NVIDIA) execution provider as an optional feature flag.
4. Document findings in `research.md` §7.

### 4.6 CI release workflow

`.github/workflows/release.yml` triggered on tag `v*`:

```yaml
on:
  push:
    tags: ['v*']

jobs:
  build-linux:
    runs-on: ubuntu-latest
    steps:
      - checkout
      - install libasound2-dev libpipewire-0.3-dev pkg-config libclang-dev
      - install onnxruntime-linux-x64-1.17.0 to /usr/local/lib
      - cargo build --release
      - download silero_vad.onnx (artifact from a fixed release URL)
      - pip install appimage-builder
      - make package-linux
      - upload voicegate-<tag>-linux-x86_64.AppImage to the GitHub release

  build-windows:
    runs-on: windows-latest
    steps:
      - checkout
      - install cargo-wix
      - cargo build --release
      - download silero_vad.onnx + onnxruntime.dll
      - make package-windows
      - upload voicegate-<tag>-windows-x86_64.msi
```

The ECAPA-TDNN model is **too large (80 MB)** for the AppImage/MSI without bloating the binary. Decision: download on first run via a built-in prompt (user consents, VoiceGate fetches from a pinned URL and verifies SHA-256). Document this in Phase 6 verification and README.

### 4.7 Graceful device disconnect (Phase 6 addition)

- Input callback error → worker thread sets `status.error_state = 1` and attempts to restart the capture stream with exponential backoff (1 s, 2 s, 4 s, 8 s, give up).
- GUI shows a persistent error banner during reconnect attempts.
- Phase 1 explicitly deferred this; Phase 6 implements it.

---

## Section 5: Cross-Platform & Resource Handling

### 5.1 Linux packaging

- **AppImage** built with `appimage-builder` bundles: the `voicegate` binary, `libonnxruntime.so`, `silero_vad.onnx`, `assets/`, a `.desktop` entry, an icon.
- ECAPA-TDNN is NOT bundled; fetched on first run.
- Target glibc: ubuntu-latest is currently 24.04 (glibc 2.39). For broader compatibility, consider building on ubuntu-22.04 in CI instead. Document the minimum glibc in README.
- `.desktop` entry registers `voicegate` as an application so GNOME/KDE app menus show it.

### 5.2 Windows packaging

- `cargo wix` produces an MSI that installs `voicegate.exe`, `onnxruntime.dll`, `silero_vad.onnx`, `assets/`, and a Start Menu shortcut.
- ECAPA-TDNN fetched on first run (same as Linux).
- MSI is unsigned in v1; signing is a v1.1 concern.
- **Prerequisite:** user must install VB-Cable separately. The MSI installer displays a page linking to https://vb-audio.com/Cable/ during install.

### 5.3 First-run model fetch

- On launch, if `models/ecapa_tdnn.onnx` is missing, the GUI shows a "Download ECAPA-TDNN model (80 MB)?" dialog.
- Download URL is pinned in a const: `const ECAPA_URL: &str = "https://github.com/<owner>/voicegate/releases/download/models-v1/ecapa_tdnn.onnx";`
- Download with `reqwest` or `ureq`? Neither is in Cargo.toml yet. Decision: use `ureq` (smallest), `cargo add ureq@2 --features tls`.
- After download, verify SHA-256 against a pinned hash. If mismatch, delete and show an error.
- On CLI builds, `voicegate enroll` / `voicegate run --headless` error out with a clear message pointing at `voicegate doctor` and the download URL.

### 5.4 Config migration

- If `config.toml` exists with missing new keys (e.g. from an older version), `Config::load` fills defaults. This is automatic with `#[serde(default)]` on each field.

### 5.5 Profile migration

- v1 profiles auto-upgrade to v2 on load (in-memory). Saves always write v2.
- A v2 profile CANNOT be read by a v1-only binary. Document this.

---

## Section 6: Verification

### Automated

1. **`test_audio_server_detect`** — Mock `Command` execution (or run on a test machine with known state) and assert the detection result.
2. **`test_profile_v1_to_v2_upgrade`** — Create a v1 profile on disk, load it with the v2-aware loader, assert `version == 2`, `anti_targets.is_empty()`, `embedding` matches.
3. **`test_profile_v2_roundtrip`** — Create a profile with 3 anti-targets, save, load, assert all fields match.
4. **`test_anti_target_margin_discrimination`** — Enroll speaker A, add speaker B as anti-target. Run `update_with_anti_targets` on a speaker-B sample. Assert the returned score is lower than `update` without anti-targets.
5. **`test_pipeline_with_anti_targets`** — Full pipeline test: enroll A, add B as anti-target, run on `mixed_ab.wav`. Assert speaker-B RMS reduction is better than Phase 4's baseline by at least 10%.
6. **`test_doctor_output_shape`** — Run `voicegate doctor`, parse output, assert all expected lines are present.
7. **Cross-platform CI matrix green:** `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release` all pass on `ubuntu-latest` and `windows-latest`.

### Manual — Linux AppImage

8. On a **fresh** Ubuntu 22.04 VM (not the dev machine):
    1. Install prerequisites: `sudo apt install libasound2 libpipewire-0.3-0`.
    2. Install onnxruntime.so to `/usr/local/lib/` (documented in README).
    3. Download `voicegate-<tag>-linux-x86_64.AppImage`, `chmod +x`, run it.
    4. First-run dialog prompts for ECAPA-TDNN download; consent; download completes.
    5. Enrollment wizard works end-to-end; enrollment completes in <60 seconds (PRD §13.1).
    6. Start the pipeline; speak; similarity meter updates; gate opens and closes correctly.
    7. Ctrl-C (or close window); PipeWire nodes torn down.

### Manual — Windows MSI

9. On a **fresh** Windows 11 VM:
    1. Install VB-Audio Virtual Cable; reboot.
    2. Install `voicegate-<tag>-windows-x86_64.msi`; launch from Start Menu.
    3. Same flow as steps 4–7 above.

### Manual — Negative enrollment

10. `voicegate enroll --wav speaker_a_enroll.wav` → primary enroll succeeds.
11. `voicegate enroll --anti-target brother1 --wav speaker_b.wav` → anti-target added.
12. Re-run the pipeline on `mixed_ab.wav`. Compare output to the Phase 4 baseline. Speaker-B bleed-through is reduced (document the improvement percentage).
13. `voicegate doctor` shows `Profile: Found (... version 2, 1 anti-target)`.

### PRD §13 success criteria audit (the final check)

Walk PRD §13 line by line and confirm each passes:

| # | Criterion | Status |
|---|-----------|:------:|
| 1 | Enrollment completes in <60 seconds | ✅/❌ per Phase 5 test 6 + Phase 6 test 8.5 |
| 2 | Enrolled user's voice passes through clearly | ✅/❌ per Phase 4 test 9 + Phase 6 test 8.6 |
| 3 | Other voices silenced >90% of the time | ✅/❌ per Phase 4 test 11 |
| 4 | End-to-end latency <50 ms | ✅/❌ per Phase 4 test 13 |
| 5 | CPU usage <10% | ✅/❌ per Phase 4 test 14 |
| 6 | No clicks/artifacts at gate transitions | ✅/❌ per Phase 4 tests 6, 8 |
| 7 | Runs stably for hours without crashes or leaks | ✅/❌ per Phase 4 test 15 (30 min) + Phase 6 soak test |
| 8 | Threshold tunable enough for similar voices | ✅/❌ per Phase 6 test 12 |
| 9 | Works on Windows 10/11 AND Ubuntu 22.04+ | ✅/❌ per Phase 6 tests 8, 9 |
| 10 | Linux: virtual mic auto-created (zero extra install) | ✅/❌ per Phase 6 test 8.6 |

Results are recorded in `PHASES-1-6-VERIFICATION.md` as the project's final closeout.

### Lint / build (reminder)

14. `cargo clippy -- -D warnings` clean on both platforms.
15. `cargo fmt --check` clean.
16. `cargo build --release` green in CI matrix.

### Acceptance thresholds

- Every row of the PRD §13 audit must be ✅.
- AppImage install-to-first-use on a fresh VM: no more than 5 minutes of user action (not counting download time).
- MSI install-to-first-use on a fresh VM (with VB-Cable pre-installed): no more than 3 minutes.
- Anti-target improvement: speaker-B RMS reduction at least 10% better than Phase 4 baseline.

---

## Section 6+: PRD Gap Additions

_Pass 1 has not yet been run. Gaps from gap analysis will be appended here as numbered subsections (6.1, 6.2, ...)._
