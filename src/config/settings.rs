use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::VoiceGateError;

/// Root TOML config.
///
/// Only the `[audio]` section is populated in Phase 1. Later phases add
/// `[vad]`, `[verification]`, `[gate]`, `[enrollment]`, and `[gui]` sections
/// per PRD §5.9.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub audio: AudioConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AudioConfig {
    /// "default" or a specific cpal input device name.
    pub input_device: String,

    /// "auto" (resolve via VirtualMic::setup) or a specific cpal output device name.
    pub output_device: String,

    /// Frame size in milliseconds. Matches PRD §5.9 TOML key `frame_size_ms`.
    /// Hard-coded to 32 in v1 -- see `Config::validate` and Decision D-001 in
    /// `docs/voicegate/research.md`. Non-default values are rejected at load
    /// time because Silero VAD requires exactly 512 samples at 16 kHz, which
    /// only aligns at 32 ms frames.
    pub frame_size_ms: u32,

    pub sample_rate: u32,
}

impl AudioConfig {
    /// Derived sample count: `frame_size_ms * sample_rate / 1000`. For the v1
    /// values (32 ms x 48 000 Hz) this is exactly 1536.
    pub fn frame_size_samples(&self) -> usize {
        (self.frame_size_ms as usize) * (self.sample_rate as usize) / 1000
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            audio: AudioConfig {
                input_device: "default".into(),
                output_device: "auto".into(),
                frame_size_ms: 32,
                sample_rate: 48_000,
            },
        }
    }
}

impl Config {
    /// Path resolution: `dirs::config_dir()/voicegate/config.toml`.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("voicegate").join("config.toml"))
    }

    /// Load config from disk, falling back to `Config::default()` if the file
    /// does not exist. Validates before returning.
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

    /// Serialize and write the current config to `default_path()`. Creates the
    /// parent directory if missing.
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

    /// Validate user-provided config values. Returns a clean error instead of
    /// panicking or silently accepting out-of-range values.
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
}
