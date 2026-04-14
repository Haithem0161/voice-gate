//! Kaldi-compatible 80-bin log-Mel filterbank extractor for WeSpeaker.
//!
//! Matches `torchaudio.compliance.kaldi.fbank` with the settings used by
//! WeSpeaker's `wespeaker/bin/infer_onnx.py` reference:
//!
//! ```python
//! waveform = waveform * (1 << 15)
//! mat = kaldi.fbank(
//!     waveform,
//!     num_mel_bins=80,
//!     frame_length=25,
//!     frame_shift=10,
//!     dither=0.0,
//!     sample_frequency=16000,
//!     window_type='hamming',
//!     use_energy=False,
//! )
//! # CMN, without CVN
//! mat = mat - torch.mean(mat, dim=0)
//! ```
//!
//! All other parameters use torchaudio's Kaldi-compatible defaults:
//! `preemphasis_coefficient=0.97`, `remove_dc_offset=true`, `snip_edges=true`,
//! `round_to_power_of_two=true`, `low_freq=20.0`, `high_freq=nyquist=8000`.
//!
//! The extractor owns a `realfft::RealFftPlanner<f32>` and the pre-computed
//! Hamming window + 80 Mel filter coefficients so `compute` does not allocate
//! any of those per call. It does allocate the output `Vec<f32>` because the
//! number of frames depends on the input length.

use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};

pub const SAMPLE_RATE_HZ: u32 = 16_000;
pub const FRAME_LENGTH_MS: u32 = 25;
pub const FRAME_SHIFT_MS: u32 = 10;
pub const FRAME_LENGTH_SAMPLES: usize =
    (FRAME_LENGTH_MS as usize) * (SAMPLE_RATE_HZ as usize) / 1000; // 400
pub const FRAME_SHIFT_SAMPLES: usize = (FRAME_SHIFT_MS as usize) * (SAMPLE_RATE_HZ as usize) / 1000; //  160
pub const FFT_SIZE: usize = 512; // next power of two >= FRAME_LENGTH_SAMPLES
pub const NUM_FFT_BINS: usize = FFT_SIZE / 2 + 1; // 257 (includes Nyquist)
pub const NUM_MEL_BINS: usize = 80;

/// Torchaudio/Kaldi default: Kaldi preemphasis coefficient. Applied after
/// DC removal and before the window. See `_get_window` in the torchaudio
/// source (`torchaudio.compliance.kaldi`).
const PREEMPHASIS_COEFFICIENT: f32 = 0.97;

/// Torchaudio/Kaldi default: lower edge of the Mel bank in Hz.
const LOW_FREQ_HZ: f32 = 20.0;

/// Numerical floor applied to Mel energies before the log. Matches the
/// `f32::EPSILON` clamp in torchaudio's `_get_epsilon`.
const EPSILON: f32 = f32::EPSILON;

/// WeSpeaker's reference multiplies the waveform by `1 << 15` before calling
/// `kaldi.fbank`. This keeps the internal values in the same numeric range
/// that Kaldi's C++ sees when reading raw PCM16 files.
const WAVEFORM_SCALE: f32 = 32768.0;

/// Owning state for a Kaldi-compatible fbank extractor.
///
/// Construct once via `new`; call `compute` per audio window.
pub struct FbankExtractor {
    /// rustfft plan for the real 512-point FFT. `Arc` because realfft hands
    /// back a Box<dyn RealToComplex<f32>> which we store as Arc so it can be
    /// cheaply cloned if we ever need to.
    fft: Arc<dyn RealToComplex<f32>>,
    /// Pre-computed 400-sample Hamming window with alpha=0.54, beta=0.46,
    /// symmetric (non-periodic). Matches torch.hamming_window.
    window: [f32; FRAME_LENGTH_SAMPLES],
    /// Pre-computed 80 Mel filters, each of length `NUM_FFT_BINS` (257),
    /// stored row-major as `[NUM_MEL_BINS][NUM_FFT_BINS]`.
    mel_filters: Vec<Vec<f32>>,
}

impl FbankExtractor {
    pub fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        let mut window = [0.0f32; FRAME_LENGTH_SAMPLES];
        for (i, slot) in window.iter_mut().enumerate() {
            // Symmetric Hamming (non-periodic): 0.54 - 0.46 * cos(2*pi*i / (N-1))
            let n_minus_1 = (FRAME_LENGTH_SAMPLES - 1) as f32;
            *slot = 0.54 - 0.46 * (2.0 * std::f32::consts::PI * (i as f32) / n_minus_1).cos();
        }

