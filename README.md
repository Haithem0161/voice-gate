# VoiceGate

**Real-time speaker isolation for Discord.** VoiceGate listens to your microphone, recognizes your voice against an enrolled profile, and routes only your audio to a virtual microphone that Discord (or any other app) picks up. Other speakers in the room are gated out before Discord ever hears them.

- Cross-platform Rust desktop binary: Windows 10/11 and Ubuntu 22.04+
- On-device speaker verification (Silero VAD + ECAPA-TDNN)
- <50 ms end-to-end latency, <10% CPU on a mid-range machine
- Zero-install virtual mic on Linux (PipeWire); VB-Audio Virtual Cable on Windows
- No cloud, no telemetry, no account

> **Status:** Active development. See [docs/voicegate/status.md](docs/voicegate/status.md) for phase progress and the [PRD](voicegate-prd.md) for the full specification.

## How it works

```
  physical mic -> cpal capture -> ring buffer -> worker thread -> virtual mic -> Discord
                                                       |
                                                       +-- Silero VAD (speech / no-speech)
                                                       |
                                                       +-- ECAPA-TDNN embedding (who is talking)
                                                       |
                                                       +-- cosine similarity vs enrolled profile
                                                       |
                                                       +-- gate state machine (open / closed / crossfade)
```

Enrollment captures ~30 seconds of your voice, extracts a 192-dim speaker embedding centroid, and saves it to `profile.bin`. At runtime, VoiceGate extracts a rolling embedding from the last 1.5 seconds of audio every 200 ms, compares it against the enrolled centroid, and opens the gate only when similarity exceeds the configured threshold (default 0.70). See [docs/voicegate/research.md](docs/voicegate/research.md) for the full design.

## Prerequisites

### Linux (Ubuntu 22.04+)

System packages:

```bash
sudo apt update
sudo apt install -y \
    build-essential pkg-config \
    libasound2-dev \
    libpipewire-0.3-dev \
    pipewire-audio-client-libraries \
    libclang-dev \
    python3 python3-pip
```

PipeWire must be running. On Ubuntu 22.04 LTS you may need to switch from PulseAudio; on 23.04+ PipeWire is default. Check with `pw-cli info`.

**ONNX Runtime shared library** (required for ML inference in Phase 2+):

```bash
wget https://github.com/microsoft/onnxruntime/releases/download/v1.17.0/onnxruntime-linux-x64-1.17.0.tgz
tar xzf onnxruntime-linux-x64-1.17.0.tgz
sudo cp onnxruntime-linux-x64-1.17.0/lib/libonnxruntime.so* /usr/local/lib/
sudo ldconfig
```

Phase 1 (audio passthrough) does not require ONNX Runtime and will build and run without it, but Phase 2 onward does.

### Windows 10/11

- [Rust](https://rustup.rs/) 1.83+ (MSVC toolchain)
- [VB-Audio Virtual Cable](https://vb-audio.com/Cable/) (free, required for the virtual microphone)
- **ONNX Runtime DLL**: download `onnxruntime.dll` from [the ONNX Runtime releases page](https://github.com/microsoft/onnxruntime/releases/tag/v1.17.0) and place it next to `voicegate.exe` or on `PATH`.

After installing VB-Cable, reboot. In Discord, set your input device to **CABLE Output (VB-Audio Virtual Cable)**. VoiceGate will write to **CABLE Input (VB-Audio Virtual Cable)**.

### Both platforms

- [Rust](https://rustup.rs/) 1.83+ (pinned by `rust-toolchain.toml`)
- Python 3.10+ (only needed once, to download and export the ONNX models)

## Build and run

```bash
# Install tooling (once)
make setup

# List your audio devices to pick input/output names
make devices

# Phase 1 smoke test: mic -> virtual mic passthrough (no gating)
make run-passthrough

# Download + export ML models (Phase 2+)
make models

# Lint gate
make lint

# Run all tests
make test

# Release build
make release
```

On Linux, the first time you run VoiceGate it will create two PipeWire nodes, `voicegate_sink` and `voicegate_mic`. `voicegate_mic` is the device Discord should select as input. On clean Ctrl-C exit VoiceGate destroys the nodes; if the process is killed hard, clean up with:

```bash
pw-cli destroy-node voicegate_sink
pw-cli destroy-node voicegate_mic
```

## Enrollment (Phase 3+)

```bash
# From a WAV file
voicegate enroll --wav path/to/30s_clean_speech.wav

# From a live mic recording (reads the PRD Appendix A passage aloud)
voicegate enroll --mic 30

# Print the recommended enrollment passage
voicegate enroll --list-passages
```

The resulting `profile.bin` is written under `dirs::data_dir()/voicegate/`:

- Linux: `~/.local/share/voicegate/profile.bin`
- Windows: `%APPDATA%\voicegate\profile.bin`

## Running with the gate (Phase 4+)

```bash
# Headless (no GUI)
voicegate run --headless --profile ~/.local/share/voicegate/profile.bin

# GUI (default)
voicegate run
```

## Configuration

User-editable TOML at:

- Linux: `~/.config/voicegate/config.toml`
- Windows: `%APPDATA%\voicegate\config.toml`

A default file is written on first launch. Full schema: see PRD §5.9.

## Project layout

```
voicegate/
  src/              Rust source
    main.rs           CLI entry point (clap)
    lib.rs            module re-exports + error types
    audio/            cpal capture/output, ring buffer, resampler, virtual mic
    config/           TOML Config struct with validate()
    ml/               Silero VAD + ECAPA-TDNN wrappers (Phase 2+)
    enrollment/       Profile format, enroll subcommand (Phase 3+)
    gate/             Gate state machine with crossfade (Phase 4+)
    pipeline/         Worker thread orchestration (Phase 4+)
    gui/              eframe/egui app + enrollment wizard (Phase 5+)
  scripts/          Python model download/export, pw-cli setup
  models/           ONNX models (gitignored, fetched via `make models`)
  tests/            Integration tests reading fixture WAVs
    fixtures/         speaker_a.wav, speaker_b.wav, mixed_ab.wav, ...
  assets/           Enrollment passages (PRD Appendix A)
  docs/voicegate/   Plan files (roadmap, research, phases, status)
  voicegate-prd.md  Product requirements document
```

## Troubleshooting

**Linux: "pw-cli not found"**
PipeWire is not installed or not running. Install `pipewire` + `pipewire-audio-client-libraries` and verify with `pw-cli info`.

**Linux: "microphone access denied"**
Check PipeWire permissions: `pactl list short sources`. Your user should have access to the default mic.

**Windows: "CABLE Input not found"**
VB-Cable is not installed. Download from https://vb-audio.com/Cable/ and reboot.

**Passthrough plays nothing in Discord (Linux)**
Confirm Discord is reading from `voicegate_mic` (not the physical mic). `pw-cli list-objects | grep voicegate` should show both `voicegate_sink` and `voicegate_mic`.

**Passthrough plays nothing in Discord (Windows)**
Confirm Discord input is set to `CABLE Output (VB-Audio Virtual Cable)` (the OUTPUT side of VB-Cable is what Discord reads).

**First model download fails**
`make models` requires Python 3.10+ and network access to GitHub and Hugging Face. Check your proxy settings.

## License

MIT. See individual model licenses in `docs/voicegate/research.md` (Silero VAD is MIT; SpeechBrain ECAPA-TDNN model weights follow VoxCeleb's research-use license).
