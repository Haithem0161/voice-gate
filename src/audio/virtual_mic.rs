//! Virtual microphone trait and platform implementations.
//!
//! The trait is the interchange point between the audio pipeline and the
//! platform-specific virtual-mic machinery. On Linux, the Phase 1 impl shells
//! out to `pw-cli` to create `voicegate_sink` and `voicegate_mic` nodes; on
//! Windows, it scans cpal output devices for VB-Audio Virtual Cable.
//!
//! See `.claude/rules/cross-platform.md` for the full table of platform
//! behavior and `docs/voicegate/phase-01.md` section 3.4 for the spec.

use crate::VoiceGateError;

pub trait VirtualMic: Send {
    /// Set up the virtual microphone. Returns the cpal output device name to
    /// write audio to. Calling `setup` twice in a row is an error.
    fn setup(&mut self) -> Result<String, VoiceGateError>;

    /// Tear down the virtual microphone. Idempotent: calling `teardown` when
    /// the mic was never set up or was already torn down must not error.
    fn teardown(&mut self) -> Result<(), VoiceGateError>;

    /// The human-readable name the end user should select as Discord's input
    /// device. On Linux this is `"voicegate_mic"`; on Windows it is
    /// `"CABLE Output (VB-Audio Virtual Cable)"`.
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
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::Duration;

    pub const SINK_NAME: &str = "voicegate_sink";
    pub const MIC_NAME: &str = "voicegate_mic";

    /// How long to wait after spawning `pw-loopback` before assuming its nodes
    /// have been registered with PipeWire. On a warm session this is well
    /// under 100 ms; we give it 500 ms of headroom.
    const LOOPBACK_BOOT_WAIT: Duration = Duration::from_millis(500);

    /// Linux virtual-mic implementation backed by `pw-loopback`.
    ///
    /// The original plan (PRD Appendix C / phase-01 section 3.4) called for
    /// `pw-cli create-node adapter ...` + `pw-link`. On PipeWire 1.0.5 that
    /// approach does not work: `pw-cli create-node` creates a node that is
    /// owned by the `pw-cli` process and dies the moment `pw-cli` exits,
    /// and the subcommand for destroying nodes is `destroy <id>`, not
    /// `destroy-node <name>`. `pw-loopback` is the correct tool: it is a
    /// long-running process that owns a capture sink + a playback source
    /// paired by an internal loopback. Killing the process tears down both
    /// nodes. See phase-01 section 6.3 for the full discovery notes.
    pub struct PwCliVirtualMic {
        child: Option<Child>,
    }

    impl PwCliVirtualMic {
        pub fn new() -> Self {
            Self { child: None }
        }
    }

    impl VirtualMic for PwCliVirtualMic {
        fn setup(&mut self) -> Result<String, VoiceGateError> {
            if self.child.is_some() {
                return Err(VoiceGateError::VirtualMic(
                    "PwCliVirtualMic::setup called twice".into(),
                ));
            }

            if which::which("pw-loopback").is_err() {
                return Err(VoiceGateError::VirtualMic(
                    "pw-loopback not found on PATH. VoiceGate requires PipeWire on Linux. \
                     Install with: sudo apt install pipewire pipewire-audio-client-libraries"
                        .into(),
                ));
            }

            let capture_props = format!(
                "node.name={SINK_NAME} \
                 node.description=\"VoiceGate Sink\" \
                 media.class=Audio/Sink"
            );
            let playback_props = format!(
                "node.name={MIC_NAME} \
                 node.description=\"VoiceGate Virtual Microphone\" \
                 media.class=Audio/Source/Virtual"
            );

            let child = Command::new("pw-loopback")
                .args(["--channels", "1"])
                .args(["--capture-props", capture_props.as_str()])
                .args(["--playback-props", playback_props.as_str()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| VoiceGateError::VirtualMic(format!("spawn pw-loopback: {e}")))?;

            // Give pw-loopback a moment to register its nodes with PipeWire
            // before the caller starts trying to open a cpal output stream
            // on them.
            thread::sleep(LOOPBACK_BOOT_WAIT);

            self.child = Some(child);
            Ok(SINK_NAME.to_string())
        }

        fn teardown(&mut self) -> Result<(), VoiceGateError> {
            let Some(mut child) = self.child.take() else {
                return Ok(());
            };

            // Ask pw-loopback to exit gracefully. std::process::Child::kill
            // sends SIGKILL on Unix, which is heavy-handed but reliable.
            // pw-loopback's atexit handlers clean up the virtual nodes
            // regardless of how it exits, so SIGKILL is fine here.
            let _ = child.kill();
            let _ = child.wait();
            Ok(())
        }

        fn discord_device_name(&self) -> &str {
            MIC_NAME
        }
    }

    impl Drop for PwCliVirtualMic {
        fn drop(&mut self) {
            // Defensive: if teardown was not called (e.g. panic in the main
            // thread), make sure we do not leak the pw-loopback process.
            let _ = self.teardown();
        }
    }
}

// --- Windows ---------------------------------------------------------------

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait};

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
            // verifies the "CABLE Input" side is present and returns its
            // name as the cpal output device VoiceGate should write to.
            let host = cpal::default_host();
            let outputs = host
                .output_devices()
                .map_err(|e| VoiceGateError::VirtualMic(format!("cpal output_devices: {e}")))?;

            for device in outputs {
                if let Ok(name) = device.name() {
                    if name == CABLE_INPUT {
                        self.set_up = true;
                        return Ok(name);
                    }
                }
            }

            Err(VoiceGateError::VirtualMic(
                "\"CABLE Input (VB-Audio Virtual Cable)\" not found. \
                 Install VB-Audio Virtual Cable from https://vb-audio.com/Cable/ and reboot."
                    .into(),
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
