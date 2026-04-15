use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use crate::audio::resampler::{Resampler48to16, INPUT_CHUNK_SAMPLES};
use crate::config::Config;
use crate::enrollment::anti_target::AntiTarget;
use crate::enrollment::profile::Profile;
use crate::gate::audio_gate::{AudioGate, GateState};
use crate::ml::embedding::{EcapaTdnn, EmbeddingWindow};
use crate::ml::similarity::SpeakerVerifier;
use crate::ml::stft::StftProcessor;
use crate::ml::tse::TseModel;
use crate::ml::vad::{SileroVad, VAD_CHUNK_SAMPLES};

/// Max samples to keep in each waveform buffer (~1s at 48kHz).
const WAVEFORM_CAPACITY: usize = 48_000;

pub struct PipelineStatus {
    pub similarity: AtomicU32,
    pub gate_state: AtomicU8,
    pub vad_active: AtomicU8,
    pub bypass_mode: AtomicU8,
    pub monitor_enabled: AtomicBool,
    /// Downsampled input waveform for GUI display. Worker pushes, GUI pops.
    pub waveform_in: Mutex<VecDeque<f32>>,
    /// Downsampled output (gated) waveform for GUI display.
    pub waveform_out: Mutex<VecDeque<f32>>,
}

impl Default for PipelineStatus {
    fn default() -> Self {
        Self {
            similarity: AtomicU32::new(0.0f32.to_bits()),
            gate_state: AtomicU8::new(GateState::Closed.as_u8()),
            vad_active: AtomicU8::new(0),
            bypass_mode: AtomicU8::new(0),
            monitor_enabled: AtomicBool::new(false),
            waveform_in: Mutex::new(VecDeque::with_capacity(WAVEFORM_CAPACITY)),
            waveform_out: Mutex::new(VecDeque::with_capacity(WAVEFORM_CAPACITY)),
        }
    }
}

pub struct PipelineProcessor {
    resampler: Resampler48to16,
    vad: SileroVad,
    ecapa: EcapaTdnn,
    window: EmbeddingWindow,
    verifier: SpeakerVerifier,
    gate: AudioGate,
    resample_accum: Vec<f32>,
    anti_targets: Vec<AntiTarget>,
    status: Arc<PipelineStatus>,
    vad_threshold: f32,
    verify_threshold: f32,
    frame_count: u64,
    // TSE (Phase 7): optional target speaker extraction.
    tse_model: Option<TseModel>,
    tse_stft: Option<StftProcessor>,
    tse_blend: f32,
    enrolled_embedding: Vec<f32>,
    tse_error_count: u32,
}

impl PipelineProcessor {
    pub fn new(
        config: &Config,
        profile: Profile,
        vad: SileroVad,
        ecapa: EcapaTdnn,
        status: Arc<PipelineStatus>,
        tse_model: Option<TseModel>,
    ) -> anyhow::Result<Self> {
        let resampler = Resampler48to16::new()?;
        let crossfade_samples = config.gate.crossfade_samples(config.audio.sample_rate);
        let gate = AudioGate::new(config.gate.hold_frames, crossfade_samples);
        let anti_targets = profile.anti_targets.clone();
        let enrolled_embedding = profile.embedding.clone();
        let verifier = SpeakerVerifier::new(
            profile.embedding,
            config.verification.threshold,
            config.verification.ema_alpha,
        );

        let tse_stft = if tse_model.is_some() {
            Some(StftProcessor::new())
        } else {
            None
        };

        Ok(Self {
            resampler,
            vad,
            ecapa,
            window: EmbeddingWindow::new(),
            verifier,
            gate,
            resample_accum: Vec::with_capacity(VAD_CHUNK_SAMPLES * 2),
            anti_targets,
            status,
            vad_threshold: config.vad.threshold,
            verify_threshold: config.verification.threshold,
            frame_count: 0,
            tse_model,
            tse_stft,
            tse_blend: config.tse.blend,
            enrolled_embedding,
            tse_error_count: 0,
        })
    }

