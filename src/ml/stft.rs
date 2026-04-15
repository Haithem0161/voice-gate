//! STFT analysis/synthesis for Target Speaker Extraction (TSE).
//!
//! Operates at 48 kHz (the pipeline's native sample rate) so TSE
//! produces full-bandwidth output without an extra resample step.
//!
//! Parameters:
//!   - FFT size: 1024
//!   - Hop size: 512 samples (~10.7 ms at 48 kHz)
//!   - Window: Hann (periodic), length 1024
//!   - Frequency bins: 513 (1024/2 + 1)
//!
//! Per 1536-sample pipeline frame, the STFT produces 3 analysis frames.
//! Synthesis uses overlap-add with analysis-only windowing (periodic Hann
//! at 50% overlap satisfies COLA, so no synthesis window is needed).
//!
//! Synthesis maintains a persistent overlap accumulator of FFT_SIZE
//! samples. After each iFFT frame is added, the first HOP_SIZE completed
//! samples are drained as output. This introduces one-hop latency
//! (~10.7 ms at 48 kHz), which is standard for streaming STFT.

use std::sync::Arc;

use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex;

/// FFT size for TSE STFT.
pub const TSE_FFT_SIZE: usize = 1024;

/// Hop size in samples. 512 = FFT_SIZE / 2, giving 50% overlap.
pub const TSE_HOP_SIZE: usize = TSE_FFT_SIZE / 2;

/// Number of frequency bins in the one-sided spectrum.
pub const TSE_NUM_BINS: usize = TSE_FFT_SIZE / 2 + 1; // 513

/// STFT processor with state for streaming chunk-by-chunk processing.
pub struct StftProcessor {
    fft_forward: Arc<dyn RealToComplex<f32>>,
    fft_inverse: Arc<dyn ComplexToReal<f32>>,
    window: Vec<f32>,
    /// Last HOP_SIZE samples from the previous input for analysis continuity.
    input_history: Vec<f32>,
    /// Persistent overlap-add accumulator, length = FFT_SIZE.
    ///
    /// Invariant: at entry to `synthesize()`, the first HOP_SIZE samples
    /// contain one iFFT contribution (from the previous call's last frame's
    /// right half). They need the current call's first frame's left half
    /// to be complete. The remaining HOP_SIZE samples are zero (fresh).
    ola_accum: Vec<f32>,
    // Scratch buffers.
    fft_in: Vec<f32>,
    fft_out: Vec<Complex<f32>>,
    ifft_in: Vec<Complex<f32>>,
    ifft_out: Vec<f32>,
}

impl StftProcessor {
    pub fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft_forward = planner.plan_fft_forward(TSE_FFT_SIZE);
        let fft_inverse = planner.plan_fft_inverse(TSE_FFT_SIZE);

        let mut window = vec![0.0f32; TSE_FFT_SIZE];
        for (i, w) in window.iter_mut().enumerate() {
            *w = 0.5
                * (1.0
                    - (2.0 * std::f32::consts::PI * i as f32 / TSE_FFT_SIZE as f32).cos());
        }

        let fft_out = fft_forward.make_output_vec();
        let ifft_in = vec![Complex::new(0.0f32, 0.0); TSE_NUM_BINS];
        let ifft_out = fft_inverse.make_output_vec();

