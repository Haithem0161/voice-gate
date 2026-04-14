//! VoiceGate -- real-time speaker isolation for Discord.
//!
//! This is the library half of the crate; `main.rs` is the binary half and
//! wires up clap subcommands to the modules re-exported here.

pub mod audio;
pub mod config;
pub mod ml;

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

    #[error("enrollment error: {0}")]
    Enrollment(String),

    #[error("gate state error: {0}")]
    Gate(String),
}
