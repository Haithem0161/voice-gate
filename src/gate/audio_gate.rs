use crate::audio::resampler::INPUT_CHUNK_SAMPLES;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateState {
    Open,
    Closed,
    Opening { progress: usize },
    Closing { progress: usize },
}

impl GateState {
    pub fn as_u8(&self) -> u8 {
        match self {
            GateState::Closed => 0,
            GateState::Opening { .. } => 1,
            GateState::Open => 2,
            GateState::Closing { .. } => 3,
        }
    }
}

pub struct AudioGate {
    state: GateState,
    hold_frames: u32,
    crossfade_samples: usize,
    frames_since_match: u32,
}

impl AudioGate {
    pub fn new(hold_frames: u32, crossfade_samples: usize) -> Self {
        Self {
            state: GateState::Closed,
            hold_frames,
            crossfade_samples,
            frames_since_match: hold_frames,
        }
    }

    pub fn process(&mut self, frame: &mut [f32], is_match: bool) {
        if is_match {
            self.frames_since_match = 0;
        } else {
            self.frames_since_match = self.frames_since_match.saturating_add(1);
        }

        let should_be_open = self.frames_since_match < self.hold_frames;

        match self.state {
            GateState::Open => {
                if !should_be_open {
                    self.state = GateState::Closing { progress: 0 };
                    self.apply_fade_out(frame, 0);
                }
            }
            GateState::Closed => {
                if should_be_open {
                    self.state = GateState::Opening { progress: 0 };
                    self.apply_fade_in(frame, 0);
                } else {
                    frame.fill(0.0);
                }
            }
            GateState::Opening { progress } => {
                if should_be_open {
                    self.apply_fade_in(frame, progress);
                } else {
                    let current_gain = progress as f32 / self.crossfade_samples.max(1) as f32;
                    let equivalent_close_progress =
                        ((1.0 - current_gain) * self.crossfade_samples as f32) as usize;
                    self.state = GateState::Closing {
                        progress: equivalent_close_progress,
                    };
                    self.apply_fade_out(frame, equivalent_close_progress);
                }
            }
            GateState::Closing { progress } => {
                if should_be_open {
                    let current_gain = 1.0 - progress as f32 / self.crossfade_samples.max(1) as f32;
                    let equivalent_open_progress =
                        (current_gain * self.crossfade_samples as f32) as usize;
                    self.state = GateState::Opening {
                        progress: equivalent_open_progress,
                    };
                    self.apply_fade_in(frame, equivalent_open_progress);
                } else {
                    self.apply_fade_out(frame, progress);
                }
            }
        }
    }

    fn apply_fade_in(&mut self, frame: &mut [f32], start_progress: usize) {
        let cs = self.crossfade_samples.max(1);
        let mut p = start_progress;
        for sample in frame.iter_mut() {
            if p >= cs {
                break;
            }
            let gain = p as f32 / cs as f32;
            *sample *= gain;
            p += 1;
        }
        if p >= cs {
            self.state = GateState::Open;
        } else {
            self.state = GateState::Opening { progress: p };
        }
    }

    fn apply_fade_out(&mut self, frame: &mut [f32], start_progress: usize) {
        let cs = self.crossfade_samples.max(1);
        let mut p = start_progress;
        for sample in frame.iter_mut() {
            if p >= cs {
                *sample = 0.0;
            } else {
                let gain = 1.0 - p as f32 / cs as f32;
                *sample *= gain;
                p += 1;
            }
        }
        if p >= cs {
            self.state = GateState::Closed;
        } else {
            self.state = GateState::Closing { progress: p };
        }
    }

    pub fn state(&self) -> GateState {
        self.state
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state, GateState::Open)
    }

    pub fn force_open(&mut self) {
        self.state = GateState::Open;
        self.frames_since_match = 0;
    }

    pub fn force_closed(&mut self) {
        self.state = GateState::Closed;
        self.frames_since_match = self.hold_frames;
    }
}

const _: () = {
    assert!(INPUT_CHUNK_SAMPLES == 1536);
};
