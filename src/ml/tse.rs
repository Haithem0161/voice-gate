//! Target Speaker Extraction (TSE) ONNX model wrapper.
//!
//! Wraps a VoiceFilterLite ONNX model that predicts a time-frequency
//! mask conditioned on the enrolled speaker's embedding. The mask is
//! applied to the input magnitude spectrogram to isolate the target
//! speaker's voice.
//!
//! ONNX inputs:
//!   - `magnitude`: f32[1, T, 513] -- input magnitude spectrogram
//!   - `embedding`: f32[1, 256]    -- enrolled speaker embedding (L2-normalized)
//!   - `state`:     f32[2, 1, 128] -- GRU hidden state
//!
//! ONNX outputs:
//!   - `mask`:   f32[1, T, 513] -- predicted mask in [0, 1]
//!   - `stateN`: f32[2, 1, 128] -- updated GRU state
//!
//! The GRU state persists across calls (same pattern as Silero VAD).
//! Reset on silence gaps > 500 ms or enrollment changes.

use std::path::Path;

use ort::session::Session;
use ort::value::{DynValue, Tensor};

use crate::ml::embedding::EMBEDDING_DIM;
use crate::ml::stft::TSE_NUM_BINS;
use crate::VoiceGateError;

/// GRU state shape: [2 layers, 1 batch, 128 hidden] = 256 floats.
/// GRU state: 2 layers x batch_size(1) x hidden_size(128).
const GRU_STATE_NUMEL: usize = 256;

pub struct TseModel {
    session: Session,
    /// Persistent GRU hidden state, shape [2, 1, 128] flattened.
    gru_state: Vec<f32>,
}

impl TseModel {
    /// Load the TSE model from an ONNX file.
    pub fn load(model_path: &Path) -> Result<Self, VoiceGateError> {
        if !model_path.exists() {
            return Err(VoiceGateError::ModelNotFound(format!(
                "TSE model not found at {}. See training/README.md for instructions.",
                model_path.display()
            )));
        }

        let session = Session::builder()
            .map_err(|e| VoiceGateError::Ml(format!("Session::builder: {e}")))?
            .commit_from_file(model_path)
            .map_err(|e| VoiceGateError::Ml(format!("load TSE model: {e}")))?;

        Ok(Self {
            session,
            gru_state: vec![0.0; GRU_STATE_NUMEL],
        })
    }

    /// Predict a time-frequency mask for the target speaker.
    ///
    /// - `magnitudes`: flat `[num_frames * TSE_NUM_BINS]` magnitude spectrogram
    /// - `num_frames`: number of STFT frames (typically 3)
    /// - `embedding`: L2-normalized speaker embedding, length EMBEDDING_DIM
    ///
    /// Returns a flat `[num_frames * TSE_NUM_BINS]` mask with values in [0, 1].
    /// Updates the internal GRU state.
    pub fn predict_mask(
        &mut self,
        magnitudes: &[f32],
        num_frames: usize,
        embedding: &[f32],
    ) -> Result<Vec<f32>, VoiceGateError> {
        debug_assert_eq!(magnitudes.len(), num_frames * TSE_NUM_BINS);
        debug_assert_eq!(embedding.len(), EMBEDDING_DIM);

        // Build magnitude tensor: [1, T, F].
        let mag_vec: Vec<f32> = magnitudes.to_vec();
        let mag_tensor: Tensor<f32> =
            Tensor::from_array(([1_usize, num_frames, TSE_NUM_BINS], mag_vec))
                .map_err(|e| VoiceGateError::Ml(format!("build magnitude tensor: {e}")))?;

        // Build embedding tensor: [1, E].
        let emb_vec: Vec<f32> = embedding.to_vec();
        let emb_tensor: Tensor<f32> =
            Tensor::from_array(([1_usize, EMBEDDING_DIM], emb_vec))
                .map_err(|e| VoiceGateError::Ml(format!("build embedding tensor: {e}")))?;

        // Build state tensor: [2, 1, 128]. Clone so ort takes ownership.
        let state_vec: Vec<f32> = self.gru_state.clone();
        let state_tensor: Tensor<f32> =
            Tensor::from_array(([2_usize, 1_usize, 128_usize], state_vec))
                .map_err(|e| VoiceGateError::Ml(format!("build state tensor: {e}")))?;

        // Run inference.
        let outputs = self
            .session
            .run(ort::inputs! {
                "magnitude" => mag_tensor,
                "embedding" => emb_tensor,
                "state" => state_tensor,
            })
            .map_err(|e| VoiceGateError::Ml(format!("TSE session.run: {e}")))?;

        // Extract mask: [1, T, F] -> flat [T * F].
        let mask_value: &DynValue = &outputs["mask"];
        let (_shape, mask_data) = mask_value
            .try_extract_tensor::<f32>()
            .map_err(|e| VoiceGateError::Ml(format!("extract mask tensor: {e}")))?;

        let expected_len = num_frames * TSE_NUM_BINS;
        if mask_data.len() != expected_len {
            return Err(VoiceGateError::Ml(format!(
                "mask has unexpected length {} (want {expected_len})",
                mask_data.len()
            )));
        }
        let mask = mask_data.to_vec();

        // Update GRU state from stateN output.
        let state_value: &DynValue = &outputs["stateN"];
        let (_, new_state) = state_value
            .try_extract_tensor::<f32>()
            .map_err(|e| VoiceGateError::Ml(format!("extract stateN tensor: {e}")))?;
        if new_state.len() != GRU_STATE_NUMEL {
            return Err(VoiceGateError::Ml(format!(
                "stateN has unexpected length {} (want {GRU_STATE_NUMEL})",
                new_state.len()
            )));
        }
        self.gru_state.copy_from_slice(new_state);

        Ok(mask)
    }

    /// Reset the GRU hidden state to zeros. Call on silence gaps > 500 ms,
    /// enrollment changes, or when TSE mode is toggled.
    pub fn reset_state(&mut self) {
        self.gru_state.fill(0.0);
    }
}
