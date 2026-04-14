//! cpal output stream wiring.
//!
//! Phase 1 stub. Real wiring lands in step 7.

use crate::audio::ring_buffer::AudioConsumer;

/// Owning handle to a running cpal output stream.
pub struct OutputStream {
    _stream: Option<cpal::Stream>,
    pub device_name: String,
}

/// Start a 48 kHz mono f32 output stream on the named device. The stream pops
/// samples from `consumer` and writes them to the device. The device is
/// expected to be the virtual-mic sink (`voicegate_sink` on Linux,
/// `CABLE Input (VB-Audio Virtual Cable)` on Windows).
pub fn start_output(_device_name: &str, _consumer: AudioConsumer) -> anyhow::Result<OutputStream> {
    anyhow::bail!("start_output not yet implemented (Phase 1, step 7)")
}
