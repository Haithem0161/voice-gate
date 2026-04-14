use voicegate::gate::audio_gate::{AudioGate, GateState};

const FRAME: usize = 1536;
const CROSSFADE: usize = 240;
const HOLD: u32 = 5;

fn continuous_sine(
    start_sample: usize,
    len: usize,
    freq_hz: f32,
    sample_rate: f32,
    amplitude: f32,
) -> Vec<f32> {
    (0..len)
        .map(|i| {
            let t = (start_sample + i) as f32 / sample_rate;
            amplitude * (2.0 * std::f32::consts::PI * freq_hz * t).sin()
        })
        .collect()
}

#[test]
fn test_gate_passthrough_open() {
    let mut gate = AudioGate::new(HOLD, CROSSFADE);
    gate.force_open();
    let original = continuous_sine(0, FRAME, 440.0, 48000.0, 0.5);
    let mut frame = original.clone();
    gate.process(&mut frame, true);
    assert_eq!(frame, original);
    assert!(gate.is_open());
}

#[test]
fn test_gate_silence_closed() {
    let mut gate = AudioGate::new(HOLD, CROSSFADE);
    gate.force_closed();
    let mut frame = continuous_sine(0, FRAME, 440.0, 48000.0, 0.5);
    gate.process(&mut frame, false);
    for s in &frame {
        assert_eq!(*s, 0.0);
    }
    assert_eq!(gate.state(), GateState::Closed);
}

#[test]
fn test_gate_crossfade_monotonic() {
    let mut gate = AudioGate::new(HOLD, CROSSFADE);
    for _ in 0..10 {
        let mut f = vec![0.0f32; FRAME];
        gate.process(&mut f, false);
    }
    assert_eq!(gate.state(), GateState::Closed);

    let mut frame = vec![1.0f32; FRAME];
    gate.process(&mut frame, true);

    for i in 1..CROSSFADE.min(FRAME) {
        assert!(
            frame[i] >= frame[i - 1] - f32::EPSILON,
            "fade-in not monotonic at sample {i}: {} < {}",
            frame[i],
            frame[i - 1]
        );
    }
}

#[test]
fn test_gate_no_clicks() {
    let mut gate = AudioGate::new(HOLD, CROSSFADE);
    let mut sample_offset = 0usize;

    // Warm up to Open state
    for _ in 0..10 {
        let mut f = continuous_sine(sample_offset, FRAME, 440.0, 48000.0, 0.5);
        gate.process(&mut f, true);
        sample_offset += FRAME;
    }

    let mut all_output: Vec<f32> = Vec::new();

    // Toggle is_match every 10 frames to create transitions
    for cycle in 0..6 {
        let is_match = cycle % 2 == 0;
        for _ in 0..10 {
            let mut f = continuous_sine(sample_offset, FRAME, 440.0, 48000.0, 0.5);
            gate.process(&mut f, is_match);
            all_output.extend_from_slice(&f);
            sample_offset += FRAME;
        }
    }

    // Compute first differences -- check for hard discontinuities.
    // A 440 Hz sine at 48 kHz with amplitude 0.5 has a max first-difference
    // of about 0.5 * 2*pi*440/48000 = 0.0288. With the crossfade gain
    // modulation we allow up to 0.05.
    let mut max_diff: f32 = 0.0;
    for i in 1..all_output.len() {
        let diff = (all_output[i] - all_output[i - 1]).abs();
        if diff > max_diff {
            max_diff = diff;
        }
    }

    assert!(
        max_diff < 0.05,
        "click detected: max first-difference = {max_diff} (limit 0.05)"
    );
}

#[test]
fn test_gate_hold_time() {
    let mut gate = AudioGate::new(HOLD, CROSSFADE);

    // Open the gate fully
    for _ in 0..20 {
        let mut f = vec![1.0f32; FRAME];
        gate.process(&mut f, true);
    }
    assert!(gate.is_open());

    // Send false frames. With hold_frames=5, frames_since_match goes:
    // frame 0: fsm=1 (< 5 -> open), frame 1: fsm=2, ..., frame 3: fsm=4 (still open)
    // frame 4: fsm=5 (NOT < 5 -> starts closing, crossfade=240 < 1536 so finishes in this frame)
    // So the gate should be Open for the first 4 false frames (indices 0-3).
    for i in 0..4 {
        let mut f = vec![1.0f32; FRAME];
        gate.process(&mut f, false);
        assert!(
            gate.is_open(),
            "gate should be Open during hold frame {i}, got {:?}",
            gate.state()
        );
    }

    // Frame 4 is where the gate starts closing (and completes within this frame)
    let mut f = vec![1.0f32; FRAME];
    gate.process(&mut f, false);
    assert_eq!(
        gate.state(),
        GateState::Closed,
        "gate should be Closed after hold expires"
    );
}

#[test]
fn test_gate_aborted_fade_in() {
    // Use a crossfade larger than one frame so we can interrupt mid-fade
    let big_crossfade = 4000;
    let mut gate = AudioGate::new(HOLD, big_crossfade);
    let mut sample_offset = 0usize;

    // Start closed
    for _ in 0..5 {
        let mut f = vec![0.0f32; FRAME];
        gate.process(&mut f, false);
    }

    // Start opening with one true frame
    let f1 = continuous_sine(sample_offset, FRAME, 440.0, 48000.0, 0.5);
    let mut f1_out = f1.clone();
    gate.process(&mut f1_out, true);
    sample_offset += FRAME;
    assert!(matches!(gate.state(), GateState::Opening { .. }));

    // Continue with true frames to keep hold alive, then false to exhaust hold
    let mut transition_output: Vec<f32> = Vec::new();
    transition_output.extend_from_slice(&f1_out);

    // Send HOLD false frames to exhaust hold_frames
    for _ in 0..(HOLD + 1) {
        let f = continuous_sine(sample_offset, FRAME, 440.0, 48000.0, 0.5);
        let mut f_out = f;
        gate.process(&mut f_out, false);
        transition_output.extend_from_slice(&f_out);
        sample_offset += FRAME;
    }

    // Check no click at any point in the transition
    let mut max_diff: f32 = 0.0;
    for i in 1..transition_output.len() {
        let diff = (transition_output[i] - transition_output[i - 1]).abs();
        if diff > max_diff {
            max_diff = diff;
        }
    }

    assert!(
        max_diff < 0.05,
        "click at aborted fade: max first-difference = {max_diff} (limit 0.05)"
    );
}
