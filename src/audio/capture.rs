//! cpal input stream wiring.
//!
//! Opens a capture stream and pushes mono f32 samples into an SPSC ring
//! buffer. Handles multiple device formats:
//!   - 48 kHz f32 (ideal, direct push)
//!   - Other rates (e.g. 44.1 kHz): accepted, caller resamples via
//!     CaptureResampler
//!   - i16 format (converted to f32 in the callback)
//!   - stereo (downmixed to mono in the callback)
//!
//! Real-time safety: the cpal callback does NOT allocate, log, or lock.
//! Only `push_slice` on the ring buffer producer and inline format
//! conversion. See `.claude/rules/audio-io.md`.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleRate, Stream, StreamConfig};
use ringbuf::traits::Producer;

use crate::audio::ring_buffer::AudioProducer;

/// Stack-buffer size for stereo-to-mono downmix and i16-to-f32 conversion.
const CONVERT_CHUNK_FRAMES: usize = 16_384;

/// Owning handle to a running cpal input stream.
pub struct CaptureStream {
    _stream: Stream,
    pub device_name: String,
    /// Actual negotiated sample rate. May differ from 48000 if the device
    /// only supports e.g. 44100. The caller must resample if this != 48000.
    pub sample_rate: u32,
}

/// Start a capture stream on the named device (or the default input device).
/// Tries 48 kHz f32 first; falls back to the device's default config with
/// in-callback format conversion. Returns the actual negotiated sample rate
/// in `CaptureStream::sample_rate`.
pub fn start_capture(
    device_name: Option<&str>,
    producer: AudioProducer,
) -> anyhow::Result<CaptureStream> {
    // On Linux with PipeWire, use the "pipewire" ALSA device AND set
    // PIPEWIRE_NODE to the default audio source. Without PIPEWIRE_NODE,
    // the pipewire ALSA plugin routes to the hardware card, not the
    // PipeWire default source (which may be a Bluetooth mic).
    let effective_name = device_name;
    #[cfg(target_os = "linux")]
    let _pipewire_override;
    #[cfg(target_os = "linux")]
    let effective_name = {
        use crate::audio::audio_server::{detect_audio_server, AudioServer};
        let requested = device_name.unwrap_or("default");
        if (requested == "default" || requested == "pipewire")
            && detect_audio_server() == AudioServer::PipeWire
        {
            // Find the default source node name and set PIPEWIRE_NODE
            if let Ok(output) = std::process::Command::new("wpctl")
                .args(["inspect", "@DEFAULT_AUDIO_SOURCE@"])
                .output()
            {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed.strip_prefix("node.name") {
                        if let Some(name) = rest.split('=').nth(1) {
                            let name = name.trim().trim_matches('"').trim();
                            tracing::info!(
                                pipewire_node = %name,
                                "routing capture to PipeWire default source"
                            );
                            std::env::set_var("PIPEWIRE_NODE", name);
                        }
                        break;
                    }
                }
            }
            _pipewire_override = "pipewire".to_string();
            Some(_pipewire_override.as_str())
        } else {
            effective_name
        }
    };

    let host = cpal::default_host();
    let device = select_input_device(&host, effective_name)?;
    let resolved_name = device.name().unwrap_or_else(|_| "<unknown>".to_string());

    let supported = device
        .default_input_config()
        .map_err(|e| anyhow::anyhow!("default_input_config on {}: {}", resolved_name, e))?;

    let default_channels = supported.channels();
    let default_rate = supported.sample_rate().0;
    let default_format = supported.sample_format();
    tracing::info!(
        device = %resolved_name,
        default_channels,
        default_rate,
        default_format = ?default_format,
        "device default config"
    );

    let (use_rate, mut use_format, use_channels) =
        pick_best_config(&device, default_rate, default_format, default_channels);

    // PipeWire's ALSA plugin reports F32 support but produces garbage data
    // when capturing in F32 format. Force I16 for PipeWire-routed devices.
    if resolved_name == "pipewire" && use_format == cpal::SampleFormat::F32 {
        tracing::info!("forcing I16 for pipewire ALSA device (F32 capture is broken)");
        use_format = cpal::SampleFormat::I16;
    }

    let stereo = use_channels >= 2;
    tracing::info!(
        device = %resolved_name,
        rate = use_rate,
        format = ?use_format,
        channels = use_channels,
        "opening input stream"
    );

    let cfg = StreamConfig {
        channels: use_channels,
        sample_rate: SampleRate(use_rate),
        buffer_size: BufferSize::Default,
    };

    let stream = match use_format {
        cpal::SampleFormat::F32 => build_f32_stream(&device, &cfg, stereo, producer)?,
        cpal::SampleFormat::I16 => build_i16_stream(&device, &cfg, stereo, producer)?,
        other => {
            anyhow::bail!(
                "device {:?} format {:?} is not supported. VoiceGate handles f32 and i16.",
                resolved_name,
                other
            );
        }
    };

    stream
        .play()
        .map_err(|e| anyhow::anyhow!("input stream play: {e}"))?;

    // Clear PIPEWIRE_NODE after capture is established so it doesn't
    // interfere with the output stream opening later.
    #[cfg(target_os = "linux")]
    std::env::remove_var("PIPEWIRE_NODE");

    Ok(CaptureStream {
        _stream: stream,
        device_name: resolved_name,
        sample_rate: use_rate,
    })
}