        Self {
            fft_forward,
            fft_inverse,
            window,
            input_history: vec![0.0f32; TSE_HOP_SIZE],
            ola_accum: vec![0.0f32; TSE_FFT_SIZE],
            fft_in: vec![0.0f32; TSE_FFT_SIZE],
            fft_out,
            ifft_in,
            ifft_out,
        }
    }

    /// Forward STFT: analyze a pipeline frame (typically 1536 samples).
    ///
    /// Returns `(magnitudes, phases, num_frames)`:
    /// - `magnitudes`: flat `[num_frames * TSE_NUM_BINS]`
    /// - `phases`: flat `[num_frames * TSE_NUM_BINS]`
    pub fn analyze(&mut self, frame: &[f32]) -> (Vec<f32>, Vec<f32>, usize) {
        let num_frames = frame.len() / TSE_HOP_SIZE;
        let mut magnitudes = vec![0.0f32; num_frames * TSE_NUM_BINS];
        let mut phases = vec![0.0f32; num_frames * TSE_NUM_BINS];

        for f in 0..num_frames {
            let hop_pos = f * TSE_HOP_SIZE;

            // Fill FFT input: window covers [hop_pos - HOP .. hop_pos + HOP).
            for i in 0..TSE_FFT_SIZE {
                let pos = hop_pos as isize - TSE_HOP_SIZE as isize + i as isize;
                self.fft_in[i] = if pos < 0 {
                    let hi = (self.input_history.len() as isize + pos) as usize;
                    self.input_history[hi]
                } else {
                    let idx = pos as usize;
                    if idx < frame.len() {
                        frame[idx]
                    } else {
                        0.0
                    }
                };
            }

            for (s, w) in self.fft_in.iter_mut().zip(self.window.iter()) {
                *s *= w;
            }

            self.fft_forward
                .process(&mut self.fft_in, &mut self.fft_out)
                .expect("realfft forward: sizes verified at construction");

            let row = f * TSE_NUM_BINS;
            for (k, c) in self.fft_out.iter().enumerate() {
                magnitudes[row + k] = (c.re * c.re + c.im * c.im).sqrt();
                phases[row + k] = c.im.atan2(c.re);
            }
        }

        if frame.len() >= TSE_HOP_SIZE {
            self.input_history
                .copy_from_slice(&frame[frame.len() - TSE_HOP_SIZE..]);
        }

        (magnitudes, phases, num_frames)
    }

    /// Inverse STFT with streaming overlap-add.
    ///
    /// For each STFT frame:
    ///   1. Compute iFFT (1024 samples, normalized by 1/N)
    ///   2. Add into ola_accum (first HOP overlaps with pending, rest is fresh)
    ///   3. Drain the first HOP_SIZE completed samples as output
    ///   4. Shift ola_accum left by HOP_SIZE
    ///
    /// Returns `num_frames * HOP_SIZE` output samples.
    pub fn synthesize(
        &mut self,
        masked_mag: &[f32],
        phases: &[f32],
        num_frames: usize,
    ) -> Vec<f32> {
        debug_assert_eq!(masked_mag.len(), num_frames * TSE_NUM_BINS);
        debug_assert_eq!(phases.len(), num_frames * TSE_NUM_BINS);

        let output_len = num_frames * TSE_HOP_SIZE;
        let mut output = Vec::with_capacity(output_len);
        let inv_n = 1.0 / TSE_FFT_SIZE as f32;

        for f in 0..num_frames {
            let row = f * TSE_NUM_BINS;

            // Reconstruct complex spectrum.
            for k in 0..TSE_NUM_BINS {
                let mag = masked_mag[row + k];
                let ph = phases[row + k];
                self.ifft_in[k] = Complex::new(mag * ph.cos(), mag * ph.sin());
            }
            self.ifft_in[0].im = 0.0;
            self.ifft_in[TSE_NUM_BINS - 1].im = 0.0;

            // Inverse FFT.
            self.fft_inverse
                .process(&mut self.ifft_in, &mut self.ifft_out)
                .expect("realfft inverse: sizes verified at construction");

            // Add normalized iFFT output into ola_accum.
            // ola_accum is always FFT_SIZE long. The first HOP_SIZE slots
            // already have a contribution from the previous frame's right
            // half. Adding the current frame's left half completes them.
            for (i, &s) in self.ifft_out.iter().enumerate() {
                self.ola_accum[i] += s * inv_n;
            }

            // The first HOP_SIZE samples are now complete (two overlapping
            // windows have contributed). Drain them as output.
            output.extend_from_slice(&self.ola_accum[..TSE_HOP_SIZE]);

            // Shift accumulator: move the right half to the left, zero the
            // right half for the next frame's fresh territory.
            self.ola_accum.copy_within(TSE_HOP_SIZE.., 0);
            for s in self.ola_accum[TSE_HOP_SIZE..].iter_mut() {
                *s = 0.0;
            }
        }

        debug_assert_eq!(output.len(), output_len);
        output
    }

    /// Reset all state.
    pub fn reset(&mut self) {
        self.input_history.fill(0.0);
        self.ola_accum.fill(0.0);
    }
}

