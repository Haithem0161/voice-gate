//! SPSC ring buffer for f32 audio samples.
//!
//! Backing store is `ringbuf::HeapRb<f32>`. The producer and consumer halves
//! are cpal-callback-safe (no lock, no alloc per push/pop). See
//! `.claude/rules/audio-io.md` for the capacity rationale.

use ringbuf::traits::Split;
use ringbuf::{HeapCons, HeapProd, HeapRb};

/// Producer half of the audio ring buffer. Callback-safe.
pub type AudioProducer = HeapProd<f32>;

/// Consumer half of the audio ring buffer. Callback-safe.
pub type AudioConsumer = HeapCons<f32>;

/// 3 seconds at 48 kHz. One of these per queue (input + output) = ~1.1 MB total.
pub const RING_CAPACITY_SAMPLES: usize = 3 * 48_000;

/// Allocate a new SPSC ring buffer and return its producer/consumer halves.
pub fn new_audio_ring(capacity_samples: usize) -> (AudioProducer, AudioConsumer) {
    HeapRb::<f32>::new(capacity_samples).split()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::traits::{Consumer, Producer};

    #[test]
    fn push_pop_preserves_sample_order() {
        let (mut prod, mut cons) = new_audio_ring(16);
        let input: Vec<f32> = (0..8).map(|i| i as f32 * 0.125).collect();
        let pushed = prod.push_slice(&input);
        assert_eq!(pushed, 8);

        let mut out = [0.0f32; 8];
        let popped = cons.pop_slice(&mut out);
        assert_eq!(popped, 8);
        assert_eq!(&out[..], &input[..]);
    }

    #[test]
    fn ring_capacity_constant_matches_3_seconds() {
        assert_eq!(RING_CAPACITY_SAMPLES, 144_000);
    }
}
