//! 48 kHz -> 16 kHz resampler for VoiceGate's ML pipeline.
//!
//! Wraps `rubato::FftFixedIn<f32>` configured for fixed 1536-sample input
//! blocks (one 32 ms frame at 48 kHz, per Decision D-001 in
//! `docs/voicegate/research.md`). Output is variable-length but averages
//! to exactly 1/3 of input in steady state (512 samples per 1536 input).
//!
//! Due to rubato's internal overlap-save buffering, a single call to
//! `process_block` may return 0, 256, or 512 samples. Across many frames
//! the total output length equals `input_samples * 16000 / 48000` to
//! within a one-sample rounding. Callers that need fixed 512-sample
//! chunks (e.g. Silero VAD) must accumulate the variable output and drain
//! 512 at a time. See `.claude/rules/audio-io.md`.

use rubato::{FftFixedIn, Resampler};

use crate::VoiceGateError;

/// Input sample rate (48 kHz, the cpal capture format).
pub const INPUT_SAMPLE_RATE: usize = 48_000;

/// Output sample rate (16 kHz, Silero VAD's required input).
pub const OUTPUT_SAMPLE_RATE: usize = 16_000;

/// Fixed input chunk size per call (32 ms at 48 kHz, per D-001).
pub const INPUT_CHUNK_SAMPLES: usize = 1536;

/// Number of sub-chunks rubato uses internally. 2 sub-chunks of 768 samples
/// each is the value used by WeSpeaker/Silero-tuned pipelines in the wild.
const SUB_CHUNKS: usize = 2;

/// Maximum number of output samples the resampler may produce in a single
/// `process_block` call. Per rubato's internal math for our config
/// (gcd(48000, 16000)=16000, fft_size_in=768, fft_size_out=256, sub_chunks=2),
/// this is 2 * 256 = 512.
pub const MAX_OUTPUT_SAMPLES: usize = 512;

pub struct Resampler48to16 {
    inner: FftFixedIn<f32>,
    /// Pre-allocated output buffer, reused across calls. Size is
    /// MAX_OUTPUT_SAMPLES so we never reallocate. `process_block` returns
    /// a slice of the actually-written portion.
    output_scratch: Vec<Vec<f32>>,
}

impl Resampler48to16 {
    /// Construct a 48 -> 16 kHz mono resampler sized for 1536-sample input
    /// chunks.
    pub fn new() -> anyhow::Result<Self> {
        let inner = FftFixedIn::<f32>::new(
            INPUT_SAMPLE_RATE,
            OUTPUT_SAMPLE_RATE,
            INPUT_CHUNK_SAMPLES,
            SUB_CHUNKS,
            1, // mono
        )
        .map_err(|e| VoiceGateError::Audio(format!("rubato FftFixedIn::new: {e}")))?;

        // rubato output is per-channel; for mono that's a single Vec of size
        // MAX_OUTPUT_SAMPLES (actually we over-allocate a bit for safety).
        let output_scratch = vec![vec![0.0f32; MAX_OUTPUT_SAMPLES]];

        Ok(Self {
            inner,
            output_scratch,
        })
    }