    pub fn process_frame(&mut self, frame: &mut [f32]) -> anyhow::Result<()> {
        debug_assert_eq!(frame.len(), INPUT_CHUNK_SAMPLES);

        // Push input waveform samples for GUI display (every 3rd sample)
        if let Ok(mut wf) = self.status.waveform_in.try_lock() {
            for (i, &s) in frame.iter().enumerate() {
                if i % 3 == 0 {
                    if wf.len() >= WAVEFORM_CAPACITY {
                        wf.pop_front();
                    }
                    wf.push_back(s);
                }
            }
        }

        self.frame_count += 1;
        if self.frame_count % 31 == 0 {
            let rms: f32 = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
            tracing::info!(
                frame = self.frame_count,
                input_rms = format!("{:.5}", rms),
                "input frame"
            );
        }

        let resampled = self.resampler.process_block(frame)?;
        self.resample_accum.extend_from_slice(resampled);

        while self.resample_accum.len() >= VAD_CHUNK_SAMPLES {
            let chunk: Vec<f32> = self.resample_accum.drain(..VAD_CHUNK_SAMPLES).collect();
            let prob = self.vad.prob(&chunk)?;
            let speech = prob >= self.vad_threshold;
            self.status
                .vad_active
                .store(speech as u8, Ordering::Relaxed);

            if self.frame_count % 31 == 0 {
                tracing::info!(
                    vad_prob = format!("{:.3}", prob),
                    speech,
                    threshold = format!("{:.2}", self.vad_threshold),
                    "VAD"
                );
            }

            if speech {
                self.window.push(&chunk);
                if self.window.should_extract() {
                    let live = self.ecapa.extract(self.window.snapshot())?;
                    if self.anti_targets.is_empty() {
                        self.verifier.update(&live);
                    } else {
                        self.verifier
                            .update_with_anti_targets(&live, &self.anti_targets);
                    }
                    self.window.mark_extracted();
                    self.status
                        .similarity
                        .store(self.verifier.current_score().to_bits(), Ordering::Relaxed);

                    tracing::info!(
                        score = format!("{:.4}", self.verifier.current_score()),
                        threshold = format!("{:.2}", self.verify_threshold),
                        is_match = self.verifier.current_score() >= self.verify_threshold,
                        "verify"
                    );
                }
            }
        }

        let bypass = self.status.bypass_mode.load(Ordering::Relaxed);
        let is_match = match bypass {
            1 => true,
            2 => false,
            _ => self.verifier.current_score() >= self.verify_threshold,
        };

        if self.frame_count % 31 == 0 {
            let out_rms: f32 =
                (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
            tracing::info!(
                gate = ?self.gate.state(),
                is_match,
                score = format!("{:.4}", self.verifier.current_score()),
                "gate decision (pre-process)"
            );
            // Log output RMS after gate processing below
            let _ = out_rms; // used after gate.process
        }

        // Apply gate or TSE extraction.
        if let (Some(tse), Some(stft)) = (&mut self.tse_model, &mut self.tse_stft) {
            if self.tse_error_count < 100 {
                if is_match {
                    // TSE path: extract target speaker's voice from the mixture.
                    match Self::apply_tse(tse, stft, frame, &self.enrolled_embedding, self.tse_blend, &mut self.gate, is_match) {
                        Ok(()) => {}
                        Err(e) => {
                            tracing::error!("TSE error, falling back to gate: {e}");
                            self.tse_error_count += 1;
                            self.gate.process(frame, is_match);
                        }
                    }
                } else {
                    // Not the enrolled speaker: silence (same as binary gate closed).
                    frame.fill(0.0);
                    tse.reset_state();
                    self.gate.process(frame, is_match); // keep gate state machine in sync
                }
            } else {
                // Too many TSE errors: fall back to binary gate permanently.
                self.gate.process(frame, is_match);
            }
        } else {
            // No TSE model: use binary gate (existing v1 behavior).
            self.gate.process(frame, is_match);
        }

        // Push output (gated) waveform samples for GUI display
        if let Ok(mut wf) = self.status.waveform_out.try_lock() {
            for (i, &s) in frame.iter().enumerate() {
                if i % 3 == 0 {
                    if wf.len() >= WAVEFORM_CAPACITY {
                        wf.pop_front();
                    }
                    wf.push_back(s);
                }
            }
        }

        if self.frame_count % 31 == 0 {
            let out_rms: f32 =
                (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
            tracing::info!(
                output_rms = format!("{:.5}", out_rms),
                gate = ?self.gate.state(),
                "output frame"
            );
        }

        self.status
            .gate_state
            .store(self.gate.state().as_u8(), Ordering::Relaxed);

        Ok(())
    }

    /// Apply TSE: STFT -> mask prediction -> iSTFT.
    /// Modifies `frame` in-place with the extracted audio.
    fn apply_tse(
        tse: &mut TseModel,
        stft: &mut StftProcessor,
        frame: &mut [f32],
        enrolled_embedding: &[f32],
        blend: f32,
        gate: &mut AudioGate,
        is_match: bool,
    ) -> anyhow::Result<()> {
        let (magnitudes, phases, num_frames) = stft.analyze(frame);

        let mask = tse.predict_mask(&magnitudes, num_frames, enrolled_embedding)?;

        // Apply mask: element-wise multiply magnitude by mask.
        let mut masked_mag = vec![0.0f32; magnitudes.len()];
        for (i, (m, &mk)) in magnitudes.iter().zip(mask.iter()).enumerate() {
            masked_mag[i] = m * mk;
        }

        let extracted = stft.synthesize(&masked_mag, &phases, num_frames);

        if blend >= 1.0 {
            // Pure TSE output.
            let copy_len = frame.len().min(extracted.len());
            frame[..copy_len].copy_from_slice(&extracted[..copy_len]);
        } else if blend <= 0.0 {
            // Pure binary gate (but TSE still ran for state continuity).
            gate.process(frame, is_match);
        } else {
            // Blend: mix TSE output with binary-gated output.
            let mut gated = frame.to_vec();
            gate.process(&mut gated, is_match);
            let copy_len = frame.len().min(extracted.len());
            for i in 0..copy_len {
                frame[i] = blend * extracted[i] + (1.0 - blend) * gated[i];
            }
        }

        Ok(())
    }

    pub fn gate(&self) -> &AudioGate {
        &self.gate
    }

    pub fn verifier(&self) -> &SpeakerVerifier {
        &self.verifier
    }
}