impl Default for StftProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stft_frame_count() {
        let mut proc = StftProcessor::new();
        let frame = vec![0.0f32; 1536];
        let (_, _, nf) = proc.analyze(&frame);
        assert_eq!(nf, 3);
    }

    #[test]
    fn test_stft_output_dimensions() {
        let mut proc = StftProcessor::new();
        let frame = vec![0.1f32; 1536];
        let (mag, phase, nf) = proc.analyze(&frame);
        assert_eq!(nf, 3);
        assert_eq!(mag.len(), 3 * TSE_NUM_BINS);
        assert_eq!(phase.len(), 3 * TSE_NUM_BINS);
    }

    #[test]
    fn test_stft_magnitudes_non_negative() {
        let mut proc = StftProcessor::new();
        let mut frame = vec![0.0f32; 1536];
        for (i, s) in frame.iter_mut().enumerate() {
            *s = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48000.0).sin();
        }
        let (mag, _, _) = proc.analyze(&frame);
        for (i, &m) in mag.iter().enumerate() {
            assert!(m >= 0.0, "magnitude at index {i} is negative: {m}");
        }
    }

    /// Zero mask should produce silence.
    #[test]
    fn test_stft_zero_mask_silence() {
        let mut proc = StftProcessor::new();
        let mut frame = vec![0.0f32; 1536];
        for (i, s) in frame.iter_mut().enumerate() {
            *s = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin() * 0.5;
        }
        let (mag, phase, nf) = proc.analyze(&frame);
        let zero_mag = vec![0.0f32; mag.len()];
        let out = proc.synthesize(&zero_mag, &phase, nf);
        assert_eq!(out.len(), 1536);
        let rms: f32 = (out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt();
        assert!(rms < 1e-6, "zero mask should produce silence, got RMS={rms}");
    }

    /// DC roundtrip: constant signal should reconstruct after priming.
    #[test]
    fn test_stft_dc_roundtrip() {
        let mut proc = StftProcessor::new();
        let dc = vec![1.0f32; 1536];
        let mut outputs = Vec::new();
        for _ in 0..5 {
            let (mag, phase, nf) = proc.analyze(&dc);
            let out = proc.synthesize(&mag, &phase, nf);
            outputs.push(out);
        }
        // Check chunk 2+ (after startup transient).
        let out = &outputs[2];
        let max_err: f32 = out.iter().map(|s| (s - 1.0).abs()).fold(0.0, f32::max);
        assert!(max_err < 0.01, "DC roundtrip error: {max_err}");
    }

    /// Sine roundtrip: 440 Hz sine should reconstruct after priming.
    #[test]
    fn test_stft_roundtrip_identity() {
        let mut proc = StftProcessor::new();
        let freq = 440.0;
        let total = 1536 * 8;
        let mut signal = vec![0.0f32; total];
        for (i, s) in signal.iter_mut().enumerate() {
            *s = (2.0 * std::f32::consts::PI * freq * i as f32 / 48000.0).sin() * 0.5;
        }

        let mut all_output = Vec::with_capacity(total);
        for chunk_idx in 0..8 {
            let start = chunk_idx * 1536;
            let chunk = &signal[start..start + 1536];
            let (mag, phase, nf) = proc.analyze(chunk);
            let out = proc.synthesize(&mag, &phase, nf);
            all_output.extend_from_slice(&out);
        }

        // The output is shifted by one hop (512 samples) due to the
        // overlap-add latency. Compare with the shifted input.
        let latency = TSE_HOP_SIZE;
        let check_start = 3 * 1536; // skip startup transient
        let check_end = 8 * 1536 - latency;
        let mut max_err = 0.0f32;
        for i in check_start..check_end {
            let err = (all_output[i] - signal[i - latency]).abs();
            if err > max_err {
                max_err = err;
            }
        }
        assert!(
            max_err < 0.01,
            "roundtrip error: {max_err} (expected < 0.01)"
        );
    }

    /// Multi-frame continuity test with latency compensation.
    #[test]
    fn test_stft_multi_frame_continuity() {
        let mut proc = StftProcessor::new();
        let total = 1536 * 10;
        let mut signal = vec![0.0f32; total];
        for (i, s) in signal.iter_mut().enumerate() {
            *s = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin() * 0.3;
        }

        let mut all_output = Vec::with_capacity(total);
        for chunk_idx in 0..10 {
            let start = chunk_idx * 1536;
            let chunk = &signal[start..start + 1536];
            let (mag, phase, nf) = proc.analyze(chunk);
            let out = proc.synthesize(&mag, &phase, nf);
            all_output.extend_from_slice(&out);
        }

        let latency = TSE_HOP_SIZE;
        let check_start = 3 * 1536;
        let check_end = 10 * 1536 - latency;
        let mut max_err = 0.0f32;
        for i in check_start..check_end {
            let err = (all_output[i] - signal[i - latency]).abs();
            if err > max_err {
                max_err = err;
            }
        }
        assert!(
            max_err < 0.01,
            "multi-frame error: {max_err} (expected < 0.01)"
        );
    }
}
