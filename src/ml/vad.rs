//! Silero VAD wrapper with persistent GRU state.
//!
//! Silero VAD is a small (~2 MB) ONNX model that outputs a speech
//! probability in [0, 1] for each 32 ms / 512-sample frame at 16 kHz.
//! The model is stateful: it contains a GRU whose hidden state must
//! persist across calls. Recreating the session or resetting the state
//! per frame causes the VAD output to flicker wildly on clean speech.
//! See `.claude/rules/ml-inference.md` and `docs/voicegate/research.md`
//! section 2 for the full rules.
//!
//! Tensor shapes (confirmed via `tests/test_ort_smoke.rs`):
//!
//! | Name     | Kind   | Type | Shape            |
//! |----------|--------|------|------------------|
//! | `input`  | input  | f32  | `[B, T]`         |
//! | `state`  | input  | f32  | `[2, B, 128]`    |
//! | `sr`     | input  | i64  | `[]` (scalar!)   |
//! | `output` | output | f32  | `[B, 1]`         |
//! | `stateN` | output | f32  | `[2, B, 128]`    |
//!
//! The `sr` scalar is the trickiest one: it is a 0-dimensional tensor,
//! not a 1-element vector. We construct it with `Tensor::from_array(((), vec![16000i64]))`
//! where `()` is the empty shape tuple.

use std::path::Path;

use ort::session::Session;
use ort::value::{DynValue, Tensor};

use crate::VoiceGateError;

/// Silero VAD input is always exactly 512 samples at 16 kHz (32 ms).
pub const VAD_CHUNK_SAMPLES: usize = 512;

/// Sample rate the model was trained at.
pub const VAD_SAMPLE_RATE: i64 = 16_000;

/// Size of the GRU hidden state: shape `[2, 1, 128]` = 256 floats.
const VAD_STATE_NUMEL: usize = 256;

/// Default speech probability threshold. Silero VAD's author recommends
/// 0.5 as a good starting point; see the snakers4/silero-vad README.
pub const DEFAULT_THRESHOLD: f32 = 0.5;

pub struct SileroVad {
    session: Session,
    /// Persistent GRU hidden state. Shape [2, 1, 128] = 256 floats.
    /// Flattened for easy tensor construction.
    state: Vec<f32>,
    /// Speech-probability threshold for `is_speech()`. Tunable.
    pub threshold: f32,
}

impl SileroVad {
    /// Load the Silero VAD model from `model_path`.
    ///
    /// Fails if the file does not exist, is not a valid ONNX model, or if
    /// the ONNX Runtime shared library cannot be loaded (LoadLibrary).
    pub fn load(model_path: &Path) -> Result<Self, VoiceGateError> {
        if !model_path.exists() {
            return Err(VoiceGateError::Ml(format!(
                "silero_vad.onnx not found at {}. Run `make models` to download it.",
                model_path.display()
            )));
        }

        let session = Session::builder()
            .map_err(|e| VoiceGateError::Ml(format!("Session::builder: {e}")))?
            .commit_from_file(model_path)
            .map_err(|e| VoiceGateError::Ml(format!("load silero_vad.onnx: {e}")))?;

        Ok(Self {
            session,
            state: vec![0.0; VAD_STATE_NUMEL],
            threshold: DEFAULT_THRESHOLD,
        })
    }

    /// Run VAD on exactly `VAD_CHUNK_SAMPLES` (512) samples at 16 kHz.
    /// Returns the raw speech probability in [0, 1].
    ///
    /// The call updates the persistent GRU state in place. Do NOT call
    /// `reset()` between consecutive frames unless there has been a real
    /// silence gap longer than ~500 ms.
    pub fn prob(&mut self, audio_16k: &[f32]) -> Result<f32, VoiceGateError> {
        if audio_16k.len() != VAD_CHUNK_SAMPLES {
            return Err(VoiceGateError::Ml(format!(
                "SileroVad::prob: expected {VAD_CHUNK_SAMPLES} samples, got {}",
                audio_16k.len()
            )));
        }

        // Build the three input tensors.
        // The `input` tensor is shape [1, 512] -- we need a Vec<f32> with
        // 512 elements and a shape [1, 512]. Copy the slice into a Vec so
        // ort owns the buffer for the duration of the run.
        let input_vec: Vec<f32> = audio_16k.to_vec();
        let input_tensor: Tensor<f32> =
            Tensor::from_array(([1_usize, VAD_CHUNK_SAMPLES], input_vec))
                .map_err(|e| VoiceGateError::Ml(format!("build input tensor: {e}")))?;

        // `state` tensor is shape [2, 1, 128] with 256 floats. Clone the
        // state vec so ort takes ownership of a fresh Vec; we will copy
        // `stateN` back into self.state after the run.
        let state_vec: Vec<f32> = self.state.clone();
        let state_tensor: Tensor<f32> = Tensor::from_array(([2_usize, 1, 128], state_vec))
            .map_err(|e| VoiceGateError::Ml(format!("build state tensor: {e}")))?;

        // `sr` tensor is a SCALAR (0-D) i64 with value 16000. The `()`
        // shape is how ort 2.x expresses 0-dim.
        let sr_tensor: Tensor<i64> = Tensor::from_array(((), vec![VAD_SAMPLE_RATE]))
            .map_err(|e| VoiceGateError::Ml(format!("build sr tensor: {e}")))?;

        let outputs = self
            .session
            .run(ort::inputs! {
                "input" => input_tensor,
                "state" => state_tensor,
                "sr" => sr_tensor,
            })
            .map_err(|e| VoiceGateError::Ml(format!("session.run: {e}")))?;

        // Read the speech probability from `output` (shape [1, 1] -> one f32).
        let output_value: &DynValue = &outputs["output"];
        let (_shape, output_data) = output_value
            .try_extract_tensor::<f32>()
            .map_err(|e| VoiceGateError::Ml(format!("extract output tensor: {e}")))?;
        if output_data.is_empty() {
            return Err(VoiceGateError::Ml(
                "silero vad produced an empty output tensor".into(),
            ));
        }
        let speech_prob = output_data[0];

        // Copy `stateN` back into self.state for the next call.
        let state_value: &DynValue = &outputs["stateN"];
        let (_, new_state) = state_value
            .try_extract_tensor::<f32>()
            .map_err(|e| VoiceGateError::Ml(format!("extract stateN tensor: {e}")))?;
        if new_state.len() != VAD_STATE_NUMEL {
            return Err(VoiceGateError::Ml(format!(
                "stateN has unexpected length {} (want {VAD_STATE_NUMEL})",
                new_state.len()
            )));
        }
        self.state.copy_from_slice(new_state);

        Ok(speech_prob)
    }

    /// `prob(audio) > self.threshold`.
    pub fn is_speech(&mut self, audio_16k: &[f32]) -> Result<bool, VoiceGateError> {
        Ok(self.prob(audio_16k)? >= self.threshold)
    }

    /// Zero the GRU hidden state. Use when a real silence gap >500 ms has
    /// occurred or at the start of enrollment.
    pub fn reset(&mut self) {
        for s in self.state.iter_mut() {
            *s = 0.0;
        }
    }
}
