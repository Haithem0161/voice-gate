//! cpal input stream wiring.
//!
//! Phase 1 ships only the type surface and a stub `start_capture` that errors
//! with "not yet implemented". The real cpal wiring lands in step 7 of the
//! Phase 1 morph once the scaffold compiles clean.

use crate::audio::ring_buffer::AudioProducer;

/// Owning handle to a running cpal input stream. Dropping it stops the stream.
pub struct CaptureStream {
    /// Kept alive so the cpal callback keeps firing.
    _stream: Option<cpal::Stream>,
    pub device_name: String,
    pub sample_rate: u32,
}

/// Start a 48 kHz mono f32 capture stream on the named device (or the default
/// input device if `None`). The stream pushes 32 ms frames into `producer`.
///
/// See `.claude/rules/audio-io.md` for the frame-size contract and the
/// ALSA variable-callback handling rules.
pub fn start_capture(
    _device_name: Option<&str>,
    _producer: AudioProducer,
) -> anyhow::Result<CaptureStream> {
    anyhow::bail!("start_capture not yet implemented (Phase 1, step 7)")
}