    /// Process exactly `INPUT_CHUNK_SAMPLES` (1536) samples of 48 kHz mono
    /// f32 audio. Returns a slice of the resampled 16 kHz mono output. The
    /// returned slice length is 0, 256, or 512 samples depending on
    /// rubato's internal overlap-save state. The slice points into the
    /// resampler's internal scratch buffer and must be copied out before
    /// the next call.
    ///
    /// Returns an error if `input_48k.len() != INPUT_CHUNK_SAMPLES`.
    pub fn process_block(&mut self, input_48k: &[f32]) -> anyhow::Result<&[f32]> {
        if input_48k.len() != INPUT_CHUNK_SAMPLES {
            anyhow::bail!(
                "Resampler48to16::process_block: expected {} input samples, got {}",
                INPUT_CHUNK_SAMPLES,
                input_48k.len()
            );
        }

        // rubato wants &[channel_slice]; mono = one channel.
        let input_channels: [&[f32]; 1] = [input_48k];

        // Call process_into_buffer. Returns (input_frames_read, output_frames_written).
        let (_, written) = self
            .inner
            .process_into_buffer(&input_channels, &mut self.output_scratch, None)
            .map_err(|e| anyhow::anyhow!("rubato process_into_buffer: {e}"))?;

        Ok(&self.output_scratch[0][..written])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_succeeds() {
        let r = Resampler48to16::new();
        assert!(r.is_ok(), "resampler should construct: {:?}", r.err());
    }

    #[test]
    fn rejects_wrong_input_size() {
        let mut r = Resampler48to16::new().unwrap();
        let short_input = vec![0.0f32; 1024];
        assert!(r.process_block(&short_input).is_err());

        let long_input = vec![0.0f32; 2048];
        assert!(r.process_block(&long_input).is_err());
    }

    #[test]
    fn output_length_is_valid_in_steady_state() {
        let mut r = Resampler48to16::new().unwrap();
        // Feed 10 frames of 1536-sample input. Accumulate total output.
        let input = vec![0.1f32; INPUT_CHUNK_SAMPLES];
        let mut total_output = 0usize;
        for _ in 0..10 {
            let out = r.process_block(&input).unwrap();
            total_output += out.len();
            // Every call produces 0, 256, or 512 samples.
            assert!(
                out.is_empty() || out.len() == 256 || out.len() == 512,
                "unexpected output length: {}",
                out.len()
            );
        }
        // 10 frames * 1536 input samples @ 48 kHz = 15360 samples = 320 ms
        // -> 320 ms * 16 kHz / 1000 = 5120 output samples (ideal)
        // rubato may be off by one frame in either direction as its
        // internal state primes, so accept a band.
        let ideal = 10 * INPUT_CHUNK_SAMPLES * OUTPUT_SAMPLE_RATE / INPUT_SAMPLE_RATE;
        let diff = (total_output as i64 - ideal as i64).abs();
        assert!(
            diff <= 256,
            "total output {} samples differs from ideal {} by more than one sub-chunk",
            total_output,
            ideal
        );
    }

    #[test]
    fn resamples_sine_wave_approximately() {
        // Generate a 1 kHz sine at 48 kHz, feed it through, and check that
        // the output (at 16 kHz) has the same approximate energy as the
        // input scaled by 1/3 (one-third the sample count).
        let mut r = Resampler48to16::new().unwrap();
        let frames = 10;
        let mut input_rms_total = 0.0f32;
        let mut output_rms_total = 0.0f32;
        let mut input_samples_total = 0usize;
        let mut output_samples_total = 0usize;

        for frame in 0..frames {
            let mut input = vec![0.0f32; INPUT_CHUNK_SAMPLES];
            for (i, s) in input.iter_mut().enumerate() {
                let global = frame * INPUT_CHUNK_SAMPLES + i;
                *s = (2.0 * std::f32::consts::PI * 1000.0 * (global as f32)
                    / INPUT_SAMPLE_RATE as f32)
                    .sin()
                    * 0.5;
            }
            for &s in &input {
                input_rms_total += s * s;
            }
            input_samples_total += input.len();

            let out = r.process_block(&input).unwrap();
            for &s in out {
                output_rms_total += s * s;
            }
            output_samples_total += out.len();
        }

        let input_rms = (input_rms_total / input_samples_total as f32).sqrt();
        let output_rms = (output_rms_total / output_samples_total as f32).sqrt();

        // Both should be close to 0.5 / sqrt(2) ~= 0.354 for a 0.5-amplitude
        // sine wave (the RMS formula), regardless of sample rate.
        let expected_rms = 0.5 / 2.0f32.sqrt();
        assert!(
            (input_rms - expected_rms).abs() < 0.02,
            "input rms {} far from expected {}",
            input_rms,
            expected_rms
        );
        assert!(
            (output_rms - expected_rms).abs() < 0.05,
            "output rms {} far from expected {}",
            output_rms,
            expected_rms
        );
    }
}
