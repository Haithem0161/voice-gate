//! Silero VAD wrapper with persistent GRU state and context buffer.
//!
//! Silero VAD is a small (~2 MB) ONNX model that outputs a speech
//! probability in [0, 1] for each 32 ms / 512-sample frame at 16 kHz.
//! The model is stateful at two levels:
//!
//!   1. **GRU hidden state** -- the 256-float `state` tensor that must
//!      persist across calls. Resetting it per frame causes the VAD
//!      output to flicker wildly on clean speech.
//!   2. **Context buffer** -- the last 64 samples of the previous input
//!      frame, prepended to the next frame's 512 samples to form the
//!      actual `[1, 576]` model input. On the first call the context is
//!      all zeros. This was discovered during Phase 2 step 12 when the
//!      VAD was returning nearly-zero probability on real speech: our
//!      implementation was feeding `[1, 512]` without the context, and
//!      the newest Silero v5 ONNX explicitly concatenates 64 context
//!      samples + 512 new samples at call time. See the reference
//!      `__call__` in `silero-vad/src/silero_vad/utils_vad.py`.
//!
//! See `.claude/rules/ml-inference.md` and `docs/voicegate/research.md`
//! section 2 for the full rules.
//!
//! Tensor shapes (confirmed via `tests/test_ort_smoke.rs` + the
//! reference Python in silero-vad's own repo):
//!
//! | Name     | Kind   | Type | Shape                       |
//! |----------|--------|------|-----------------------------|
//! | `input`  | input  | f32  | `[B, 576]` (64 ctx + 512)   |
//! | `state`  | input  | f32  | `[2, B, 128]`               |
//! | `sr`     | input  | i64  | `[]` (scalar!)              |
//! | `output` | output | f32  | `[B, 1]`                    |
//! | `stateN` | output | f32  | `[2, B, 128]`               |
//!
//! The `sr` scalar is a 0-dimensional tensor, not a 1-element vector.
//! We construct it with `Tensor::from_array(((), vec![16000i64]))` where
//! `()` is the empty-shape tuple.

use std::path::Path;

use ort::session::Session;
use ort::value::{DynValue, Tensor};

use crate::VoiceGateError;

/// Samples per user-facing VAD call. This is the 32 ms frame length at
/// 16 kHz, matching our downstream resampler output. The actual ONNX
/// `input` tensor is 64 + 512 = 576 samples once the context buffer
/// is prepended.
pub const VAD_CHUNK_SAMPLES: usize = 512;

/// Number of samples from the previous frame the Silero VAD v5 model
/// expects as "context" on the next call. 64 samples = 4 ms at 16 kHz.
const VAD_CONTEXT_SAMPLES: usize = 64;

/// Full ONNX input tensor length.
const VAD_INPUT_SAMPLES: usize = VAD_CONTEXT_SAMPLES + VAD_CHUNK_SAMPLES; // 576

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
    /// 64-sample context buffer carried over from the previous call's
    /// input. Zero-initialized on the first call and after `reset()`.
    context: [f32; VAD_CONTEXT_SAMPLES],
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
            return Err(VoiceGateError::ModelNotFound(format!(
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
            context: [0.0; VAD_CONTEXT_SAMPLES],
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
        // The `input` tensor is shape [1, 576] = [1, context(64) + chunk(512)].
        // Prepend self.context and append the new chunk, then save the
        // LAST 64 samples of the concatenated input as the next call's
        // context. See silero-vad/src/silero_vad/utils_vad.py __call__.
        let mut input_vec: Vec<f32> = Vec::with_capacity(VAD_INPUT_SAMPLES);
        input_vec.extend_from_slice(&self.context);
        input_vec.extend_from_slice(audio_16k);
        debug_assert_eq!(input_vec.len(), VAD_INPUT_SAMPLES);

        // Save the tail as the new context BEFORE handing ownership of
        // input_vec to ort (otherwise we'd need to clone).
        self.context
            .copy_from_slice(&input_vec[VAD_INPUT_SAMPLES - VAD_CONTEXT_SAMPLES..]);

        let input_tensor: Tensor<f32> =
            Tensor::from_array(([1_usize, VAD_INPUT_SAMPLES], input_vec))
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

    /// Zero the GRU hidden state AND the context buffer. Use when a real
    /// silence gap >500 ms has occurred or at the start of enrollment.
    pub fn reset(&mut self) {
        for s in self.state.iter_mut() {
            *s = 0.0;
        }
        for c in self.context.iter_mut() {
            *c = 0.0;
        }
    }
}
