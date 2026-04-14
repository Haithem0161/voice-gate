//! VoiceGate -- real-time speaker isolation for Discord.
//!
//! This is the library half of the crate; `main.rs` is the binary half and
//! wires up clap subcommands to the modules re-exported here.

pub mod audio;
pub mod config;
pub mod enrollment;
pub mod gate;
pub mod ml;
pub mod pipeline;

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
}

impl From<enrollment::profile::ProfileError> for VoiceGateError {
    fn from(e: enrollment::profile::ProfileError) -> Self {
        VoiceGateError::ProfileFormat(e.to_string())
    }
}
