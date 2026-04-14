//! WeSpeaker ResNet34 speaker embedding wrapper + real-time window buffer.
//!
//! Takes raw 16 kHz f32 audio, computes Kaldi-compatible log-Mel fbank
//! features (80 bins) via `crate::ml::fbank`, applies CMN, feeds a
//! `[1, T, 80]` tensor through WeSpeaker's ResNet34 ONNX model, extracts
//! the `[1, 256]` output tensor, L2-normalizes, and returns a
//! `Vec<f32>` of length `EMBEDDING_DIM` (256).
//!
//! See `docs/voicegate/phase-02.md` sections 3.3, 3.6, and 4.2 for the
//! full spec, and `docs/voicegate/research.md` section 3 for D-002R
//! (WeSpeaker as primary model).
//!
//! The struct is named `EcapaTdnn` for historical continuity with the
//! original plan; it actually wraps a WeSpeaker ResNet34 model after
//! D-002R. Renaming is a Phase 6 cleanup item.

use std::path::Path;

use ort::session::Session;
use ort::value::{DynValue, Tensor};

use crate::ml::fbank::{FbankExtractor, NUM_MEL_BINS};
use crate::VoiceGateError;

/// Dimensionality of the speaker embedding produced by WeSpeaker
/// ResNet34_LM. Per Decision D-002R + D-003: this is a const, not a
/// literal, because profile.bin stores it in its header and a profile
/// from a different embedding model is safely rejected by `Profile::load`
/// (Phase 3).
pub const EMBEDDING_DIM: usize = 256;

/// Minimum window length (samples @ 16 kHz) before the first embedding
/// extraction fires. 0.5 s -- anything shorter produces noisy embeddings.
pub const MIN_WINDOW_SAMPLES_16K: usize = 8_000;

/// Maximum window length (samples @ 16 kHz). 1.5 s -- beyond this the
/// window starts discarding its oldest samples on push.
pub const MAX_WINDOW_SAMPLES_16K: usize = 24_000;

/// Re-extract the embedding every this many pushed samples once the
/// minimum is met. 200 ms at 16 kHz = 3200 samples = 5 Hz update rate.
pub const REEXTRACT_INTERVAL_SAMPLES_16K: usize = 3_200;

/// WeSpeaker speaker embedding model (ResNet34-LM, 256-dim).
pub struct EcapaTdnn {
    session: Session,
    fbank: FbankExtractor,
    /// Name of the input tensor on this specific ONNX file. For the
    /// official WeSpeaker ResNet34 ONNX this is `"feats"`, but some
    /// forks use `"input"`. Resolved at load time, not hard-coded.
    input_name: String,
    /// Name of the output tensor. For the official WeSpeaker ONNX this
    /// is `"embs"`, but Phase 1's ort smoke test confirms we still read
    /// whatever name the session reports. Resolved at load time.
    output_name: String,
}

impl EcapaTdnn {
    /// Load the WeSpeaker ResNet34 model from `model_path`.
    pub fn load(model_path: &Path) -> Result<Self, VoiceGateError> {
        if !model_path.exists() {
            return Err(VoiceGateError::ModelNotFound(format!(
                "wespeaker ONNX not found at {}. Run `make models` to download it.",
                model_path.display()
            )));
        }

        let session = Session::builder()
            .map_err(|e| VoiceGateError::Ml(format!("Session::builder: {e}")))?
            .commit_from_file(model_path)
            .map_err(|e| VoiceGateError::Ml(format!("load wespeaker: {e}")))?;

        let input_name = session
            .inputs
            .first()
            .map(|i| i.name.clone())
            .ok_or_else(|| VoiceGateError::Ml("wespeaker ONNX has no inputs".into()))?;
        let output_name = session
            .outputs
            .first()
            .map(|o| o.name.clone())
            .ok_or_else(|| VoiceGateError::Ml("wespeaker ONNX has no outputs".into()))?;

        tracing::info!(
            input_name = %input_name,
            output_name = %output_name,
            "loaded wespeaker resnet34 model"
        );

        Ok(Self {
            session,
            fbank: FbankExtractor::new(),
            input_name,
            output_name,
        })
    }

