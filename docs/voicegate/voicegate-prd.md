# VoiceGate — Real-Time Speaker Isolation for Discord

## Product Requirements Document

**Version:** 1.0
**Date:** April 13, 2026
**Author:** Claude (Anthropic) + You
**Target Developer:** Senior backend dev (Rust / Node)
**Build Tool:** Claude Code

---

## 1. Problem Statement

Three brothers share a single room, each on Discord simultaneously. Every microphone picks up all three voices, creating crosstalk hell. Existing noise suppression tools (Krisp, Discord built-in, RNNoise, NVIDIA Broadcast, SteelSeries Sonar) are designed to filter **non-voice noise** (fans, keyboards, dogs) while **preserving all human voices** — the exact opposite of what's needed here.

**What's actually needed:** A tool that learns the user's voice during a short enrollment session, then acts as a real-time gatekeeper — only passing audio frames that match the enrolled voice to a virtual microphone, and silencing everything else.

---

## 2. Product Overview

**VoiceGate** is a cross-platform desktop application written in Rust that sits between the user's physical microphone and Discord (via a virtual audio device). It runs on **Windows 10/11** and **Ubuntu Linux 22.04+**. It uses ML-based speaker verification to decide in real-time whether incoming audio belongs to the enrolled user or someone else, and gates accordingly.

### 2.1 High-Level Flow

```
Physical Mic → VoiceGate → Virtual Mic → Discord
                  │
                  ├── Is anyone talking? (VAD)
                  ├── Is it ME talking? (Speaker Verification)
                  ├── Yes → pass audio through
                  └── No → output silence
```

### 2.2 Core Value Proposition

- **Voice-specific isolation**, not generic noise suppression
- **Runs locally**, no data leaves the machine
- **Low latency** (<50ms end-to-end)
- **Free and open source**
- **Works with any mic**, no special hardware

---

## 3. Target Platforms & Requirements

### 3.1 Common Requirements

| Requirement | Detail |
|---|---|
| ML runtime | ONNX Runtime (CPU inference, GPU optional) |
| Min CPU | Any modern x86_64 (i5/Ryzen 5 or better recommended) |
| RAM | ~200MB working set |
| Disk | ~100MB (models + binary) |

### 3.2 Windows

| Requirement | Detail |
|---|---|
| OS | Windows 10/11 (x64) |
| Audio backend | WASAPI (via `cpal`) |
| Virtual mic | VB-Audio Virtual Cable (free, user-installed) |

### 3.3 Ubuntu Linux

| Requirement | Detail |
|---|---|
| OS | Ubuntu 22.04+ (x64), or any distro with PipeWire 0.3.48+ |
| Audio backend | PipeWire (via `cpal` ALSA backend) or PulseAudio |
| Virtual mic | PipeWire virtual source (created automatically by VoiceGate, no extra install needed) |
| Dependencies | `libpipewire-0.3-dev`, `libasound2-dev`, `pkg-config` |

**Key Linux difference:** On Linux, VoiceGate creates its own virtual microphone using PipeWire's module system — **no third-party virtual cable software is needed.** The app spawns a PipeWire source node that Discord can select as an input device. This is handled via a helper script or direct PipeWire API calls at startup.

---

## 4. Architecture

### 4.1 System Architecture Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                     VoiceGate Process                        │
│                                                             │
│  ┌──────────┐    ┌──────────────┐    ┌───────────────────┐  │
│  │ Physical  │    │ Audio Engine │    │  Virtual Mic Out  │  │
│  │ Mic Input ├───►│ (Ring Buffer)├───►│  Win: VB-Cable    │  │
│  │ (cpal)    │    │              │    │  Linux: PipeWire  │  │
│  └──────────┘    └──────┬───────┘    └───────────────────┘  │
│                         │                                    │
│                         ▼                                    │
│               ┌─────────────────┐                            │
│               │   ML Pipeline   │                            │
│               │                 │                            │
│               │  1. Resampler   │ 48kHz → 16kHz              │
│               │  2. Silero VAD  │ ~1ms inference              │
│               │  3. ECAPA-TDNN  │ ~5-10ms inference           │
│               │  4. Similarity  │ cosine distance             │
│               │  5. Gate Logic  │ threshold + hysteresis       │
│               └─────────────────┘                            │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐   │
│  │                  Enrollment Store                     │   │
│  │  Win: %APPDATA%\voicegate\profile.bin                 │   │
│  │  Linux: ~/.local/share/voicegate/profile.bin          │   │
│  └──────────────────────────────────────────────────────┘   │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐   │
│  │                   Config Store                        │   │
│  │  Win: %APPDATA%\voicegate\config.toml                 │   │
│  │  Linux: ~/.config/voicegate/config.toml               │   │
│  └──────────────────────────────────────────────────────┘   │
│                                                             │
│  ┌──────────────────────────────────────────────────────┐   │
│  │                    GUI (egui)                         │   │
│  │  - Device selection                                   │   │
│  │  - Enrollment wizard                                  │   │
│  │  - Threshold slider                                   │   │
│  │  - Real-time similarity meter                         │   │
│  │  - Bypass toggle                                      │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### 4.2 Module Breakdown

The project is organized into these Rust modules/crates:

```
voicegate/
├── Cargo.toml
├── models/                          # ONNX model files (git-lfs or downloaded at build)
│   ├── silero_vad.onnx             # ~2MB - Voice Activity Detection
│   └── ecapa_tdnn.onnx             # ~80MB - Speaker embedding extraction
├── src/
│   ├── main.rs                     # Entry point, CLI arg parsing, app bootstrap
│   ├── audio/
│   │   ├── mod.rs
│   │   ├── capture.rs              # Mic capture via cpal (cross-platform)
│   │   ├── output.rs               # Virtual mic output via cpal
│   │   ├── ring_buffer.rs          # Lock-free ring buffer for audio frames
│   │   ├── resampler.rs            # 48kHz ↔ 16kHz conversion (rubato)
│   │   └── virtual_mic.rs          # Platform-specific virtual mic setup
│   ├── ml/
│   │   ├── mod.rs
│   │   ├── vad.rs                  # Silero VAD wrapper
│   │   ├── embedding.rs            # ECAPA-TDNN embedding extraction
│   │   └── similarity.rs           # Cosine similarity + decision logic
│   ├── gate/
│   │   ├── mod.rs
│   │   └── gate.rs                 # Gating logic with hysteresis + crossfade
│   ├── enrollment/
│   │   ├── mod.rs
│   │   └── enroll.rs               # Voice enrollment session manager
│   ├── config/
│   │   ├── mod.rs
│   │   └── settings.rs             # Config load/save (TOML)
│   └── gui/
│       ├── mod.rs
│       └── app.rs                  # egui-based control panel
├── scripts/
│   ├── download_models.py          # Downloads ONNX models from HuggingFace
│   ├── export_ecapa.py             # Exports SpeechBrain ECAPA-TDNN to ONNX
│   └── setup_pipewire.sh           # Linux: creates PipeWire virtual source
└── tests/
    ├── test_vad.rs
    ├── test_embedding.rs
    ├── test_gate.rs
    └── fixtures/                   # Test WAV files
        ├── speaker_a.wav
        ├── speaker_b.wav
        └── mixed.wav
```

