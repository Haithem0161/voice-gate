//! cpal input stream wiring.
//!
//! Opens a 48 kHz mono f32 input stream and pushes samples into an SPSC
//! ring buffer. Downmixes stereo-to-mono in the callback if the device
//! does not support mono natively. Handles the ALSA variable-callback
//! case by letting the worker pop fixed-size chunks from the ring.
//!
//! Real-time safety: the cpal callback does NOT allocate, log, or lock.
//! Only `push_slice` on the ring buffer producer and (if stereo) an
//! inline downmix into a stack buffer. See `.claude/rules/audio-io.md`.

use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleRate, Stream, StreamConfig};
use ringbuf::traits::Producer;

use crate::audio::ring_buffer::AudioProducer;

const TARGET_SAMPLE_RATE: u32 = 48_000;
const STREAM_TIMEOUT: Option<Duration> = None;

/// Stack-buffer size for stereo-to-mono downmix. Caps the worst-case ALSA
/// chunk we will downmix in a single callback invocation. Anything larger
/// is processed in multiple chunks.
const DOWNMIX_CHUNK_FRAMES: usize = 16_384;

/// Owning handle to a running cpal input stream.
///
/// Keep this alive for as long as you want the callback to fire. Dropping
/// the handle stops the stream.
pub struct CaptureStream {
    _stream: Stream,
    pub device_name: String,
    pub sample_rate: u32,
}

/// Start a 48 kHz mono f32 capture stream on the named device (or the default
/// input device if `device_name` is `None` or `"default"`). The stream pushes
/// samples into `producer`.
pub fn start_capture(
    device_name: Option<&str>,
    producer: AudioProducer,
) -> anyhow::Result<CaptureStream> {
    let host = cpal::default_host();
    let device = select_input_device(&host, device_name)?;
    let resolved_name = device.name().unwrap_or_else(|_| "<unknown>".to_string());

    let supported = device
        .default_input_config()
        .map_err(|e| anyhow::anyhow!("default_input_config on {}: {}", resolved_name, e))?;

    let default_channels = supported.channels();
    let default_rate = supported.sample_rate().0;
    tracing::info!(
        device = %resolved_name,
        default_channels,
        default_rate,
        default_format = ?supported.sample_format(),
        "device default config"
    );

    // VoiceGate's entire pipeline is pinned to 48 000 Hz mono f32 (see
    // Decision D-001). If the device's default is not 48 kHz, search
    // supported_input_configs for a 48 kHz variant. If none exists, error
    // out with a clear message -- Phase 1 does not do capture-side rate
    // conversion (rubato pre-capture resampling is an explicit v1.1 non-goal).
    let host_channels = if default_rate == TARGET_SAMPLE_RATE
        && supported.sample_format() == cpal::SampleFormat::F32
    {
        default_channels
    } else {
        find_48khz_f32_channels(&device).ok_or_else(|| {
            anyhow::anyhow!(
                "device {:?} does not support 48 000 Hz f32 capture. \
                 VoiceGate v1 requires a 48 kHz f32 microphone. \
                 Default config was {} Hz / {:?}.",
                resolved_name,
                default_rate,
                supported.sample_format()
            )
        })?
    };
    let stereo = host_channels >= 2;
    tracing::info!(
        device = %resolved_name,
        host_channels,
        "opening input stream at 48000 Hz"
    );

    // Buffer size: always BufferSize::Default. Rationale:
    //
    // 1. PipeWire's ALSA compatibility layer (which is what "default" resolves
    //    to on every modern Linux setup) rejects BufferSize::Fixed(1536) at
    //    snd_pcm_hw_params time, even when supported_input_configs() reports
    //    a range that includes 1536. This was observed on PipeWire 1.0.5 with
    //    a fatal I/O error (errno 5). Native ALSA without PipeWire would honor
    //    Fixed in most cases, but that is the vanishing case in 2026.
    // 2. The entire reason the capture path feeds a ringbuf SPSC queue instead
    //    of handing frames directly to the worker is to absorb variable
    //    callback chunk sizes. The worker pops exactly 1536-sample frames from
    //    the ring. See phase-01 section 4.4 and .claude/rules/audio-io.md.
    // 3. Retrying build_input_stream with a different BufferSize is impractical
    //    because the callback closure is consumed by the first attempt.
    let cfg = StreamConfig {
        channels: host_channels,
        sample_rate: SampleRate(TARGET_SAMPLE_RATE),
        buffer_size: BufferSize::Default,
    };

    let stream = build_input_stream_with_downmix(&device, &cfg, stereo, producer)?;
    stream
        .play()
        .map_err(|e| anyhow::anyhow!("input stream play: {e}"))?;

    Ok(CaptureStream {
        _stream: stream,
        device_name: resolved_name,
        sample_rate: TARGET_SAMPLE_RATE,
    })
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

/// Scan `supported_input_configs()` for any f32 config that supports 48 000 Hz
/// capture. Returns the channel count of the first matching config, or `None`
/// if no 48 kHz f32 variant exists. Prefers mono (1 channel) over stereo
/// because it saves the downmix step in the callback.
///
/// VoiceGate pins the internal audio format to `f32` and never converts at
/// capture time -- that is the frame-size / D-001 contract. Devices that
/// only offer i16 at 48 kHz are therefore not usable by Phase 1. A pre-capture
/// format conversion would be a Phase 2+ enhancement.
fn find_48khz_f32_channels(device: &cpal::Device) -> Option<u16> {
    let configs = device.supported_input_configs().ok()?;
    let matching: Vec<u16> = configs
        .filter(|cfg| {
            cfg.min_sample_rate().0 <= TARGET_SAMPLE_RATE
                && TARGET_SAMPLE_RATE <= cfg.max_sample_rate().0
                && cfg.sample_format() == cpal::SampleFormat::F32
        })
        .map(|cfg| cfg.channels())
        .collect();

    if matching.is_empty() {
        return None;
    }
    matching
        .iter()
        .find(|&&ch| ch == 1)
        .copied()
        .or_else(|| matching.first().copied())
}

fn build_input_stream_with_downmix(
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
        // Stereo -> mono, processed in stack-bounded chunks.
        let frame_count = data.len() / 2;
        let mut scratch = [0.0f32; DOWNMIX_CHUNK_FRAMES];
        let mut processed = 0;
        while processed < frame_count {
            let chunk = (frame_count - processed).min(DOWNMIX_CHUNK_FRAMES);
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
        .build_input_stream::<f32, _, _>(cfg, data_callback, error_callback, STREAM_TIMEOUT)
        .map_err(|e| anyhow::anyhow!("build_input_stream: {e}"))
}
