//! Enrollment session: VAD-segmented speech collector + centroid extractor.
//!
//! The flow is:
//!
//! 1. Caller constructs an `EnrollmentSession` with already-loaded VAD
//!    and ECAPA (WeSpeaker) models.
//! 2. Caller pushes raw 16 kHz f32 audio via `push_audio` as many times
//!    as needed, until `is_ready()` returns true or the caller chooses
//!    to stop.
//! 3. Caller calls `finalize(self)` which:
//!    1. Walks the accumulated audio in 512-sample chunks, running
//!       Silero VAD over each chunk.
//!    2. Builds contiguous "speech run" buffers; emits each run as
//!       a SEGMENT_SAMPLES_16K (3 s) segment once it reaches that
//!       length. Shorter runs are discarded.
//!    3. Requires at least MIN_SEGMENTS (5) complete segments.
//!    4. Extracts one embedding per segment via EcapaTdnn.
//!    5. Averages the embeddings component-wise and L2-normalizes.
//!    6. Returns the centroid as a Vec<f32> of length EMBEDDING_DIM.
//!
//! **Privacy invariant** (PRD section 2.2 + phase-03 section 5.4):
//! DO NOT write `accumulated_16k` or `speech_segments` to disk at any
//! point. Enrollment captures live audio transiently; only the final
//! 256-float centroid ever leaves the process's memory.

use crate::ml::embedding::{EcapaTdnn, EMBEDDING_DIM};
use crate::ml::similarity::l2_normalize;
use crate::ml::vad::{SileroVad, VAD_CHUNK_SAMPLES};

/// Minimum raw audio duration before `is_ready()` returns true.
/// 20 seconds gives enough margin for 5 x 3-second speech segments
/// with some silence buffer.
pub const MIN_ENROLL_DURATION_SAMPLES_16K: usize = 16_000 * 20;

/// Length of one enrollment segment at 16 kHz. 3 seconds gives the
/// speaker embedding model enough context to be stable without tying
/// the enrollment to a single utterance.
pub const SEGMENT_SAMPLES_16K: usize = 16_000 * 3;

/// Minimum number of 3-second speech segments required to finalize.
pub const MIN_SEGMENTS: usize = 5;

/// Maximum number of segments to keep. Extra segments beyond this are
/// dropped to bound the amount of ONNX inference `finalize` does.
pub const MAX_SEGMENTS: usize = 12;

pub struct EnrollmentSession {
    vad: SileroVad,
    ecapa: EcapaTdnn,
    accumulated_16k: Vec<f32>,
}

impl EnrollmentSession {
    pub fn new(vad: SileroVad, ecapa: EcapaTdnn) -> Self {
        Self {
            vad,
            ecapa,
            accumulated_16k: Vec::new(),
        }
    }

    /// Append raw 16 kHz f32 audio to the session. No VAD filtering is
    /// applied here -- the full stream is saved (in memory only) so that
    /// `finalize` can re-run VAD with a fresh GRU state over all of it.
    pub fn push_audio(&mut self, audio_16k: &[f32]) {
        self.accumulated_16k.extend_from_slice(audio_16k);
    }

    /// How many seconds of RAW audio have been pushed so far.
    pub fn duration_seconds(&self) -> f32 {
        self.accumulated_16k.len() as f32 / 16_000.0
    }

    /// Whether enough data has been gathered to finalize. This is a hint,
    /// not a hard gate -- `finalize` will also check segment count and
    /// return an error if too few segments were extracted.
    pub fn is_ready(&self) -> bool {
        self.accumulated_16k.len() >= MIN_ENROLL_DURATION_SAMPLES_16K
    }

