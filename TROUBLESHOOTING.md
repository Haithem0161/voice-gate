# VoiceGate Troubleshooting

## Linux

### "ALSA function 'snd_pcm_hw_params' failed with error 'I/O error (5)'"

The default ALSA device is rejecting the audio format request. This commonly
happens on PipeWire systems where the ALSA compatibility shim does not honor
all reported capabilities.

**Fix:** Set your input device to a `plughw` device in the config file:

```toml
# ~/.config/voicegate/config.toml
[audio]
input_device = "plughw:CARD=PCH,DEV=0"
```

Run `voicegate devices` to find the correct `plughw` device name for your
hardware. The `plughw` prefix routes through ALSA's plugin layer, which
handles format and rate conversion transparently.

### "pw-loopback not found on PATH"

VoiceGate requires PipeWire for the virtual microphone on Linux.

```bash
sudo apt install pipewire pipewire-audio-client-libraries
```

### "pactl not found"

If your system uses PulseAudio instead of PipeWire:

```bash
sudo apt install pulseaudio-utils
```

### PipeWire nodes not cleaned up after a crash

If VoiceGate is killed with SIGKILL (not SIGTERM or Ctrl-C), the
`pw-loopback` child process may survive. Check and clean up:

```bash
pgrep -a pw-loopback | grep voicegate
# If found:
pkill -f "pw-loopback.*voicegate"
```

### "output device 'voicegate_sink' not found"

VoiceGate creates a PipeWire virtual sink via `pw-loopback`. cpal's ALSA
backend cannot see PipeWire nodes by name, so VoiceGate routes through the
`PIPEWIRE_NODE` environment variable automatically. If this still fails,
check that PipeWire is running:

```bash
pw-cli info 0
```

## Windows

### "CABLE Input (VB-Audio Virtual Cable) not found"

VoiceGate requires VB-Audio Virtual Cable on Windows.

1. Download from https://vb-audio.com/Cable/
2. Install and reboot
3. Re-launch VoiceGate

### Discord does not see VoiceGate microphone

In Discord settings, set your input device to "CABLE Output (VB-Audio
Virtual Cable)". VoiceGate writes to "CABLE Input" (which is the output
side from VoiceGate's perspective).

## General

### "model not found"

Run `make models` to download the required ONNX models, or set the
`VOICEGATE_MODELS_DIR` environment variable to the directory containing
`silero_vad.onnx` and `wespeaker_resnet34_lm.onnx`.

### "ONNX Runtime is not available"

Install ONNX Runtime 1.22.x:

**Linux:**
```bash
wget https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/onnxruntime-linux-x64-1.22.0.tgz
tar xzf onnxruntime-linux-x64-1.22.0.tgz
sudo cp onnxruntime-linux-x64-1.22.0/lib/libonnxruntime.so* /usr/local/lib/
sudo ldconfig
```

**Windows:**
Download `onnxruntime.dll` from the same release page and place it next
to `voicegate.exe` or on your `%PATH%`.

### Profile errors after upgrading

VoiceGate v2 profiles include anti-target data. The v2 loader reads both
v1 and v2 profiles. However, a v2 profile cannot be read by an older v1
build. If you downgrade, re-enroll.

### Run `voicegate doctor`

The `doctor` subcommand prints a full diagnostic report of your
environment: audio server, models, profile, config, and devices.