---

## 5. Detailed Component Specifications

### 5.1 Audio Capture (`audio/capture.rs`)

**Responsibility:** Capture raw PCM audio from the user's selected physical microphone.

**Implementation:**
- Use `cpal` crate — it abstracts over platform backends automatically:
  - **Windows:** WASAPI backend (default)
  - **Linux:** ALSA backend (PipeWire and PulseAudio both expose ALSA-compatible interfaces)
- Capture format: **f32 samples, 48000 Hz, mono** (downmix stereo if needed)
- Frame size: **20ms** (960 samples at 48kHz) — this is the atomic unit of processing
- Push frames into a lock-free ring buffer (`ringbuf` crate or custom `audio/ring_buffer.rs`)
- Handle device disconnection gracefully (attempt reconnect with backoff)

**Key details:**
```rust
// Pseudocode for capture setup — works on both Windows and Linux via cpal
let device = select_input_device(config.input_device_name)?;
let stream_config = cpal::StreamConfig {
    channels: 1,
    sample_rate: cpal::SampleRate(48000),
    buffer_size: cpal::BufferSize::Fixed(960), // 20ms
};

let stream = device.build_input_stream(
    &stream_config,
    move |data: &[f32], _| {
        ring_buffer.push_slice(data);
    },
    error_callback,
    None,
)?;
```

**Linux note:** Some ALSA devices don't support `BufferSize::Fixed`. Fall back to `BufferSize::Default` and handle variable-size callbacks by accumulating into the ring buffer and draining in fixed-size chunks from the processing thread.

### 5.2 Audio Output & Virtual Microphone (`audio/output.rs`, `audio/virtual_mic.rs`)

**Responsibility:** Write processed (gated) audio to a virtual microphone device that Discord can see as an input.

The virtual mic strategy is **platform-specific** — this is the biggest cross-platform divergence in the entire project.

#### 5.2.1 Windows: VB-Audio Virtual Cable

**Implementation:**
- Output to "CABLE Input (VB-Audio Virtual Cable)" via cpal — it appears as a normal output device
- Same format as capture: f32, 48kHz, mono
- The output callback pulls from a processed audio queue
- Discord selects "CABLE Output (VB-Audio Virtual Cable)" as its input device

**VB-Cable setup (user-facing):**
1. User downloads and installs VB-Audio Virtual Cable (free) from https://vb-audio.com/Cable/
2. In Discord: Settings → Voice & Video → Input Device → "CABLE Output (VB-Audio Virtual Cable)"
3. VoiceGate writes to "CABLE Input" — Discord reads from "CABLE Output"

#### 5.2.2 Linux: PipeWire Virtual Source (zero-install)

On Linux with PipeWire (default on Ubuntu 22.10+ and Fedora 34+), VoiceGate creates its own virtual microphone source **automatically at startup** — no third-party software needed.

**Implementation — Option A: pw-loopback (simplest, recommended for v1):**

VoiceGate spawns a `pw-loopback` process that creates a virtual source, then writes audio to it via a named pipe or a PipeWire filter node.

Actually, the cleanest approach is to use **`pipewire-rs`** (Rust bindings for PipeWire) to create a PipeWire source node directly:

```rust
// Pseudocode — PipeWire virtual source creation
// Uses the pipewire crate (Rust bindings)

use pipewire as pw;

fn create_virtual_source() -> pw::stream::Stream {
    let mainloop = pw::main_loop::MainLoop::new()?;
    let context = pw::context::Context::new(&mainloop)?;
    let core = context.connect(None)?;

    let stream = pw::stream::Stream::new(
        &core,
        "VoiceGate",
        pw::properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",  // Makes it appear as a mic
            *pw::keys::MEDIA_CLASS => "Audio/Source",
            *pw::keys::NODE_NAME => "voicegate_mic",
            *pw::keys::NODE_DESCRIPTION => "VoiceGate Virtual Microphone",
        },
    )?;

    // Connect as a source (producer of audio)
    stream.connect(
        pw::stream::Direction::Output,  // We output audio INTO the PipeWire graph
        None,
        pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
        &mut [/* audio format params */],
    )?;

    stream
}
```

In the stream's `process` callback, VoiceGate writes gated audio frames into the PipeWire buffer. Discord sees "VoiceGate Virtual Microphone" as an input device.

**Implementation — Option B: Shell script fallback (if pipewire-rs is painful):**

Create a FIFO pipe and use `pw-cat` or `pactl` to load a virtual source module:

```bash
#!/bin/bash
# scripts/setup_pipewire.sh — creates a VoiceGate virtual mic via PipeWire

# Create a null sink that VoiceGate writes to
pw-cli create-node adapter '{
    factory.name = support.null-audio-sink
    node.name = "voicegate_sink"
    node.description = "VoiceGate Sink"
    media.class = "Audio/Sink"
    audio.position = "MONO"
    audio.rate = 48000
}'

# Create a virtual source that mirrors the sink (Discord reads from this)
pw-cli create-node adapter '{
    factory.name = support.null-audio-sink
    node.name = "voicegate_mic"
    node.description = "VoiceGate Virtual Microphone"
    media.class = "Audio/Source/Virtual"
    audio.position = "MONO"
    audio.rate = 48000
}'

# Link them
pw-link voicegate_sink:monitor_MONO voicegate_mic:input_MONO

echo "VoiceGate virtual mic created. Select 'VoiceGate Virtual Microphone' in Discord."
```

With this approach, VoiceGate writes to the "voicegate_sink" via cpal (it shows up as an output device), and Discord reads from "voicegate_mic" (which mirrors the sink).

**PulseAudio fallback (Ubuntu 22.04 without PipeWire):**

