//! Virtual microphone trait and platform implementations.
//!
//! The trait is the interchange point between the audio pipeline and the
//! platform-specific virtual-mic machinery. On Linux, the Phase 1 impl shells
//! out to `pw-cli` to create `voicegate_sink` and `voicegate_mic` nodes; on
//! Windows, it scans cpal output devices for VB-Audio Virtual Cable.
//!
//! See `.claude/rules/cross-platform.md` for the full table of platform
//! behavior and `docs/voicegate/phase-01.md` §3.4 for the spec.

use crate::VoiceGateError;

pub trait VirtualMic: Send {
    /// Set up the virtual microphone. Returns the cpal output device name to
    /// write audio to. Idempotent-safe in the happy path: calling `setup`
    /// twice in a row is an error.
    fn setup(&mut self) -> Result<String, VoiceGateError>;

    /// Tear down the virtual microphone. Idempotent: calling `teardown` when
    /// the mic was never set up or was already torn down must not error.
    fn teardown(&mut self) -> Result<(), VoiceGateError>;

    /// The human-readable name the end user should select as Discord's input
    /// device. On Linux this is `"voicegate_mic"`; on Windows it is
    /// `"CABLE Output (VB-Audio Virtual Cable)"` (note: that is the OUTPUT
    /// side of VB-Cable from VB-Cable's perspective, which is the INPUT from
    /// Discord's perspective -- see `cross-platform.md`).
    fn discord_device_name(&self) -> &str;
}

/// Create the appropriate virtual-mic impl for the current platform.
pub fn create_virtual_mic() -> Box<dyn VirtualMic> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::PwCliVirtualMic::new())
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::VbCableVirtualMic::new())
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        Box::new(unsupported::UnsupportedVirtualMic)
    }
}

// --- Linux -----------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    // Used by the real pw-cli shell-out in step 7 of the Phase 1 morph.
    #[allow(dead_code)]
    pub const SINK_NAME: &str = "voicegate_sink";
    pub const MIC_NAME: &str = "voicegate_mic";

    pub struct PwCliVirtualMic {
        /// Whether `setup` completed successfully. Set to false after
        /// successful teardown so the handle can be reused.
        set_up: bool,
    }

    impl PwCliVirtualMic {
        pub fn new() -> Self {
            Self { set_up: false }
        }
    }

    impl VirtualMic for PwCliVirtualMic {
        fn setup(&mut self) -> Result<String, VoiceGateError> {
            if self.set_up {
                return Err(VoiceGateError::VirtualMic(
                    "PwCliVirtualMic::setup called twice".into(),
                ));
            }
            // Real pw-cli shell-out lands in step 7 of the Phase 1 morph.
            Err(VoiceGateError::VirtualMic(
                "pw-cli setup not yet implemented (Phase 1, step 7)".into(),
            ))
        }

        fn teardown(&mut self) -> Result<(), VoiceGateError> {
            // Idempotent: if we never set up, there is nothing to tear down.
            self.set_up = false;
            Ok(())
        }

        fn discord_device_name(&self) -> &str {
            MIC_NAME
        }
    }
}

// --- Windows ---------------------------------------------------------------

#[cfg(target_os = "windows")]
mod windows {
    use super::*;

    // Used by the real cpal device scan in step 7 of the Phase 1 morph.
    #[allow(dead_code)]
    pub const CABLE_INPUT: &str = "CABLE Input (VB-Audio Virtual Cable)";
    pub const CABLE_OUTPUT: &str = "CABLE Output (VB-Audio Virtual Cable)";

    pub struct VbCableVirtualMic {
        set_up: bool,
    }

    impl VbCableVirtualMic {
        pub fn new() -> Self {
            Self { set_up: false }
        }
    }

    impl VirtualMic for VbCableVirtualMic {
        fn setup(&mut self) -> Result<String, VoiceGateError> {
            // VB-Cable is user-installed and persistent across runs. setup()
            // only verifies the device is present; it does not create it.
            Err(VoiceGateError::VirtualMic(
                "VB-Cable detection not yet implemented (Phase 1, step 7)".into(),
            ))
        }

        fn teardown(&mut self) -> Result<(), VoiceGateError> {
            self.set_up = false;
            Ok(())
        }

        fn discord_device_name(&self) -> &str {
            CABLE_OUTPUT
        }
    }
}

// --- Fallback (macOS, BSD, etc.) -------------------------------------------

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported {
    use super::*;

    pub struct UnsupportedVirtualMic;

    impl VirtualMic for UnsupportedVirtualMic {
        fn setup(&mut self) -> Result<String, VoiceGateError> {
            Err(VoiceGateError::VirtualMic(
                "VoiceGate v1 supports only Linux and Windows".into(),
            ))
        }

        fn teardown(&mut self) -> Result<(), VoiceGateError> {
            Ok(())
        }

        fn discord_device_name(&self) -> &str {
            "unsupported"
        }
    }
}
