//! Rubato-based 48 kHz -> 16 kHz resampler.
//!
//! Phase 1 stub. The real `FftFixedIn::<f32>::new(48_000, 16_000, 1536, 1, 1)`
//! wiring lands in Phase 2 when Silero VAD requires 16 kHz input.

/// Placeholder resampler. Fleshed out in Phase 2.
pub struct Resampler48to16;

impl Resampler48to16 {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Resampler48to16 {
    fn default() -> Self {
        Self::new()
    }
}
