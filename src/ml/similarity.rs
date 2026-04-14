//! Cosine similarity, L2 normalization, and speaker verifier with EMA
//! smoothing.
//!
//! The gate runs on `current_score = EMA(cosine(enrolled, live))`. EMA
//! alpha defaults to 0.3 (see `docs/voicegate/research.md` section 3).
//! Pass 1 gap G-007: `SpeakerVerifier::update` MUST NOT be called during
//! VAD-inactive frames -- that would drift `current_score` towards the
//! similarity of silence against the enrolled centroid and cause
//! spurious gate closes. The caller (Phase 4+ pipeline) is responsible
//! for only calling `update` when the VAD fires.

use crate::ml::embedding::EMBEDDING_DIM;

/// Default verification threshold (PRD section 5.7 slider default).
/// Cosine similarity must exceed this for the gate to open.
pub const DEFAULT_THRESHOLD: f32 = 0.70;

/// Default EMA smoothing factor. alpha=0.3 means a single outlier moves
/// the running score by 30% of the delta; ~5 updates to settle after a
/// speaker change at 200 ms extraction cadence (~1 s transition).
pub const DEFAULT_EMA_ALPHA: f32 = 0.3;

/// In-place L2 normalization. If the input has near-zero norm the
/// function leaves it unchanged (division-by-zero guard) and returns
/// early. The epsilon here is 1e-12 which is below any real embedding's
/// norm (WeSpeaker outputs are typically in [0.5, 3.0] range pre-norm).
pub fn l2_normalize(v: &mut [f32]) {
    let norm_sq: f32 = v.iter().map(|x| x * x).sum();
    let norm = norm_sq.sqrt();
    if norm < 1e-12 {
        return;
    }
    for x in v.iter_mut() {
        *x /= norm;
    }
}

/// Cosine similarity on two L2-normalized vectors of equal length.
/// When inputs are unit vectors this reduces to the dot product, which
/// is what we actually compute. Returns a value in [-1, 1].
///
/// Note: this function assumes inputs are already L2-normalized. Calling
/// it on non-normalized vectors produces the dot product, which is
/// proportional to cosine similarity but scaled by the norms. The
/// `SpeakerVerifier` below enforces normalization at construction and
/// via `l2_normalize` on every live embedding.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Result of a verification step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VerifyResult {
    /// `current_score >= threshold`. The gate should be open.
    Match(f32),
    /// `current_score < threshold`. The gate should be closed.
    NoMatch(f32),
}

impl VerifyResult {
    pub fn score(&self) -> f32 {
        match self {
            VerifyResult::Match(s) | VerifyResult::NoMatch(s) => *s,
        }
    }

    pub fn is_match(&self) -> bool {
        matches!(self, VerifyResult::Match(_))
    }
}

/// Stateful speaker verifier with EMA-smoothed similarity.
pub struct SpeakerVerifier {
    /// L2-normalized enrolled centroid. Length = `EMBEDDING_DIM`.
    enrolled: Vec<f32>,
    /// Verification threshold (gate opens when current_score >= this).
    pub threshold: f32,
    /// EMA smoothing factor, in [0, 1].
    pub ema_alpha: f32,
    /// Running EMA-smoothed similarity score.
    current_score: f32,
}

impl SpeakerVerifier {
    /// Construct a verifier from an already-L2-normalized enrolled
    /// embedding. If `enrolled` is not normalized, cosine similarity
    /// will be wrong; callers should run `l2_normalize` before this.
    pub fn new(enrolled: Vec<f32>, threshold: f32, ema_alpha: f32) -> Self {
        debug_assert_eq!(enrolled.len(), EMBEDDING_DIM);
        Self {
            enrolled,
            threshold,
            ema_alpha,
            current_score: 0.0,
        }
    }