    /// Walk `accumulated_16k`, segment via VAD, extract one embedding per
    /// segment, average, L2-normalize, return the centroid.
    ///
    /// Consumes self because the session is a one-shot: once finalized,
    /// the audio buffer is dropped and the caller should not be tempted
    /// to reuse it.
    pub fn finalize(mut self) -> anyhow::Result<Vec<f32>> {
        // Fresh GRU state for the finalize pass.
        self.vad.reset();

        let segments = segment_by_vad(&mut self.vad, &self.accumulated_16k)?;
        // Drop accumulated audio as soon as segmentation is done.
        // Privacy: see module-level comment.
        drop(self.accumulated_16k);

        if segments.len() < MIN_SEGMENTS {
            anyhow::bail!(
                "enrollment produced only {} speech segments, need at least {MIN_SEGMENTS}. \
                 Speak for longer or more clearly. Each segment needs {SEGMENT_SAMPLES_16K} \
                 samples ({} s) of continuous VAD-active speech.",
                segments.len(),
                SEGMENT_SAMPLES_16K as f32 / 16_000.0,
            );
        }

        // Cap at MAX_SEGMENTS to bound inference cost. Keeping the first N
        // is fine because the enrollment script asks the user to read a
        // pangram passage -- the ordering has no meaningful bias.
        let kept: Vec<Vec<f32>> = segments.into_iter().take(MAX_SEGMENTS).collect();
        let num_kept = kept.len();
        tracing::info!(
            segments = num_kept,
            "enrollment finalize: extracting {num_kept} embeddings"
        );

        // Extract one embedding per segment.
        let mut centroid = vec![0.0f32; EMBEDDING_DIM];
        for (i, seg) in kept.iter().enumerate() {
            let emb = self
                .ecapa
                .extract(seg)
                .map_err(|e| anyhow::anyhow!("extract embedding for segment {i}: {e}"))?;
            debug_assert_eq!(emb.len(), EMBEDDING_DIM);
            for (c, v) in centroid.iter_mut().zip(emb.iter()) {
                *c += *v;
            }
        }

        // Average.
        let n = num_kept as f32;
        for c in centroid.iter_mut() {
            *c /= n;
        }

        // L2-normalize so the returned vector is a proper unit embedding,
        // suitable for direct cosine similarity against future live
        // embeddings. This matches the `enrolled` contract of
        // SpeakerVerifier::new.
        l2_normalize(&mut centroid);

        Ok(centroid)
    }
}

/// Walk `audio` in 512-sample chunks, run VAD over each, and return the
/// list of 3-second contiguous-speech segments. Shorter runs are discarded.
///
/// This is factored out so it can be unit-tested with synthetic inputs
/// and a mocked VAD (the real VAD needs a real ONNX session, which only
/// works at the integration-test level).
fn segment_by_vad(vad: &mut SileroVad, audio: &[f32]) -> anyhow::Result<Vec<Vec<f32>>> {
    let mut segments: Vec<Vec<f32>> = Vec::new();
    let mut current_run: Vec<f32> = Vec::with_capacity(SEGMENT_SAMPLES_16K);

    let num_chunks = audio.len() / VAD_CHUNK_SAMPLES;
    for i in 0..num_chunks {
        let start = i * VAD_CHUNK_SAMPLES;
        let chunk = &audio[start..start + VAD_CHUNK_SAMPLES];
        let speech = vad
            .is_speech(chunk)
            .map_err(|e| anyhow::anyhow!("vad.is_speech: {e}"))?;
        if speech {
            current_run.extend_from_slice(chunk);
            // If the current run has reached one full segment's worth,
            // emit it and start over. We DON'T try to pack multiple
            // segments into a single run -- each segment should come
            // from a distinct speech run for maximum diversity of
            // phonetic content.
            if current_run.len() >= SEGMENT_SAMPLES_16K {
                let mut segment = Vec::with_capacity(SEGMENT_SAMPLES_16K);
                segment.extend_from_slice(&current_run[..SEGMENT_SAMPLES_16K]);
                segments.push(segment);
                current_run.clear();
                if segments.len() >= MAX_SEGMENTS {
                    break;
                }
            }
        } else {
            // Silence -- drop the current partial run. This is the
            // natural speech-pause behavior: pauses reset the segment
            // accumulator, which means short runs never accidentally
            // concatenate into a "segment" split across a long gap.
            current_run.clear();
        }
    }

    Ok(segments)
}

// Compile-time invariant checks. These are stronger than unit tests
// because they fail at build time rather than test time. We cannot
// unit-test `finalize()` directly because it requires a loaded
// SileroVad + EcapaTdnn session. The real tests for the segment ->
// embedding -> centroid pipeline live in tests/test_enrollment.rs
// which uses the actual ONNX models and fixture WAVs.
const _: () = {
    assert!(MIN_ENROLL_DURATION_SAMPLES_16K == 16_000 * 20);
    assert!(SEGMENT_SAMPLES_16K == 16_000 * 3);
    assert!(MIN_SEGMENTS >= 1);
    assert!(MAX_SEGMENTS >= MIN_SEGMENTS);
};
