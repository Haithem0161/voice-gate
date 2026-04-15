//! cpal output stream wiring.
//!
//! Opens a 48 kHz mono f32 output stream on a named device (typically the
//! virtual-mic sink: `voicegate_sink` on Linux or
//! `CABLE Input (VB-Audio Virtual Cable)` on Windows). The stream pops
//! samples from an SPSC ring buffer consumer and writes them to the device.
//!
//! Real-time safety: the cpal callback does NOT allocate, log, or lock.
//! Only `pop_slice` on the ring buffer consumer, plus zero-fill if the
//! ring is empty (silence underflow).

use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleRate, Stream, StreamConfig};
use ringbuf::traits::Consumer;

use crate::audio::ring_buffer::AudioConsumer;

const TARGET_SAMPLE_RATE: u32 = 48_000;
const STREAM_TIMEOUT: Option<Duration> = None;

/// Stack-buffer size for mono-to-stereo upmix when the output device does not
/// natively support mono. Sized to cover any realistic single-callback chunk.
const UPMIX_CHUNK_FRAMES: usize = 16_384;

/// Owning handle to a running cpal output stream.
pub struct OutputStream {
    _stream: Stream,
    pub device_name: String,
}

/// Start a 48 kHz mono f32 output stream on `device_name`, popping samples
/// from `consumer`. If the named device only supports stereo, the callback
/// duplicates the mono stream into both channels.
///
/// On Linux, if `device_name` is a PipeWire node name (e.g. `voicegate_sink`)
/// that cpal's ALSA backend cannot enumerate directly, we set the
/// `PIPEWIRE_NODE` env var to route the `pipewire` ALSA device to the
/// target node. This resolves G-016.
pub fn start_output(device_name: &str, consumer: AudioConsumer) -> anyhow::Result<OutputStream> {
    let host = cpal::default_host();

    // G-016 fix: on Linux, PipeWire nodes are not visible as ALSA device
    // names. If the requested name is not in cpal's device list, try
    // routing via the PIPEWIRE_NODE env var + the "pipewire" ALSA device.
    let (device, actual_name) = match find_output_device(&host, device_name) {
        Ok(dev) => {
            let name = dev.name().unwrap_or_else(|_| device_name.to_string());
            (dev, name)
        }
        Err(_) => {
            #[cfg(target_os = "linux")]
            {
                tracing::info!(
                    requested = %device_name,
                    "output device not found by name, trying PIPEWIRE_NODE routing"
                );
                // Set PIPEWIRE_NODE temporarily -- will be removed after
                // the stream is built so it doesn't affect capture routing.
                std::env::set_var("PIPEWIRE_NODE", device_name);
                let dev = find_output_device(&host, "pipewire").map_err(|_| {
                    anyhow::anyhow!(
                        "output device {:?} not found, and 'pipewire' ALSA device also \
                         unavailable. Is PipeWire running?",
                        device_name
                    )
                })?;
                let name = dev.name().unwrap_or_else(|_| "pipewire".to_string());
                (dev, name)
            }
            #[cfg(not(target_os = "linux"))]
            {
                anyhow::bail!("output device {:?} not found", device_name);
            }
        }
    };

    let resolved_name = actual_name;

    let supported = device
        .default_output_config()
        .map_err(|e| anyhow::anyhow!("default_output_config on {}: {}", resolved_name, e))?;

    let host_channels = supported.channels();
    let stereo = host_channels >= 2;
    tracing::info!(
        device = %resolved_name,
        host_channels,
        "opening output stream"
    );

    // See the analogous comment in capture.rs: BufferSize::Default is
    // the only viable choice on modern Linux (PipeWire's ALSA layer rejects
    // Fixed requests) and the ring buffer absorbs variable callback sizes.
    let cfg = StreamConfig {
        channels: host_channels,
        sample_rate: SampleRate(TARGET_SAMPLE_RATE),
        buffer_size: BufferSize::Default,
    };

    let stream = build_output_stream_with_upmix(&device, &cfg, stereo, consumer)?;
    stream
        .play()
        .map_err(|e| anyhow::anyhow!("output stream play: {e}"))?;

    // Clear PIPEWIRE_NODE so it doesn't affect subsequent capture streams.
    #[cfg(target_os = "linux")]
    std::env::remove_var("PIPEWIRE_NODE");

    Ok(OutputStream {
        _stream: stream,
        device_name: resolved_name,
    })
}

/// List cpal output devices and return their names, with the default marked.
pub fn list_output_devices() -> anyhow::Result<Vec<(String, bool)>> {
    let host = cpal::default_host();
    let default = host.default_output_device().and_then(|d| d.name().ok());

    let mut out = Vec::new();
    for device in host
        .output_devices()
        .map_err(|e| anyhow::anyhow!("output_devices: {e}"))?
    {
        if let Ok(name) = device.name() {
            let is_default = default.as_deref() == Some(name.as_str());
            out.push((name, is_default));
        }
    }
    Ok(out)
}

fn find_output_device(host: &cpal::Host, name: &str) -> anyhow::Result<cpal::Device> {
    for device in host
        .output_devices()
        .map_err(|e| anyhow::anyhow!("output_devices: {e}"))?
    {
        if let Ok(dev_name) = device.name() {
            if dev_name == name {
                return Ok(device);
            }
        }
    }
    anyhow::bail!("output device {:?} not found", name)
}

fn build_output_stream_with_upmix(
    device: &cpal::Device,
    cfg: &StreamConfig,
    stereo: bool,
    mut consumer: AudioConsumer,
) -> anyhow::Result<Stream> {
    let data_callback = move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
        if !stereo {
            let n = consumer.pop_slice(data);
            // Zero-fill the tail if the ring is under-full, to keep the
            // output from glitching with stale buffer contents.
            for sample in &mut data[n..] {
                *sample = 0.0;
            }
            return;
        }

        // Stereo output: pop half the frame count into a mono scratch, then
        // duplicate into L/R in the destination buffer.
        let frame_count = data.len() / 2;
        let mut processed = 0;
        while processed < frame_count {
            let chunk = (frame_count - processed).min(UPMIX_CHUNK_FRAMES);
            let mut scratch = [0.0f32; UPMIX_CHUNK_FRAMES];
            let n = consumer.pop_slice(&mut scratch[..chunk]);
            let (dst_chunk, _) = data[processed * 2..].split_at_mut(chunk * 2);
            for (pair, src) in dst_chunk.chunks_exact_mut(2).zip(scratch[..n].iter()) {
                pair[0] = *src;
                pair[1] = *src;
            }
            // Zero-fill any tail that could not be filled from the ring.
            for pair in dst_chunk.chunks_exact_mut(2).skip(n) {
                pair[0] = 0.0;
                pair[1] = 0.0;
            }
            processed += chunk;
        }
    };

    let error_callback = |err: cpal::StreamError| {
        tracing::error!(%err, "output stream error");
    };

    device
        .build_output_stream::<f32, _, _>(cfg, data_callback, error_callback, STREAM_TIMEOUT)
        .map_err(|e| anyhow::anyhow!("build_output_stream: {e}"))
}
