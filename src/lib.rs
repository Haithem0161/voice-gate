//! VoiceGate -- real-time speaker isolation for Discord.
//!
//! This is the library half of the crate; `main.rs` is the binary half and
//! wires up clap subcommands to the modules re-exported here.

pub mod app_controller;
pub mod audio;
pub mod config;
pub mod enrollment;
pub mod gate;
pub mod gui;
pub mod ml;
pub mod pipeline;

use std::path::{Path, PathBuf};

/// Resolve a model file name (e.g. `silero_vad.onnx`) via the lookup order:
/// env var VOICEGATE_MODELS_DIR -> executable-relative -> repo-relative.
pub fn resolve_model_path(name: &str) -> anyhow::Result<PathBuf> {
    if let Ok(dir) = std::env::var("VOICEGATE_MODELS_DIR") {
        let candidate = PathBuf::from(dir).join(name);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("models").join(name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    let candidate = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("models")
        .join(name);
    if candidate.exists() {
        return Ok(candidate);
    }
    anyhow::bail!(
        "model {name} not found. Run `make models` to download it, or set VOICEGATE_MODELS_DIR."
    )
}

/// Resolve an asset file name (e.g. `enrollment_passages.txt`) via the
/// lookup order: env var VOICEGATE_ASSETS_DIR -> executable-relative -> repo-relative.
pub fn resolve_asset_path(name: &str) -> anyhow::Result<PathBuf> {
    if let Ok(dir) = std::env::var("VOICEGATE_ASSETS_DIR") {
        let candidate = PathBuf::from(dir).join(name);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("assets").join(name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    let candidate = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join(name);
    if candidate.exists() {
        return Ok(candidate);
    }
    anyhow::bail!(
        "asset {name} not found. Set VOICEGATE_ASSETS_DIR or place it next to the executable."
    )
}

/// Top-level error type for all VoiceGate domain boundaries.
///
/// Individual modules either return `Result<T, VoiceGateError>` directly or
/// use `anyhow::Result<T>` in application layers and convert at the boundary.
/// See `.claude/rules/rust-desktop.md` for the full policy.
#[derive(Debug, thiserror::Error)]
pub enum VoiceGateError {
    #[error("audio device error: {0}")]
    Audio(String),

    #[error("virtual microphone setup failed: {0}")]
    VirtualMic(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("ML inference error: {0}")]
    Ml(String),

    #[error("ML model file not found: {0}")]
    ModelNotFound(String),

    #[error(
        "ONNX Runtime is not available. Install libonnxruntime.so 1.22.x (Linux) or \
         onnxruntime.dll 1.22.x (Windows) -- see README.md for instructions."
    )]
    OrtUnavailable,

    #[error("enrollment error: {0}")]
    Enrollment(String),

    #[error("profile format error: {0}")]
    ProfileFormat(String),

    #[error("gate state error: {0}")]
    Gate(String),

    #[error("pipeline error: {0}")]
    Pipeline(String),

    #[error("GUI error: {0}")]
    Gui(String),
}

impl From<enrollment::profile::ProfileError> for VoiceGateError {
    fn from(e: enrollment::profile::ProfileError) -> Self {
        VoiceGateError::ProfileFormat(e.to_string())
    }
}