```bash
# Create a virtual source via PulseAudio
pactl load-module module-null-sink sink_name=voicegate_sink sink_properties=device.description="VoiceGate_Sink" rate=48000 channels=1
pactl load-module module-remap-source master=voicegate_sink.monitor source_name=voicegate_mic source_properties=device.description="VoiceGate_Virtual_Microphone"
```

#### 5.2.3 Platform Abstraction (`audio/virtual_mic.rs`)

```rust
pub trait VirtualMic {
    /// Set up the virtual microphone. Returns the device name to use with cpal for output.
    fn setup(&mut self) -> Result<String>;
    /// Tear down the virtual microphone on exit.
    fn teardown(&mut self) -> Result<()>;
    /// Get the name Discord should use as input device (for display in GUI).
    fn discord_device_name(&self) -> &str;
}

#[cfg(target_os = "windows")]
pub struct WindowsVirtualMic { /* VB-Cable detection */ }

#[cfg(target_os = "linux")]
pub struct LinuxVirtualMic {
    pipewire_nodes: Vec<u32>,  // Node IDs to clean up on exit
}

/// Factory function
pub fn create_virtual_mic() -> Box<dyn VirtualMic> {
    #[cfg(target_os = "windows")]
    { Box::new(WindowsVirtualMic::new()) }

    #[cfg(target_os = "linux")]
    { Box::new(LinuxVirtualMic::new()) }
}
```

**Windows `setup()`:** Scan cpal output devices for "CABLE Input (VB-Audio Virtual Cable)". If not found, show an error in the GUI with a download link. Return the device name.

**Linux `setup()`:** Run the PipeWire/PulseAudio commands to create virtual source nodes. Return "voicegate_sink" as the cpal output target. On `teardown()`, destroy the created nodes via `pw-cli destroy-node` or `pactl unload-module`.

**Linux auto-detection:** At startup, detect whether the system uses PipeWire or PulseAudio:
```rust
fn detect_audio_server() -> AudioServer {
    // Check if PipeWire is running
    if Command::new("pw-cli").arg("info").output().is_ok() {
        AudioServer::PipeWire
    } else if Command::new("pactl").arg("info").output().is_ok() {
        AudioServer::PulseAudio
    } else {
        AudioServer::Unknown  // Fall back to ALSA only, user must configure manually
    }
}
```

### 5.3 Resampler (`audio/resampler.rs`)

**Responsibility:** Convert between 48kHz (audio device rate) and 16kHz (model input rate).