    /// Update with a new live embedding (which must be L2-normalized)
    /// and return the new `VerifyResult`.
    ///
    /// **G-007 (phase-02 section 6.1):** the caller MUST NOT call this
    /// during VAD-inactive frames. During silence, leave `current_score`
    /// unchanged so the gate's hold-time can apply and the meter does
    /// not flicker.
    pub fn update(&mut self, live: &[f32]) -> VerifyResult {
        debug_assert_eq!(live.len(), EMBEDDING_DIM);
        let raw = cosine_similarity(&self.enrolled, live);
        self.current_score = self.ema_alpha * raw + (1.0 - self.ema_alpha) * self.current_score;

        if self.current_score >= self.threshold {
            VerifyResult::Match(self.current_score)
        } else {
            VerifyResult::NoMatch(self.current_score)
        }
    }

    /// Current EMA-smoothed similarity score.
    pub fn current_score(&self) -> f32 {
        self.current_score
    }

    /// Reset the EMA state to 0. Call only at enrollment boundaries or
    /// on long silence gaps (>500 ms) when G-007's "don't update on
    /// silence" rule no longer applies.
    pub fn reset(&mut self) {
        self.current_score = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_vec(i: usize, n: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; n];
        v[i] = 1.0;
        v
    }

    #[test]
    fn l2_normalize_scales_to_unit() {
        let mut v = vec![3.0, 4.0, 0.0];
        l2_normalize(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6);
        assert!((v[1] - 0.8).abs() < 1e-6);
        assert!((v[2] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn l2_normalize_preserves_zero() {
        let mut v = vec![0.0f32; 4];
        l2_normalize(&mut v);
        for &x in &v {
            assert_eq!(x, 0.0);
        }
    }

    #[test]
    fn cosine_similarity_parallel_is_one() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_is_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_antiparallel_is_negative_one() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![-1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn verifier_matches_same_enrolled() {
        // Use a unit vector at index 0 as the enrolled centroid.
        let enrolled = unit_vec(0, EMBEDDING_DIM);
        let mut v = SpeakerVerifier::new(enrolled.clone(), 0.7, 0.3);

        // Feed the same unit vector -- raw cosine is 1.0 each time.
        // After enough updates the EMA converges to 1.0.
        for _ in 0..20 {
            v.update(&enrolled);
        }
        assert!(v.current_score() > 0.95);
        assert!(v.update(&enrolled).is_match());
    }

    #[test]
    fn verifier_rejects_orthogonal() {
        let enrolled = unit_vec(0, EMBEDDING_DIM);
        let stranger = unit_vec(1, EMBEDDING_DIM); // orthogonal, cosine = 0
        let mut v = SpeakerVerifier::new(enrolled, 0.7, 0.3);
        for _ in 0..20 {
            v.update(&stranger);
        }
        // Should be near 0, well below the 0.7 threshold.
        assert!(v.current_score().abs() < 0.05);
        assert!(!v.update(&stranger).is_match());
    }

    #[test]
    fn ema_smoothing_converges_at_expected_rate() {
        // With alpha=0.3 and a raw score of 1.0 from starting score 0.0,
        // after one update: 0.3 * 1.0 + 0.7 * 0.0 = 0.3
        // after two updates: 0.3 * 1.0 + 0.7 * 0.3 = 0.51
        // after three updates: 0.3 * 1.0 + 0.7 * 0.51 = 0.657
        let enrolled = unit_vec(0, EMBEDDING_DIM);
        let mut v = SpeakerVerifier::new(enrolled.clone(), 0.7, 0.3);

        v.update(&enrolled);
        assert!((v.current_score() - 0.3).abs() < 1e-5);

        v.update(&enrolled);
        assert!((v.current_score() - 0.51).abs() < 1e-5);

        v.update(&enrolled);
        assert!((v.current_score() - 0.657).abs() < 1e-5);
    }

    #[test]
    fn reset_zeroes_current_score() {
        let enrolled = unit_vec(0, EMBEDDING_DIM);
        let mut v = SpeakerVerifier::new(enrolled.clone(), 0.7, 0.3);
        for _ in 0..10 {
            v.update(&enrolled);
        }
        assert!(v.current_score() > 0.5);
        v.reset();
        assert_eq!(v.current_score(), 0.0);
    }
}
