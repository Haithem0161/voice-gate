use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::VoiceGateError;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub audio: AudioConfig,
    pub vad: VadConfig,
    pub verification: VerificationConfig,
    pub gate: GateConfig,
    pub enrollment: EnrollmentConfig,
    pub gui: GuiConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AudioConfig {
    pub input_device: String,
    pub output_device: String,
    pub frame_size_ms: u32,
    pub sample_rate: u32,
}

impl AudioConfig {
    pub fn frame_size_samples(&self) -> usize {
        (self.frame_size_ms as usize) * (self.sample_rate as usize) / 1000
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VadConfig {
    pub threshold: f32,
    pub model_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VerificationConfig {
    pub threshold: f32,
    pub ema_alpha: f32,
    pub model_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GateConfig {
    pub hold_frames: u32,
    pub crossfade_ms: f32,
}

impl GateConfig {
    pub fn crossfade_samples(&self, sample_rate: u32) -> usize {
        (self.crossfade_ms * sample_rate as f32 / 1000.0) as usize
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EnrollmentConfig {
    pub profile_path: String,
    pub min_duration_sec: u32,
    pub segment_duration_sec: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GuiConfig {
    pub show_similarity_meter: bool,
    pub show_waveform: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input_device: "default".into(),
            output_device: "auto".into(),
            frame_size_ms: 32,
            sample_rate: 48_000,
        }
    }
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            // 0.15 is more forgiving than Silero's recommended 0.5.
            // Bluetooth mics and quiet environments produce low VAD
            // probabilities even during clear speech. A lower threshold
            // ensures the pipeline detects speech on all mic types.
            threshold: 0.15,
            model_path: "models/silero_vad.onnx".into(),
        }
    }
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            // 0.45 is calibrated for real-world Bluetooth mics where
            // cosine similarity plateaus around 0.5-0.6 due to codec
            // artifacts and lower signal quality. The PRD's 0.70 was
            // based on clean 48kHz wired mic recordings.
            threshold: 0.45,
            ema_alpha: 0.3,
            model_path: "models/wespeaker_resnet34_lm.onnx".into(),
        }
    }
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            hold_frames: 5,
            crossfade_ms: 5.0,
        }
    }
}

impl Default for EnrollmentConfig {
    fn default() -> Self {
        Self {
            profile_path: "auto".into(),
            min_duration_sec: 20,
            segment_duration_sec: 3,
        }
    }
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            show_similarity_meter: true,
            show_waveform: false,
        }
    }
}

impl Config {
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("voicegate").join("config.toml"))
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::default_path()
            .ok_or_else(|| VoiceGateError::Config("could not resolve config dir".into()))?;

        let config = if path.exists() {
            let text = fs::read_to_string(&path)
                .map_err(|e| VoiceGateError::Config(format!("read {}: {}", path.display(), e)))?;
            toml::from_str::<Config>(&text)
                .map_err(|e| VoiceGateError::Config(format!("parse {}: {}", path.display(), e)))?
        } else {
            Config::default()
        };

        config.validate()?;
        Ok(config)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::default_path()
            .ok_or_else(|| VoiceGateError::Config("could not resolve config dir".into()))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                VoiceGateError::Config(format!("mkdir {}: {}", parent.display(), e))
            })?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| VoiceGateError::Config(format!("serialize: {e}")))?;
        fs::write(&path, text)
            .map_err(|e| VoiceGateError::Config(format!("write {}: {}", path.display(), e)))?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), VoiceGateError> {
        if self.audio.frame_size_ms != 32 {
            return Err(VoiceGateError::Config(format!(
                "audio.frame_size_ms = {} is not supported. Only 32 is valid in v1 \
                 (Silero VAD requires 512 samples at 16 kHz, which only aligns at 32 ms).",
                self.audio.frame_size_ms
            )));
        }
        if self.audio.sample_rate != 48_000 {
            return Err(VoiceGateError::Config(format!(
                "audio.sample_rate = {} is not supported. Only 48000 is valid in v1.",
                self.audio.sample_rate
            )));
        }
        if self.gate.crossfade_ms <= 0.0 {
            return Err(VoiceGateError::Config(
                "gate.crossfade_ms must be positive".into(),
            ));
        }
        if self.verification.ema_alpha <= 0.0 || self.verification.ema_alpha > 1.0 {
            return Err(VoiceGateError::Config(
                "verification.ema_alpha must be in (0, 1]".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_validates() {
        let c = Config::default();
        c.validate().expect("default must validate");
        assert_eq!(c.audio.frame_size_samples(), 1536);
    }

    #[test]
    fn non_32_frame_size_is_rejected() {
        let mut c = Config::default();
        c.audio.frame_size_ms = 20;
        assert!(c.validate().is_err());
    }

    #[test]
    fn non_48k_sample_rate_is_rejected() {
        let mut c = Config::default();
        c.audio.sample_rate = 44_100;
        assert!(c.validate().is_err());
    }

    #[test]
    fn frame_size_samples_matches_formula() {
        let c = Config::default();
        assert_eq!(
            c.audio.frame_size_samples(),
            (c.audio.frame_size_ms as usize) * (c.audio.sample_rate as usize) / 1000
        );
    }

    #[test]
    fn crossfade_samples_at_48k() {
        let g = GateConfig::default();
        assert_eq!(g.crossfade_samples(48_000), 240);
    }

    #[test]
    fn zero_crossfade_is_rejected() {
        let mut c = Config::default();
        c.gate.crossfade_ms = 0.0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn bad_ema_alpha_is_rejected() {
        let mut c = Config::default();
        c.verification.ema_alpha = 0.0;
        assert!(c.validate().is_err());
        c.verification.ema_alpha = 1.5;
        assert!(c.validate().is_err());
    }

    #[test]
    fn serde_roundtrip_with_defaults() {
        let c = Config::default();
        let text = toml::to_string_pretty(&c).unwrap();
        let c2: Config = toml::from_str(&text).unwrap();
        assert_eq!(c2.audio.frame_size_ms, 32);
        assert_eq!(c2.gate.hold_frames, 5);
        assert!((c2.gate.crossfade_ms - 5.0).abs() < f32::EPSILON);
        assert!((c2.verification.threshold - 0.70).abs() < f32::EPSILON);
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let text = "[audio]\ninput_device = \"mic1\"\noutput_device = \"auto\"\nframe_size_ms = 32\nsample_rate = 48000\n";
        let c: Config = toml::from_str(text).unwrap();
        c.validate().unwrap();
        assert_eq!(c.gate.hold_frames, 5);
        assert!((c.verification.ema_alpha - 0.3).abs() < f32::EPSILON);
    }
}