    /// Extract an L2-normalized 256-float speaker embedding from a slice
    /// of 16 kHz f32 audio. The input must be at least
    /// `MIN_WINDOW_SAMPLES_16K` (8000 = 0.5 s) long for the embedding
    /// to be meaningful.
    ///
    /// Pipeline: fbank (with CMN) -> [1, T, 80] tensor -> session.run ->
    /// [1, 256] output -> L2 normalize.
    pub fn extract(&mut self, audio_16k: &[f32]) -> Result<Vec<f32>, VoiceGateError> {
        if audio_16k.len() < MIN_WINDOW_SAMPLES_16K {
            return Err(VoiceGateError::Ml(format!(
                "EcapaTdnn::extract: input too short ({} samples < {} minimum)",
                audio_16k.len(),
                MIN_WINDOW_SAMPLES_16K
            )));
        }

        // 1. Fbank extraction. Returns flattened [T, 80] row-major.
        //    FbankExtractor already applies CMN, matching WeSpeaker's
        //    Python reference mat - torch.mean(mat, dim=0).
        let feats = self.fbank.compute(audio_16k);
        if feats.is_empty() {
            return Err(VoiceGateError::Ml(
                "fbank produced zero frames (audio too short after framing)".into(),
            ));
        }
        let num_frames = feats.len() / NUM_MEL_BINS;
        debug_assert_eq!(feats.len(), num_frames * NUM_MEL_BINS);

        // 2. Build the [1, T, 80] f32 tensor. We hand ort an owned Vec
        //    so it can take ownership and we do not need to worry about
        //    lifetime of `feats` past session.run.
        let feats_tensor: Tensor<f32> =
            Tensor::from_array(([1_usize, num_frames, NUM_MEL_BINS], feats))
                .map_err(|e| VoiceGateError::Ml(format!("build feats tensor: {e}")))?;

        // 3. Run inference.
        let outputs = self
            .session
            .run(ort::inputs! { self.input_name.as_str() => feats_tensor })
            .map_err(|e| VoiceGateError::Ml(format!("wespeaker session.run: {e}")))?;

        // 4. Read the [1, 256] output into an owned Vec.
        let output_value: &DynValue = &outputs[self.output_name.as_str()];
        let (shape, data) = output_value
            .try_extract_tensor::<f32>()
            .map_err(|e| VoiceGateError::Ml(format!("extract embedding tensor: {e}")))?;

        if data.len() != EMBEDDING_DIM {
            return Err(VoiceGateError::Ml(format!(
                "wespeaker output has {} elements, expected {EMBEDDING_DIM}. shape: {:?}",
                data.len(),
                shape
            )));
        }

        let mut embedding: Vec<f32> = data.to_vec();

        // 5. L2 normalize. WeSpeaker's ONNX does not normalize internally;
        //    cosine similarity is only meaningful on unit vectors, so this
        //    step is mandatory.
        crate::ml::similarity::l2_normalize(&mut embedding);

        Ok(embedding)
    }
}

/// Rolling window of recent 16 kHz audio used to trigger periodic
/// embedding extractions in the real-time pipeline.
///
/// The window discards its oldest samples once it exceeds
/// `MAX_WINDOW_SAMPLES_16K` (1.5 s). `should_extract` returns true once
/// at least `MIN_WINDOW_SAMPLES_16K` (0.5 s) have been accumulated AND
/// at least `REEXTRACT_INTERVAL_SAMPLES_16K` (200 ms) have been pushed
/// since the last extraction. The caller calls `mark_extracted` after
/// each successful `EcapaTdnn::extract` to reset the interval counter.
pub struct EmbeddingWindow {
    buf: Vec<f32>,
    samples_since_last_extract: usize,
}

impl EmbeddingWindow {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(MAX_WINDOW_SAMPLES_16K),
            samples_since_last_extract: 0,
        }
    }

    /// Append 16 kHz audio samples to the window. If the window exceeds
    /// its max size, the oldest samples are dropped (tail-shift). Not a
    /// hot path -- this runs ~31 Hz in Phase 4+, each push is at most
    /// 512 samples.
    pub fn push(&mut self, audio_16k: &[f32]) {
        self.buf.extend_from_slice(audio_16k);
        if self.buf.len() > MAX_WINDOW_SAMPLES_16K {
            let drop = self.buf.len() - MAX_WINDOW_SAMPLES_16K;
            self.buf.drain(..drop);
        }
        self.samples_since_last_extract += audio_16k.len();
    }

    pub fn should_extract(&self) -> bool {
        self.buf.len() >= MIN_WINDOW_SAMPLES_16K
            && self.samples_since_last_extract >= REEXTRACT_INTERVAL_SAMPLES_16K
    }

    pub fn snapshot(&self) -> &[f32] {
        &self.buf
    }

    pub fn mark_extracted(&mut self) {
        self.samples_since_last_extract = 0;
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.samples_since_last_extract = 0;
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

impl Default for EmbeddingWindow {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_starts_empty_and_does_not_extract() {
        let w = EmbeddingWindow::new();
        assert_eq!(w.len(), 0);
        assert!(!w.should_extract());
    }

    #[test]
    fn window_extracts_once_min_samples_reached() {
        let mut w = EmbeddingWindow::new();
        // Push less than MIN -- should not extract.
        w.push(&vec![0.1f32; MIN_WINDOW_SAMPLES_16K - 1]);
        assert!(!w.should_extract());
        // Pushing one more sample crosses MIN and ALSO crosses
        // REEXTRACT_INTERVAL since MIN (8000) > REEXTRACT (3200).
        w.push(&[0.1f32]);
        assert!(w.should_extract());
    }

    #[test]
    fn mark_extracted_resets_interval() {
        let mut w = EmbeddingWindow::new();
        w.push(&vec![0.1f32; MIN_WINDOW_SAMPLES_16K + 1000]);
        assert!(w.should_extract());
        w.mark_extracted();
        assert!(!w.should_extract());
        // Another 3200 samples brings it back.
        w.push(&vec![0.1f32; REEXTRACT_INTERVAL_SAMPLES_16K]);
        assert!(w.should_extract());
    }

    #[test]
    fn window_caps_at_max() {
        let mut w = EmbeddingWindow::new();
        w.push(&vec![1.0f32; MAX_WINDOW_SAMPLES_16K + 5_000]);
        assert_eq!(w.len(), MAX_WINDOW_SAMPLES_16K);
    }

    #[test]
    fn reset_clears_everything() {
        let mut w = EmbeddingWindow::new();
        w.push(&vec![0.1f32; 10_000]);
        w.reset();
        assert_eq!(w.len(), 0);
        assert!(!w.should_extract());
    }
}