**Implementation:**
- Use `rubato` crate (high-quality async resampling)
- Downsample 48kHz → 16kHz for model inference (factor 3)
- Keep original 48kHz frames for passthrough (don't upsample back — just gate the original)
- Pre-allocate resampler buffers at startup to avoid runtime allocations

```rust
// 20ms at 48kHz = 960 samples
// 20ms at 16kHz = 320 samples
// Resample ratio: 16000/48000 = 1/3

let resampler = rubato::FftFixedIn::<f32>::new(48000, 16000, 960, 1, 1)?;
```

### 5.4 Silero VAD (`ml/vad.rs`)

**Responsibility:** First-pass filter — determine if any voice is present in the current frame. If no voice detected, skip the expensive embedding step and output silence immediately.

**Model:** Silero VAD v5 (ONNX)
- Source: https://github.com/snakers4/silero-vad
- Size: ~2MB
- Input: 16kHz f32 audio, 512 samples per chunk (32ms)
- Output: float [0.0, 1.0] — probability of speech
- Inference time: <1ms on CPU

**Implementation:**
```rust
pub struct SileroVad {
    session: ort::Session,
    state: Vec<f32>,  // Hidden state (GRU), persists across frames
    sample_rate: i64, // 16000
}

impl SileroVad {
    pub fn is_speech(&mut self, audio_16k: &[f32]) -> bool {
        // Run inference
        let prob = self.run_inference(audio_16k);
        prob > self.threshold  // Default threshold: 0.5
    }
}
```

**Important notes:**
- Silero VAD is stateful — it maintains internal GRU hidden state across calls. Do NOT recreate the session per frame.
- The VAD expects chunks of exactly 512 samples (32ms at 16kHz). If our frame is 320 samples (20ms at 16kHz), we need to buffer two frames and run VAD on 640 samples, or adjust frame size.
- **Recommended adjustment:** Use 30ms frames (1440 samples at 48kHz, 480 samples at 16kHz) and run VAD on 512-sample windows with overlap.
- OR: Use 32ms frames (1536 samples at 48kHz, 512 samples at 16kHz) to align exactly with VAD input size.

**Decision: Use 32ms frame size** to align with Silero VAD's expected 512-sample input. This gives us 1536 samples at 48kHz per frame, which is still well within latency budget.

### 5.5 Speaker Embedding Extraction (`ml/embedding.rs`)

**Responsibility:** Extract a fixed-size speaker embedding vector from an audio segment, used for comparing against the enrolled voice.

**Model:** ECAPA-TDNN (SpeechBrain)
- Source: https://huggingface.co/speechbrain/spkrec-ecapa-voxceleb
- Export to ONNX using `scripts/export_ecapa.py`
- Size: ~80MB (ONNX)
- Input: 16kHz f32 audio, variable length (minimum ~0.5s, optimal 1-3s)
- Output: 192-dimensional embedding vector (f32)
- Inference time: 5-15ms on CPU for 1s of audio

**Implementation:**
```rust
pub struct EcapaTdnn {
    session: ort::Session,
}

impl EcapaTdnn {
    /// Extract embedding from audio segment
    /// audio_16k: f32 samples at 16kHz, should be 0.5-3 seconds
    pub fn extract_embedding(&self, audio_16k: &[f32]) -> Vec<f32> {
        // Normalize audio
        let normalized = self.normalize(audio_16k);
        
        // Create input tensor [1, num_samples]
        let input = ndarray::Array2::from_shape_vec(
            (1, normalized.len()),
            normalized
        ).unwrap();
        
        // Run inference
        let outputs = self.session.run(ort::inputs!["input" => input])?;
        
        // Output shape: [1, 192]
        let embedding = outputs["output"].extract_tensor::<f32>()?;
        
        // L2 normalize the embedding
        l2_normalize(embedding.to_vec())
    }
}
```

**Sliding window approach for real-time:**

We can't wait 3 seconds to accumulate audio before running the model. Instead:

1. Maintain a sliding window buffer of the last **1.5 seconds** of audio (24000 samples at 16kHz)
2. When VAD triggers (speech detected), start filling the window
3. After accumulating at least **0.5 seconds** of speech (8000 samples), run embedding extraction
4. Continue running every **~200ms** (3200 samples) on the latest 1.5s window for updated similarity scores
5. Use exponential moving average on similarity scores to smooth decisions

```
Time: ──────────────────────────────────────────────►
Audio: [====VAD ON====][========SPEECH=========][===]
Window:                 [------1.5 seconds------]
                              ▲ extract embedding here
                                   ▲ and here (every 200ms)
```

### 5.6 Similarity Computation (`ml/similarity.rs`)

**Responsibility:** Compare the live embedding against the enrolled embedding and produce a match decision.

**Implementation:**
```rust
/// Cosine similarity between two L2-normalized vectors
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
    // Since both are L2-normalized, this is the cosine similarity
}

pub struct SpeakerVerifier {
    enrolled_embedding: Vec<f32>,  // Loaded from profile
    threshold: f32,                // Default: 0.70, configurable
    ema_alpha: f32,                // Smoothing factor: 0.3
    current_score: f32,            // EMA-smoothed similarity
}

impl SpeakerVerifier {
    pub fn update(&mut self, live_embedding: &[f32]) -> VerifyResult {
        let raw_score = cosine_similarity(&self.enrolled_embedding, live_embedding);
        
        // Exponential moving average for smoothing
        self.current_score = self.ema_alpha * raw_score 
                           + (1.0 - self.ema_alpha) * self.current_score;
        
        if self.current_score > self.threshold {
            VerifyResult::Match(self.current_score)
        } else {
            VerifyResult::NoMatch(self.current_score)
        }
    }
}
```

**Threshold guidance:**
- `0.60` — very loose, will let similar voices through. Use if brothers sound very different.
- `0.70` — balanced default. Good starting point.
- `0.80` — strict. May clip your own voice in some tones/volumes. Use if brothers sound similar.
- `0.85+` — very strict. Risk of false negatives (your own voice gets gated).

Expose this as a slider in the GUI so the user can tune it to their specific situation.

### 5.7 Gate Logic (`gate/gate.rs`)

**Responsibility:** Apply the pass/silence decision to the audio stream with smooth transitions to avoid clicks and pops.

**Implementation:**

```rust
pub struct AudioGate {
    state: GateState,
    crossfade_samples: usize,  // 256 samples (~5ms at 48kHz)
    hold_frames: usize,        // How many frames to keep open after last match (default: 5 = 160ms)
    frames_since_match: usize,
}

enum GateState {
    Open,
    Closing(usize),  // crossfade progress
    Closed,
    Opening(usize),  // crossfade progress
}

impl AudioGate {
    /// Process a frame of audio based on the ML decision
    pub fn process(&mut self, frame: &mut [f32], is_match: bool) {
        if is_match {
            self.frames_since_match = 0;
        } else {
            self.frames_since_match += 1;
        }
        
        let should_be_open = self.frames_since_match < self.hold_frames;
        
        match (&self.state, should_be_open) {
            (GateState::Closed, true) => {
                self.state = GateState::Opening(0);
                self.apply_fade_in(frame);
            }
            (GateState::Open, false) => {
                self.state = GateState::Closing(0);
                self.apply_fade_out(frame);
            }
            (GateState::Closed, false) => {
                // Silence the frame
                frame.fill(0.0);
            }
            (GateState::Open, true) => {
                // Pass through unchanged
            }
            // Handle ongoing crossfades...
            _ => self.continue_crossfade(frame),
        }
    }
    
    fn apply_fade_in(&self, frame: &mut [f32]) {
        for (i, sample) in frame.iter_mut().enumerate() {
            let gain = (i as f32 / self.crossfade_samples as f32).min(1.0);
            *sample *= gain;
        }
    }
    
    fn apply_fade_out(&self, frame: &mut [f32]) {
        for (i, sample) in frame.iter_mut().enumerate() {
            let gain = 1.0 - (i as f32 / self.crossfade_samples as f32).min(1.0);
            *sample *= gain;
        }
    }
}
```

**Hold time** is critical — it prevents the gate from rapidly opening/closing during natural speech pauses. 5 frames × 32ms = 160ms hold is a good default. This means after the last positive match, audio continues to pass for 160ms before the gate closes.

### 5.8 Enrollment (`enrollment/enroll.rs`)

**Responsibility:** Guide the user through recording their voice and computing a stable speaker embedding for future comparison.

**Enrollment flow:**

1. **Prompt user** to read a provided passage aloud for ~30 seconds
   - Use a pangram-heavy passage that covers diverse phonemes
   - Example passages stored in `assets/enrollment_passages.txt`
2. **Record** audio continuously during the enrollment session
3. **Segment** the recording using VAD to extract speech-only portions
4. **Extract embeddings** from multiple ~3-second segments (aim for 8-10 segments)
5. **Average** all embeddings (centroid) and L2-normalize → this is the enrolled embedding
6. **Save** the embedding to the platform-appropriate data directory:
   - Windows: `%APPDATA%\voicegate\profile.bin`
   - Linux: `~/.local/share/voicegate/profile.bin`

```rust
pub struct EnrollmentSession {
    recordings: Vec<Vec<f32>>,  // Raw 16kHz audio segments
    embeddings: Vec<Vec<f32>>,  // Extracted embeddings per segment
}

impl EnrollmentSession {
    pub fn finalize(&self) -> Vec<f32> {
        // Compute centroid of all embeddings
        let dim = self.embeddings[0].len();  // 192
        let mut centroid = vec![0.0f32; dim];
        
        for emb in &self.embeddings {
            for (i, val) in emb.iter().enumerate() {
                centroid[i] += val;
            }
        }
        
        let n = self.embeddings.len() as f32;
        for val in centroid.iter_mut() {
            *val /= n;
        }
        
        l2_normalize(centroid)
    }
}
```

**Profile format (`profile.bin`):**
```
[4 bytes: magic "VGPR"]
[4 bytes: version u32 = 1]
[4 bytes: embedding_dim u32 = 192]
[768 bytes: embedding f32 × 192]
[4 bytes: checksum CRC32]
```

**Optional negative enrollment:**

For better accuracy (especially with similar-sounding siblings), support optional negative enrollment:
1. Record each brother's voice for ~15 seconds
2. Extract their embeddings
3. Store as "anti-targets"
4. During verification, compute similarity against both the target and anti-targets
5. Use the margin: `score = similarity_to_self - max(similarity_to_others)` instead of raw similarity

This significantly improves discrimination between similar voices.

### 5.9 Configuration (`config/settings.rs`)

**Config file location:**
- **Windows:** `%APPDATA%\voicegate\config.toml`
- **Linux:** `~/.config/voicegate/config.toml`

(Resolved via the `dirs` crate — `dirs::config_dir()`)

```toml
[audio]
# On Windows: specific device name like "Microphone (Realtek High Definition Audio)"
# On Linux: ALSA device name like "default" or "hw:0,0", or PipeWire node name
input_device = "default"
# On Windows: "CABLE Input (VB-Audio Virtual Cable)"
# On Linux: auto-created "voicegate_sink" (leave as "auto" to let VoiceGate create it)
output_device = "auto"
frame_size_ms = 32
sample_rate = 48000

[vad]
threshold = 0.5           # Speech probability threshold
model_path = "models/silero_vad.onnx"

[verification]
threshold = 0.70          # Cosine similarity threshold (0.0 - 1.0)
embedding_window_sec = 1.5  # Sliding window size for embedding extraction
embedding_interval_ms = 200  # How often to re-extract embedding during speech
ema_alpha = 0.3            # Smoothing factor for similarity scores
model_path = "models/ecapa_tdnn.onnx"

[gate]
hold_frames = 5           # Frames to hold gate open after last match (× 32ms)
crossfade_ms = 5          # Crossfade duration for gate transitions

[enrollment]
# Resolved via dirs::data_dir():
#   Windows: %APPDATA%\voicegate\profile.bin
#   Linux: ~/.local/share/voicegate/profile.bin
profile_path = "auto"
min_duration_sec = 20     # Minimum enrollment recording duration
segment_duration_sec = 3  # Length of each enrollment segment

[gui]
show_similarity_meter = true
show_waveform = false
```

### 5.10 GUI (`gui/app.rs`)

**Framework:** `eframe` / `egui` — immediate-mode, lightweight, native on Windows.

**Screens:**

**1. Main Screen (normal operation)**
```
┌─────────────────────────────────────────────┐
│  VoiceGate                           [─][□][×]│
├─────────────────────────────────────────────┤
│                                             │
│  Status: ● Active                           │
│                                             │
│  Input:  [Realtek Mic           ▼]          │
│  Output: [CABLE Input (VB-Audio) ▼]         │
│                                             │
│  ┌─ Similarity ──────────────────────────┐  │
│  │ ████████████████░░░░░░░░  0.82        │  │
│  └───────────────────────────────────────┘  │
│                                             │
│  Threshold: [=========●=====] 0.70          │
│  Hold time: [===●===========] 160ms         │
│                                             │
│  [Re-enroll Voice]  [⏸ Bypass]              │
│                                             │
│  Gate: ● OPEN  │  VAD: ● Speech             │
│                                             │
└─────────────────────────────────────────────┘
```

**2. Enrollment Screen**
```
┌─────────────────────────────────────────────┐
│  Voice Enrollment                           │
├─────────────────────────────────────────────┤
│                                             │
│  Please read the following text aloud:      │
│                                             │
│  "The quick brown fox jumps over the lazy   │
│   dog. Pack my box with five dozen liquor   │
│   jugs. How vexingly quick daft zebras      │
│   jump. The five boxing wizards jump        │
│   quickly..."                               │
│                                             │
│  Recording: ●●●●●●●●○○○○○○○  15s / 30s     │
│                                             │
│  [Cancel]                    [Finish Early] │
│                                             │
└─────────────────────────────────────────────┘
```

---

## 6. Processing Pipeline — Detailed Frame-by-Frame Flow

This is the exact sequence of operations for every audio frame:

```
1. cpal capture callback fires with 1536 f32 samples (32ms at 48kHz)

2. Push raw samples into ring buffer
   └─ Ring buffer capacity: ~3 seconds (144,000 samples at 48kHz)

3. Processing thread pulls 1536 samples from ring buffer

4. Resample 48kHz → 16kHz
   └─ 1536 samples → 512 samples
   └─ Using rubato FftFixedIn resampler

5. Run Silero VAD on 512 samples (16kHz)
   └─ Output: speech_probability (f32)
   └─ If speech_probability < 0.5:
      └─ Gate decision = CLOSED
      └─ Skip to step 9

6. Append 512 samples to sliding window buffer (16kHz)
   └─ Window size: 24000 samples (1.5 seconds)
   └─ Circular/ring buffer, oldest samples dropped

7. If enough speech accumulated (>= 8000 samples / 0.5s)
   AND time since last embedding extraction >= 200ms:
   └─ Extract ECAPA-TDNN embedding from window buffer
   └─ Returns 192-dim f32 vector

8. Compute similarity decision
   └─ cosine_similarity(live_embedding, enrolled_embedding)
   └─ Apply EMA smoothing
   └─ If smoothed_score >= threshold: gate decision = OPEN
   └─ Else: gate decision = CLOSED

9. Apply gate to ORIGINAL 48kHz frame (1536 samples)
   └─ If OPEN: pass through unchanged
   └─ If CLOSED: write zeros (silence)
   └─ If transitioning: apply crossfade

10. Write gated 48kHz frame to output ring buffer

11. cpal output callback fires, pulls from output ring buffer,
    sends to virtual mic:
      - Windows: VB-Cable virtual input device
      - Linux: PipeWire/PulseAudio virtual source node
```

**Threading model:**

```
Thread 1: cpal input callback (real-time priority)
  └─ Writes to input ring buffer
  └─ MUST be non-blocking, no allocations, no locks

Thread 2: Processing thread (normal priority)
  └─ Reads from input ring buffer
  └─ Runs ML pipeline (VAD + embedding + similarity)
  └─ Applies gate
  └─ Writes to output ring buffer

Thread 3: cpal output callback (real-time priority)
  └─ Reads from output ring buffer
  └─ Writes to virtual mic
  └─ MUST be non-blocking, no allocations, no locks

Thread 4: GUI thread (normal priority)
  └─ egui render loop
  └─ Reads status from shared atomic state
  └─ User interactions update config via Arc<Mutex<Config>>
```

**Inter-thread communication:**
- Ring buffers: lock-free SPSC (`ringbuf` crate)
- Config updates: `Arc<RwLock<Config>>`
- Status reporting (similarity score, gate state): `Arc<AtomicF32>` or similar atomics
- Shutdown signal: `Arc<AtomicBool>`

---

## 7. ML Model Details & Export

### 7.1 Silero VAD

**Source:** https://github.com/snakers4/silero-vad/blob/master/src/silero_vad/data/silero_vad.onnx

Download directly — it's already in ONNX format.

**Input tensors:**
- `input`: f32 [1, 512] — 512 audio samples at 16kHz
- `sr`: i64 [1] — sample rate (16000)
- `state`: f32 [2, 1, 128] — GRU hidden state (initialize with zeros, persist across calls)

**Output tensors:**
- `output`: f32 [1] — speech probability
- `stateN`: f32 [2, 1, 128] — updated hidden state (feed back as input for next call)

### 7.2 ECAPA-TDNN Export

The model needs to be exported from SpeechBrain's PyTorch format to ONNX. Here's the export script:

**`scripts/export_ecapa.py`:**
```python
"""
Export SpeechBrain ECAPA-TDNN to ONNX format.

Usage:
    pip install speechbrain torch onnx onnxruntime
    python scripts/export_ecapa.py --output models/ecapa_tdnn.onnx

This downloads the pretrained model from HuggingFace and exports it.
"""

import torch
import speechbrain as sb
from speechbrain.inference.speaker import EncoderClassifier

def export():
    # Load pretrained model
    classifier = EncoderClassifier.from_hparams(
        source="speechbrain/spkrec-ecapa-voxceleb",
        savedir="tmp_model"
    )

    # Create dummy input (1 second of audio at 16kHz)
    dummy_input = torch.randn(1, 16000)

    # Get the encoder model
    model = classifier.mods.embedding_model

    # Set to eval mode
    model.eval()

    # Export to ONNX
    torch.onnx.export(
        model,
        dummy_input,
        "models/ecapa_tdnn.onnx",
        input_names=["input"],
        output_names=["output"],
        dynamic_axes={
            "input": {1: "audio_length"},  # Variable length audio
            "output": {0: "batch"}
        },
        opset_version=14,
        do_constant_folding=True,
    )

    print("Exported to models/ecapa_tdnn.onnx")

    # Verify
    import onnxruntime as onnxrt
    session = onnxrt.InferenceSession("models/ecapa_tdnn.onnx")
    result = session.run(None, {"input": dummy_input.numpy()})
    print(f"Output shape: {result[0].shape}")  # Should be [1, 192]

if __name__ == "__main__":
    export()
```

**IMPORTANT NOTE:** The exact export process may need adjustment based on SpeechBrain's model structure. The embedding model might be wrapped in additional layers. Claude Code should:
1. Install speechbrain and inspect `classifier.mods` to find the right submodule
2. Test that the ONNX output matches the PyTorch output
3. Ensure the ONNX model accepts variable-length input

**Alternative: Use a pre-exported model**

If exporting is painful, there are pre-exported ONNX speaker verification models on HuggingFace. Search for `ecapa-tdnn onnx` or consider using `wespeaker` models which have official ONNX exports:
- https://github.com/wenet-e2e/wespeaker (has ONNX export support built-in)

### 7.3 Model Download Script

**`scripts/download_models.py`:**
```python
"""
Download required ONNX models.

Usage: python scripts/download_models.py
"""
import urllib.request
import os

MODELS = {
    "silero_vad.onnx": "https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx",
    # ECAPA-TDNN URL depends on chosen source — update after export/finding pre-exported model
}

os.makedirs("models", exist_ok=True)

for name, url in MODELS.items():
    path = f"models/{name}"
    if os.path.exists(path):
        print(f"Already exists: {path}")
        continue
    print(f"Downloading {name}...")
    urllib.request.urlretrieve(url, path)
    print(f"Saved to {path}")
```

---

## 8. Rust Crate Dependencies

**`Cargo.toml`:**
```toml
[package]
name = "voicegate"
version = "0.1.0"
edition = "2021"

[dependencies]
# Audio I/O
cpal = "0.15"                    # Cross-platform audio capture/playback (WASAPI on Windows, ALSA on Linux)
rubato = "0.14"                  # High-quality audio resampling

# ML Inference
ort = { version = "2", features = ["load-dynamic"] }  # ONNX Runtime bindings
ndarray = "0.15"                 # N-dimensional arrays for tensor ops

# Ring buffer
ringbuf = "0.4"                  # Lock-free SPSC ring buffer

# GUI
eframe = "0.28"                  # egui framework
egui = "0.28"

# Audio file handling (for enrollment)
hound = "3.5"                    # WAV reading/writing

# Config
serde = { version = "1", features = ["derive"] }
toml = "0.8"

# Utils
anyhow = "1"                     # Error handling
log = "0.4"
env_logger = "0.11"
dirs = "5"                       # Platform-appropriate config directories (~/.config on Linux, AppData on Windows)
which = "6"                      # Find executables (pw-cli, pactl) on Linux

# Linux: PipeWire bindings (optional, for direct PipeWire API usage)
[target.'cfg(target_os = "linux")'.dependencies]
pipewire = { version = "0.8", optional = true }  # Direct PipeWire API (alternative to shell commands)

[features]
default = []
pipewire-native = ["pipewire"]   # Use PipeWire Rust bindings instead of shell commands

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
```

---

## 9. Build & Run Instructions

### 9.1 Prerequisites

#### Common (both platforms)

```bash
# 1. Install Rust (if not already)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Install Python dependencies (for model export)
pip install speechbrain torch onnx onnxruntime numpy
```

#### Windows-specific

```bash
# 3. Download ONNX Runtime shared library for Windows
# The `ort` crate with `load-dynamic` feature needs onnxruntime.dll
# Download from: https://github.com/microsoft/onnxruntime/releases
# Place onnxruntime.dll in the project root or system PATH

# 4. Install VB-Audio Virtual Cable
# Download from: https://vb-audio.com/Cable/
# Run the installer, reboot
```

#### Ubuntu Linux-specific

```bash
# 3. Install system dependencies
sudo apt update
sudo apt install -y \
    build-essential \
    pkg-config \
    libasound2-dev \
    libpipewire-0.3-dev \
    pipewire \
    pipewire-audio-client-libraries \
    libclang-dev

# 4. Install ONNX Runtime shared library
# Download from: https://github.com/microsoft/onnxruntime/releases
# Extract and copy to /usr/local/lib:
wget https://github.com/microsoft/onnxruntime/releases/download/v1.17.0/onnxruntime-linux-x64-1.17.0.tgz
tar xzf onnxruntime-linux-x64-1.17.0.tgz
sudo cp onnxruntime-linux-x64-1.17.0/lib/libonnxruntime.so* /usr/local/lib/
sudo ldconfig

# 5. Verify PipeWire is running (Ubuntu 22.10+ has it by default)
pw-cli info
# If this fails, you're on PulseAudio — VoiceGate handles both, but PipeWire is preferred.

# 6. No virtual cable install needed — VoiceGate creates its own virtual mic via PipeWire/PulseAudio
```

### 9.2 Model Preparation

```bash
# Download VAD model
python scripts/download_models.py

# Export ECAPA-TDNN to ONNX (or download pre-exported)
python scripts/export_ecapa.py --output models/ecapa_tdnn.onnx
```

### 9.3 Build & Run

```bash
# Development
cargo run

# Release (optimized, much faster inference)
cargo run --release

# With logging
RUST_LOG=debug cargo run --release
```

---

## 10. Testing Strategy

### 10.1 Unit Tests

| Test | What it validates |
|---|---|
| `test_vad_detects_speech` | Silero VAD returns >0.5 for speech WAV file |
| `test_vad_rejects_silence` | Silero VAD returns <0.5 for silence |
| `test_vad_rejects_noise` | Silero VAD returns <0.5 for non-speech noise |
| `test_embedding_consistency` | Same audio → same embedding (cosine sim > 0.99) |
| `test_embedding_discrimination` | Different speakers → different embeddings (cosine sim < 0.5) |
| `test_enrollment_centroid` | Multiple segments average to stable centroid |
| `test_gate_crossfade` | No clicks/pops at gate transitions |
| `test_gate_hold_time` | Gate stays open during brief pauses |
| `test_resampler_quality` | 48k→16k→compare with reference |
| `test_cosine_similarity` | Math correctness with known vectors |

### 10.2 Integration Tests

| Test | What it validates |
|---|---|
| `test_full_pipeline_pass` | Enrolled speaker audio passes through gate |
| `test_full_pipeline_block` | Non-enrolled speaker audio is silenced |
| `test_full_pipeline_mixed` | When both speak, only enrolled voice's segments pass |
| `test_latency` | End-to-end processing < 50ms |

### 10.3 Test Fixtures

Create test WAV files in `tests/fixtures/`:
- `speaker_a.wav` — 10s of speaker A talking (16kHz, mono)
- `speaker_b.wav` — 10s of speaker B talking (16kHz, mono)
- `speaker_a_enroll.wav` — 30s of speaker A for enrollment
- `mixed_ab.wav` — speakers A and B talking over each other
- `silence.wav` — 5s of room noise / silence
- `noise.wav` — 5s of non-speech noise (music, fan, etc.)

You can record these yourself with your brothers to make the tests maximally realistic.

---

## 11. Known Challenges & Mitigations

### 11.1 Overlapping Speech

**Problem:** When you and a brother speak simultaneously, the audio frame contains both voices. The embedding extraction will produce a blended embedding that may not match either speaker cleanly.

**Mitigation options (in order of complexity):**
1. **Accept some bleed-through during overlap.** The gate stays open because your voice IS present, and the brother's voice rides along. This is the pragmatic first approach.
2. **Use a source separation model** (like SepFormer or Conv-TasNet) as a preprocessing step to split the mixed signal before verification. This is much heavier (~100ms+ latency) and should be a v2 feature.
3. **Spectral masking** — compute a time-frequency mask based on the enrolled voice's spectral profile and apply it before output. Lighter than full separation but less effective.

**v1 recommendation:** Accept approach 1. In practice, if the mic is close to your mouth (6-12 inches), your voice will be significantly louder than your brothers'. The embedding extraction will be dominated by the louder voice, which is yours.

### 11.2 Similar-Sounding Siblings

**Problem:** Brothers, especially close in age, may have similar vocal characteristics.

**Mitigation:**
- Implement **negative enrollment** (Section 5.8) — record brothers' voices as anti-targets
- Use a **margin-based decision**: `score = sim(self) - max(sim(brothers))` with a lower threshold
- Allow fine-tuning the threshold per-user via the GUI slider

### 11.3 Voice Variation

**Problem:** Your voice sounds different when whispering, shouting, laughing, or sick.

**Mitigation:**
- During enrollment, encourage the user to speak at various volumes and tones
- Use **multiple enrollment sessions** that are averaged
- Allow re-enrollment if accuracy degrades

### 11.4 Virtual Audio Device

**Windows:** Requires users to install VB-Audio Virtual Cable (free, third-party). This is friction but acceptable for v1. VB-Cable is widely used and takes 30 seconds to install. Document it clearly. A future enhancement could be writing a custom Windows audio driver.

**Linux:** No third-party install needed. VoiceGate creates PipeWire/PulseAudio virtual source nodes automatically at startup and tears them down on exit. This is a much better UX. However, edge cases exist:
- User might be on raw ALSA with no PipeWire or PulseAudio (rare on modern Ubuntu, but possible on minimal installs)
- PipeWire node creation might fail due to permissions or outdated PipeWire versions
- VoiceGate should detect the audio server, try PipeWire first, fall back to PulseAudio, and show a clear error if neither works

### 11.5 CPU Usage

**Problem:** Running ECAPA-TDNN every 200ms may use noticeable CPU.

**Mitigation:**
- Use `ort` with ONNX Runtime's CPU execution provider (optimized with AVX2)
- Only run embedding extraction when VAD is active (saves CPU when nobody is talking)
- Profile and consider using a smaller model (like x-vector instead of ECAPA-TDNN) if CPU is too high
- GPU offload options:
  - **Windows:** DirectML execution provider (works with AMD/Intel/NVIDIA)
  - **Linux:** CUDA execution provider (NVIDIA GPUs) or ROCm (AMD GPUs)
  - Both are optional — CPU-only is the default and works fine on modern hardware

---

## 12. Future Enhancements (v2+)

These are explicitly out of scope for v1 but worth noting:

1. **Source separation preprocessing** — use a lightweight separation model to split mixed audio before verification
2. **Custom Windows virtual audio driver** — eliminate VB-Cable dependency on Windows
3. **Multi-profile support** — each brother installs VoiceGate with their own profile
4. **System tray mode** — minimize to tray with status indicator (Windows tray / Linux AppIndicator)
5. **Auto-threshold calibration** — during enrollment, also record brothers' voices and automatically set the optimal threshold
6. **Per-application routing** — route different apps through different profiles
7. **macOS support** — add CoreAudio backend and virtual device creation via `BlackHole` or `AudioServerPlugin`
8. **Installer/MSI package** — proper Windows installer with bundled VB-Cable; `.deb`/`.AppImage` for Linux
9. **Noise suppression chaining** — apply RNNoise after voice isolation for double cleanup
10. **WebRTC integration** — expose as a WebRTC-compatible audio processor for browser-based voice chat
11. **Flatpak/Snap packaging** — for easy Linux distribution (note: PipeWire access may need portal permissions)

---

## 13. Success Criteria

The tool is considered working when:

1. ✅ Enrollment completes in <60 seconds
2. ✅ The enrolled user's voice passes through to Discord clearly
3. ✅ Other voices in the same room are silenced >90% of the time
4. ✅ End-to-end latency is <50ms (imperceptible in conversation)
5. ✅ CPU usage is <10% on a mid-range machine during active use
6. ✅ No audible clicks or artifacts at gate transitions
7. ✅ The tool runs stably for hours without crashes or memory leaks
8. ✅ The similarity threshold is tunable enough to handle siblings with similar voices
9. ✅ Works on both Windows 10/11 and Ubuntu 22.04+ with the same binary codebase
10. ✅ Linux: virtual mic is created/destroyed automatically (zero extra software install)

---

## 14. Quick-Start for Claude Code

When using Claude Code to build this project, here's the recommended order:

### Phase 1: Audio Foundation
1. Set up Cargo project with dependencies
2. Implement `audio/capture.rs` — list devices, capture from selected mic (works on both platforms via cpal)
3. Implement `audio/virtual_mic.rs` — platform abstraction:
   - Windows: detect VB-Cable, error if missing with install link
   - Linux: detect PipeWire vs PulseAudio, create virtual source nodes
4. Implement `audio/output.rs` — output to virtual mic device
5. Implement `audio/ring_buffer.rs` — lock-free SPSC
6. Test: audio passthrough (mic → virtual mic with no processing) on both Windows and Linux

### Phase 2: ML Pipeline
6. Download Silero VAD ONNX model
7. Implement `ml/vad.rs` — load model, run inference
8. Implement `audio/resampler.rs` — 48kHz ↔ 16kHz
9. Export/download ECAPA-TDNN ONNX model
10. Implement `ml/embedding.rs` — load model, extract embeddings
11. Implement `ml/similarity.rs` — cosine similarity + EMA
12. Test: run pipeline on WAV files, verify embeddings make sense

### Phase 3: Enrollment
13. Implement `enrollment/enroll.rs` — recording + processing
14. Implement profile save/load
15. Test: enroll, then verify live audio matches

### Phase 4: Gate + Integration
16. Implement `gate/gate.rs` — gate logic with crossfade
17. Wire everything together in the processing thread
18. Implement `config/settings.rs` — TOML config
19. Test: full pipeline end-to-end

### Phase 5: GUI
20. Implement `gui/app.rs` — device selection, threshold slider, similarity meter
21. Add enrollment wizard UI
22. Add bypass toggle
23. Polish and release build

---

## Appendix A: Enrollment Passage

Use this text for voice enrollment (covers many English phonemes):

> The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs. How vexingly quick daft zebras jump. The five boxing wizards jump quickly. Sphinx of black quartz, judge my vow. Two driven jocks help fax my big quiz. The jay, pig, fox, zebra and my wolves quack. Crazy Frederick bought many very exquisite opal jewels. We promptly judged antique ivory buckles for the next prize. A mad boxer shot a quick, gloved jab to the jaw of his dizzy opponent.

## Appendix B: Useful References

- **cpal docs:** https://docs.rs/cpal/latest/cpal/
- **ort (ONNX Runtime for Rust):** https://docs.rs/ort/latest/ort/
- **Silero VAD:** https://github.com/snakers4/silero-vad
- **SpeechBrain ECAPA-TDNN:** https://huggingface.co/speechbrain/spkrec-ecapa-voxceleb
- **WeSpeaker (alternative, has ONNX):** https://github.com/wenet-e2e/wespeaker
- **rubato resampler:** https://docs.rs/rubato/latest/rubato/
- **VB-Audio Virtual Cable (Windows):** https://vb-audio.com/Cable/
- **PipeWire docs:** https://docs.pipewire.org/
- **pipewire-rs (Rust bindings):** https://gitlab.freedesktop.org/pipewire/pipewire-rs
- **PipeWire virtual devices wiki:** https://gitlab.freedesktop.org/pipewire/pipewire/-/wikis/Virtual-Devices
- **egui:** https://docs.rs/egui/latest/egui/
- **ringbuf crate:** https://docs.rs/ringbuf/latest/ringbuf/

## Appendix C: Linux Virtual Mic — Quick Reference

**PipeWire (preferred, Ubuntu 22.10+):**
```bash
# Create a null sink that VoiceGate writes to
pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=voicegate_sink node.description="VoiceGate Sink" media.class=Audio/Sink audio.position=MONO audio.rate=48000 }'

# Create a virtual source that Discord reads from
pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=voicegate_mic node.description="VoiceGate Virtual Microphone" media.class=Audio/Source/Virtual audio.position=MONO audio.rate=48000 }'

# Link sink monitor to virtual source input
pw-link voicegate_sink:monitor_MONO voicegate_mic:input_MONO

# Verify — should show both nodes
pw-cli list-objects | grep voicegate

# Cleanup on exit
pw-cli destroy-node voicegate_sink
pw-cli destroy-node voicegate_mic
```

**PulseAudio fallback (Ubuntu 22.04 without PipeWire):**
```bash
# Create null sink + remap source
pactl load-module module-null-sink sink_name=voicegate_sink sink_properties=device.description="VoiceGate_Sink" rate=48000 channels=1 format=float32le
pactl load-module module-remap-source master=voicegate_sink.monitor source_name=voicegate_mic source_properties=device.description="VoiceGate_Virtual_Microphone"

# Verify
pactl list sources short | grep voicegate

# Cleanup — unload by module index (returned by load-module)
pactl unload-module <index>
```

**Discord setup on Linux:**
1. Open Discord → Settings → Voice & Video
2. Input Device → select "VoiceGate Virtual Microphone"
3. That's it — VoiceGate handles the rest