        let mel_filters =
            build_mel_filters(NUM_MEL_BINS, FFT_SIZE, SAMPLE_RATE_HZ as f32, LOW_FREQ_HZ);

        Self {
            fft,
            window,
            mel_filters,
        }
    }

    /// Number of frames produced for an audio input of length `audio_len`
    /// samples under the Kaldi `snip_edges=true` rule.
    ///
    /// The formula from torchaudio `_get_strided`:
    ///     m = (audio_len - window_size) / window_shift + 1
    /// If `audio_len < window_size`, returns 0.
    pub fn num_frames(audio_len: usize) -> usize {
        if audio_len < FRAME_LENGTH_SAMPLES {
            0
        } else {
            (audio_len - FRAME_LENGTH_SAMPLES) / FRAME_SHIFT_SAMPLES + 1
        }
    }

    /// Compute the log-Mel fbank for a 16 kHz f32 audio slice in the [-1, 1]
    /// range. Returns a flat `Vec<f32>` of length `num_frames * NUM_MEL_BINS`
    /// laid out row-major: row `t` occupies `[t * 80 .. (t + 1) * 80]`.
    ///
    /// The returned features are log-compressed AND cepstral-mean-normalized
    /// per WeSpeaker's reference (`mat - torch.mean(mat, dim=0)`).
    ///
    /// If the input is shorter than a single frame (400 samples), returns
    /// an empty Vec.
    pub fn compute(&self, audio_16k: &[f32]) -> Vec<f32> {
        let num_frames = Self::num_frames(audio_16k.len());
        if num_frames == 0 {
            return Vec::new();
        }

        // Scratch buffers for one frame. These are allocated per-call but
        // not per-frame: the per-frame cost is fills + FFT + mel sum.
        let mut fft_input = self.fft.make_input_vec();
        let mut fft_output = self.fft.make_output_vec();
        debug_assert_eq!(fft_input.len(), FFT_SIZE);
        debug_assert_eq!(fft_output.len(), NUM_FFT_BINS);

        let mut features = vec![0.0f32; num_frames * NUM_MEL_BINS];

        for frame_idx in 0..num_frames {
            let start = frame_idx * FRAME_SHIFT_SAMPLES;
            let frame = &audio_16k[start..start + FRAME_LENGTH_SAMPLES];

            // Copy the frame into the FFT input buffer, with two changes:
            //   1. Scale by 2^15 (WeSpeaker reference).
            //   2. Zero-pad the tail from FRAME_LENGTH_SAMPLES (400) to FFT_SIZE (512).
            for (dst, &src) in fft_input[..FRAME_LENGTH_SAMPLES]
                .iter_mut()
                .zip(frame.iter())
            {
                *dst = src * WAVEFORM_SCALE;
            }
            for slot in fft_input[FRAME_LENGTH_SAMPLES..].iter_mut() {
                *slot = 0.0;
            }

            // Remove DC offset (subtract mean over the 400 real samples;
            // the zero tail is not included in the mean).
            let mean =
                fft_input[..FRAME_LENGTH_SAMPLES].iter().sum::<f32>() / FRAME_LENGTH_SAMPLES as f32;
            for s in fft_input[..FRAME_LENGTH_SAMPLES].iter_mut() {
                *s -= mean;
            }

            // Preemphasis: x[i] -= 0.97 * x[i-1], with "replicate" boundary
            // at i=0 (so x[0] -= 0.97 * x[0] = 0.03 * x[0]).
            // We apply it in-place right-to-left to avoid aliasing.
            for i in (1..FRAME_LENGTH_SAMPLES).rev() {
                fft_input[i] -= PREEMPHASIS_COEFFICIENT * fft_input[i - 1];
            }
            fft_input[0] -= PREEMPHASIS_COEFFICIENT * fft_input[0];

            // Apply the Hamming window to the 400 real samples. The zero
            // tail stays zero.
            for (s, w) in fft_input[..FRAME_LENGTH_SAMPLES]
                .iter_mut()
                .zip(self.window.iter())
            {
                *s *= *w;
            }

            // Real FFT: 512 real -> 257 complex.
            // Process can fail only if the buffer lengths are wrong; we
            // enforced them above, so expect is safe here.
            self.fft
                .process(&mut fft_input, &mut fft_output)
                .expect("realfft sizes checked at construction");

            // Power spectrum: |FFT|^2 per bin.
            // Then multiply against each of the 80 Mel filters.
            // Then clamp to epsilon and take the natural log.
            let row_offset = frame_idx * NUM_MEL_BINS;
            for (mel_idx, filter) in self.mel_filters.iter().enumerate() {
                let mut energy = 0.0f32;
                for (bin, c) in fft_output.iter().enumerate() {
                    let power = c.re * c.re + c.im * c.im;
                    energy += power * filter[bin];
                }
                let clamped = energy.max(EPSILON);
                features[row_offset + mel_idx] = clamped.ln();
            }
        }

        // CMN: subtract each of the 80 column means over time.
        // Per WeSpeaker's reference: `mat = mat - torch.mean(mat, dim=0)`.
        let mut col_means = [0.0f32; NUM_MEL_BINS];
        for t in 0..num_frames {
            let row = &features[t * NUM_MEL_BINS..(t + 1) * NUM_MEL_BINS];
            for (m, r) in col_means.iter_mut().zip(row.iter()) {
                *m += *r;
            }
        }
        for m in col_means.iter_mut() {
            *m /= num_frames as f32;
        }
        for t in 0..num_frames {
            let row = &mut features[t * NUM_MEL_BINS..(t + 1) * NUM_MEL_BINS];
            for (r, m) in row.iter_mut().zip(col_means.iter()) {
                *r -= *m;
            }
        }

        features
    }
}

