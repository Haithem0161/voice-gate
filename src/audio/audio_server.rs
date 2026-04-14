//! Linux audio server detection.
//!
//! Probes the runtime environment to determine whether PipeWire or
//! PulseAudio is running. Used by `create_virtual_mic()` to pick the
//! right virtual-mic implementation, and by `voicegate doctor` to
//! report the environment.

use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioServer {
    PipeWire,
    PulseAudio,
    Unknown,
}

impl std::fmt::Display for AudioServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioServer::PipeWire => write!(f, "PipeWire"),
            AudioServer::PulseAudio => write!(f, "PulseAudio"),
            AudioServer::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Detect the running audio server on Linux. Checks PipeWire first
/// (preferred), then PulseAudio, then gives up.
pub fn detect_audio_server() -> AudioServer {
    if is_pipewire_running() {
        AudioServer::PipeWire
    } else if is_pulseaudio_running() {
        AudioServer::PulseAudio
    } else {
        AudioServer::Unknown
    }
}

fn is_pipewire_running() -> bool {
    which::which("pw-cli").is_ok()
        && Command::new("pw-cli")
            .arg("info")
            .arg("0")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
}

fn is_pulseaudio_running() -> bool {
    which::which("pactl").is_ok()
        && Command::new("pactl")
            .arg("info")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
}