/// Pick the best input config. If the device defaults to 48kHz, use it.
/// Otherwise use the device's default rate -- do NOT trust
/// supported_input_configs claiming 48kHz is available, because
/// PipeWire's ALSA compatibility shim reports 48kHz in the supported
/// range but rejects it at snd_pcm_hw_params time (G-015).
fn pick_best_config(
    _device: &cpal::Device,
    default_rate: u32,
    default_format: cpal::SampleFormat,
    default_channels: u16,
) -> (u32, cpal::SampleFormat, u16) {
    // Use the device's default config. The caller will resample if
    // the rate is not 48000. The format may be f32 or i16; both are
    // handled by the stream builder.
    let format = match default_format {
        cpal::SampleFormat::F32 | cpal::SampleFormat::I16 => default_format,
        // If the default is some other format, try f32
        _ => cpal::SampleFormat::F32,
    };
    (default_rate, format, default_channels)
}

/// List cpal input devices and return their names, with the default marked.
pub fn list_input_devices() -> anyhow::Result<Vec<(String, bool)>> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.name().ok());

    let mut out = Vec::new();
    for device in host
        .input_devices()
        .map_err(|e| anyhow::anyhow!("input_devices: {e}"))?
    {
        if let Ok(name) = device.name() {
            let is_default = default.as_deref() == Some(name.as_str());
            out.push((name, is_default));
        }
    }
    Ok(out)
}

fn select_input_device(host: &cpal::Host, requested: Option<&str>) -> anyhow::Result<cpal::Device> {
    let requested = requested.filter(|s| !s.is_empty() && *s != "default");

    if let Some(name) = requested {
        for device in host
            .input_devices()
            .map_err(|e| anyhow::anyhow!("input_devices: {e}"))?
        {
            if let Ok(dev_name) = device.name() {
                if dev_name == name {
                    return Ok(device);
                }
            }
        }
        tracing::warn!(
            requested = %name,
            "requested input device not found, falling back to default"
        );
    }

    host.default_input_device()
        .ok_or_else(|| anyhow::anyhow!("no default input device available"))
}

fn build_f32_stream(
    device: &cpal::Device,
    cfg: &StreamConfig,
    stereo: bool,
    mut producer: AudioProducer,
) -> anyhow::Result<Stream> {
    let data_callback = move |data: &[f32], _info: &cpal::InputCallbackInfo| {
        if !stereo {
            producer.push_slice(data);
            return;
        }
        let frame_count = data.len() / 2;
        let mut scratch = [0.0f32; CONVERT_CHUNK_FRAMES];
        let mut processed = 0;
        while processed < frame_count {
            let chunk = (frame_count - processed).min(CONVERT_CHUNK_FRAMES);
            for (out, pair) in scratch[..chunk]
                .iter_mut()
                .zip(data[processed * 2..].chunks_exact(2))
            {
                *out = 0.5 * (pair[0] + pair[1]);
            }
            producer.push_slice(&scratch[..chunk]);
            processed += chunk;
        }
    };

    let error_callback = |err: cpal::StreamError| {
        tracing::error!(%err, "input stream error");
    };

    device
        .build_input_stream::<f32, _, _>(cfg, data_callback, error_callback, None)
        .map_err(|e| anyhow::anyhow!("build_input_stream f32: {e}"))
}

fn build_i16_stream(
    device: &cpal::Device,
    cfg: &StreamConfig,
    stereo: bool,
    mut producer: AudioProducer,
) -> anyhow::Result<Stream> {
    let data_callback = move |data: &[i16], _info: &cpal::InputCallbackInfo| {
        let mut scratch = [0.0f32; CONVERT_CHUNK_FRAMES];
        if !stereo {
            let mut processed = 0;
            while processed < data.len() {
                let chunk = (data.len() - processed).min(CONVERT_CHUNK_FRAMES);
                for (out, &sample) in scratch[..chunk]
                    .iter_mut()
                    .zip(&data[processed..processed + chunk])
                {
                    *out = sample as f32 / i16::MAX as f32;
                }
                producer.push_slice(&scratch[..chunk]);
                processed += chunk;
            }
        } else {
            let frame_count = data.len() / 2;
            let mut processed = 0;
            while processed < frame_count {
                let chunk = (frame_count - processed).min(CONVERT_CHUNK_FRAMES);
                for (out, pair) in scratch[..chunk]
                    .iter_mut()
                    .zip(data[processed * 2..].chunks_exact(2))
                {
                    let l = pair[0] as f32 / i16::MAX as f32;
                    let r = pair[1] as f32 / i16::MAX as f32;
                    *out = 0.5 * (l + r);
                }
                producer.push_slice(&scratch[..chunk]);
                processed += chunk;
            }
        }
    };

    let error_callback = |err: cpal::StreamError| {
        tracing::error!(%err, "input stream error (i16)");
    };

    device
        .build_input_stream::<i16, _, _>(cfg, data_callback, error_callback, None)
        .map_err(|e| anyhow::anyhow!("build_input_stream i16: {e}"))
}