impl Default for FbankExtractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Kaldi's Mel scale: `mel = 1127 * ln(1 + hz / 700)`.
/// Note: Kaldi uses natural log with the 1127 constant, NOT the
/// 2595 * log10(1 + hz/700) convention that HTK uses.
fn hz_to_mel(hz: f32) -> f32 {
    1127.0 * (1.0 + hz / 700.0).ln()
}

/// Build `num_bins` triangular Mel filters of length `NUM_FFT_BINS`.
///
/// Matches torchaudio's `get_mel_banks` exactly, modulo VTLN (which we do
/// not use: `vtln_warp_factor == 1.0`). Specifically:
///   - `high_freq` defaults to `nyquist = sample_rate / 2`.
///   - FFT bin width is `sample_rate / fft_size`.
///   - The mel range `[mel_low, mel_high]` is divided into `num_bins + 1`
///     equal steps, producing `num_bins + 2` boundary points; each filter is
///     a triangle from `bin[k]` (left) to `bin[k+1]` (center) to `bin[k+2]`
///     (right), all in Mel space.
///   - The output includes one extra zero-valued Nyquist bin on the right
///     so the filter length matches the rfft output length
///     (`fft_size / 2 + 1` instead of `fft_size / 2`).
fn build_mel_filters(
    num_bins: usize,
    fft_size: usize,
    sample_rate_hz: f32,
    low_freq_hz: f32,
) -> Vec<Vec<f32>> {
    let nyquist = sample_rate_hz / 2.0;
    let high_freq_hz = nyquist;
    let num_fft_bins = fft_size / 2; // 256 -- torchaudio's get_mel_banks uses this
    let fft_bin_width = sample_rate_hz / fft_size as f32;

    let mel_low = hz_to_mel(low_freq_hz);
    let mel_high = hz_to_mel(high_freq_hz);
    let mel_delta = (mel_high - mel_low) / (num_bins as f32 + 1.0);

    let mut filters = vec![vec![0.0f32; num_fft_bins + 1]; num_bins];

    for (bin, filter) in filters.iter_mut().enumerate() {
        let left_mel = mel_low + (bin as f32) * mel_delta;
        let center_mel = mel_low + (bin as f32 + 1.0) * mel_delta;
        let right_mel = mel_low + (bin as f32 + 2.0) * mel_delta;

        for (k, slot) in filter[..num_fft_bins].iter_mut().enumerate() {
            let freq = (k as f32) * fft_bin_width;
            let mel = hz_to_mel(freq);

            let up_slope = (mel - left_mel) / (center_mel - left_mel);
            let down_slope = (right_mel - mel) / (right_mel - center_mel);
            // torchaudio: bins = torch.max(torch.zeros(1), torch.min(up_slope, down_slope))
            let val = up_slope.min(down_slope).max(0.0);
            *slot = val;
        }
        // The Nyquist bin (index num_fft_bins) stays at 0.0 -- matches
        // torchaudio's `F.pad(mel_energies, (0, 1), mode='constant', value=0)`.
    }

    filters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn num_frames_too_short() {
        assert_eq!(FbankExtractor::num_frames(0), 0);
        assert_eq!(FbankExtractor::num_frames(399), 0);
    }

    #[test]
    fn num_frames_exact_window() {
        // 400 samples = 1 frame exactly
        assert_eq!(FbankExtractor::num_frames(400), 1);
    }

    #[test]
    fn num_frames_standard_windows() {
        // 0.5 s @ 16 kHz = 8000 samples -> (8000 - 400)/160 + 1 = 48
        assert_eq!(FbankExtractor::num_frames(8_000), 48);
        // 1.5 s @ 16 kHz = 24000 samples -> (24000 - 400)/160 + 1 = 148
        assert_eq!(FbankExtractor::num_frames(24_000), 148);
    }

    #[test]
    fn hz_to_mel_matches_kaldi() {
        // Kaldi uses natural-log-based mel with 1127 constant.
        // mel(0) = 0
        assert!((hz_to_mel(0.0)).abs() < 1e-5);
        // mel(700) = 1127 * ln(2) ~= 781.17
        assert!((hz_to_mel(700.0) - 1127.0 * 2.0f32.ln()).abs() < 1e-3);
    }

    #[test]
    fn mel_filters_shape_and_unity() {
        let filters = build_mel_filters(80, 512, 16000.0, 20.0);
        assert_eq!(filters.len(), 80);
        assert_eq!(filters[0].len(), 257);
        // All coefficients are in [0, 1]
        for filter in &filters {
            for &v in filter {
                assert!((0.0..=1.01).contains(&v));
            }
        }
        // The Nyquist bin (index 256) is always 0 for every filter.
        for filter in &filters {
            assert_eq!(filter[256], 0.0);
        }
    }

    #[test]
    fn window_is_hamming_symmetric() {
        let ex = FbankExtractor::new();
        // Symmetric: window[i] == window[N-1-i]
        for i in 0..FRAME_LENGTH_SAMPLES / 2 {
            let a = ex.window[i];
            let b = ex.window[FRAME_LENGTH_SAMPLES - 1 - i];
            assert!((a - b).abs() < 1e-6);
        }
        // Endpoints: 0.54 - 0.46 = 0.08
        assert!((ex.window[0] - 0.08).abs() < 1e-5);
        assert!((ex.window[FRAME_LENGTH_SAMPLES - 1] - 0.08).abs() < 1e-5);
        // Center (i = (N-1)/2 rounded): close to 1.0
        let center = ex.window[(FRAME_LENGTH_SAMPLES - 1) / 2];
        assert!(
            center > 0.99,
            "hamming center should be close to 1.0, got {center}"
        );
    }

    #[test]
    fn compute_on_tone_produces_finite_features() {
        let ex = FbankExtractor::new();
        // 1 s of 440 Hz sine at 16 kHz.
        let len = 16_000;
        let mut audio = vec![0.0f32; len];
        for (i, s) in audio.iter_mut().enumerate() {
            *s = (2.0 * std::f32::consts::PI * 440.0 * (i as f32) / 16_000.0).sin() * 0.5;
        }
        let features = ex.compute(&audio);
        let expected_frames = FbankExtractor::num_frames(len);
        assert_eq!(features.len(), expected_frames * NUM_MEL_BINS);

        // Sanity: all features are finite (no NaN / Inf).
        for (i, &v) in features.iter().enumerate() {
            assert!(v.is_finite(), "feature {i} is not finite: {v}");
        }

        // CMN guarantees each column has mean 0 over time (within fp tolerance).
        for col in 0..NUM_MEL_BINS {
            let mut sum = 0.0f32;
            for t in 0..expected_frames {
                sum += features[t * NUM_MEL_BINS + col];
            }
            let mean = sum / expected_frames as f32;
            assert!(
                mean.abs() < 1e-3,
                "column {col} mean not zero after CMN: {mean}"
            );
        }
    }

    #[test]
    fn compute_on_short_input_returns_empty() {
        let ex = FbankExtractor::new();
        let audio = vec![0.0f32; 399];
        let features = ex.compute(&audio);
        assert_eq!(features.len(), 0);
    }
}
