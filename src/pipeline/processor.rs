use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;

use crate::audio::resampler::{Resampler48to16, INPUT_CHUNK_SAMPLES};
use crate::config::Config;
use crate::enrollment::profile::Profile;
use crate::gate::audio_gate::{AudioGate, GateState};
use crate::ml::embedding::{EcapaTdnn, EmbeddingWindow};
use crate::ml::similarity::SpeakerVerifier;
use crate::ml::vad::{SileroVad, VAD_CHUNK_SAMPLES};

pub struct PipelineStatus {
    pub similarity: AtomicU32,
    pub gate_state: AtomicU8,
    pub vad_active: AtomicU8,
    pub bypass_mode: AtomicU8,
}

impl Default for PipelineStatus {
    fn default() -> Self {
        Self {
            similarity: AtomicU32::new(0.0f32.to_bits()),
            gate_state: AtomicU8::new(GateState::Closed.as_u8()),
            vad_active: AtomicU8::new(0),
            bypass_mode: AtomicU8::new(0),
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
    status: Arc<PipelineStatus>,
    vad_threshold: f32,
    verify_threshold: f32,
}

impl PipelineProcessor {
    pub fn new(
        config: &Config,
        profile: Profile,
        vad: SileroVad,
        ecapa: EcapaTdnn,
        status: Arc<PipelineStatus>,
    ) -> anyhow::Result<Self> {
        let resampler = Resampler48to16::new()?;
        let crossfade_samples = config.gate.crossfade_samples(config.audio.sample_rate);
        let gate = AudioGate::new(config.gate.hold_frames, crossfade_samples);
        let verifier = SpeakerVerifier::new(
            profile.embedding,
            config.verification.threshold,
            config.verification.ema_alpha,
        );

        Ok(Self {
            resampler,
            vad,
            ecapa,
            window: EmbeddingWindow::new(),
            verifier,
            gate,
            resample_accum: Vec::with_capacity(VAD_CHUNK_SAMPLES * 2),
            status,
            vad_threshold: config.vad.threshold,
            verify_threshold: config.verification.threshold,
        })
    }

    pub fn process_frame(&mut self, frame: &mut [f32]) -> anyhow::Result<()> {
        debug_assert_eq!(frame.len(), INPUT_CHUNK_SAMPLES);

        let resampled = self.resampler.process_block(frame)?;
        self.resample_accum.extend_from_slice(resampled);

        while self.resample_accum.len() >= VAD_CHUNK_SAMPLES {
            let chunk: Vec<f32> = self.resample_accum.drain(..VAD_CHUNK_SAMPLES).collect();
            let prob = self.vad.prob(&chunk)?;
            let speech = prob >= self.vad_threshold;
            self.status
                .vad_active
                .store(speech as u8, Ordering::Relaxed);

            if speech {
                self.window.push(&chunk);
                if self.window.should_extract() {
                    let live = self.ecapa.extract(self.window.snapshot())?;
                    self.verifier.update(&live);
                    self.window.mark_extracted();
                    self.status
                        .similarity
                        .store(self.verifier.current_score().to_bits(), Ordering::Relaxed);
                }
            }
        }

        let bypass = self.status.bypass_mode.load(Ordering::Relaxed);
        let is_match = match bypass {
            1 => true,
            2 => false,
            _ => self.verifier.current_score() >= self.verify_threshold,
        };

        self.gate.process(frame, is_match);
        self.status
            .gate_state
            .store(self.gate.state().as_u8(), Ordering::Relaxed);

        Ok(())
    }

    pub fn gate(&self) -> &AudioGate {
        &self.gate
    }

    pub fn verifier(&self) -> &SpeakerVerifier {
        &self.verifier
    }
}
