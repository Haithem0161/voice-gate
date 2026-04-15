//! Virtual microphone trait and platform implementations.
//!
//! On Linux, the factory detects the audio server (PipeWire or PulseAudio)
//! and picks the matching implementation. On Windows, it scans for VB-Cable.

use crate::VoiceGateError;

pub trait VirtualMic: Send {
    fn setup(&mut self) -> Result<String, VoiceGateError>;
    fn teardown(&mut self) -> Result<(), VoiceGateError>;
    fn discord_device_name(&self) -> &str;
}

pub fn create_virtual_mic() -> Box<dyn VirtualMic> {
    #[cfg(target_os = "linux")]
    {
        use crate::audio::audio_server::{detect_audio_server, AudioServer};
        match detect_audio_server() {
            AudioServer::PipeWire => Box::new(linux::PwCliVirtualMic::new()),
            AudioServer::PulseAudio => Box::new(linux::PulseVirtualMic::new()),
            AudioServer::Unknown => {
                tracing::warn!("no supported audio server detected, trying PipeWire anyway");
                Box::new(linux::PwCliVirtualMic::new())
            }
        }
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

    const LOOPBACK_BOOT_WAIT: Duration = Duration::from_millis(500);

    // --- PipeWire (pw-loopback) ---

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

            // Kill any stale pw-loopback processes from crashed runs.
            // Duplicate nodes confuse PipeWire's routing.
            let _ = Command::new("pkill")
                .args(["-f", "pw-loopback.*voicegate"])
                .output();
            thread::sleep(Duration::from_millis(200));

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

            thread::sleep(LOOPBACK_BOOT_WAIT);

            self.child = Some(child);
            Ok(SINK_NAME.to_string())
        }

        fn teardown(&mut self) -> Result<(), VoiceGateError> {
            let Some(mut child) = self.child.take() else {
                return Ok(());
            };
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
            let _ = self.teardown();
        }
    }

    // --- PulseAudio ---

    pub struct PulseVirtualMic {
        loaded_modules: Vec<u32>,
    }

    impl PulseVirtualMic {
        pub fn new() -> Self {
            Self {
                loaded_modules: Vec::new(),
            }
        }

        fn load_module(&mut self, args: &str) -> Result<u32, VoiceGateError> {
            let output = Command::new("pactl")
                .args(["load-module", args.split_whitespace().next().unwrap_or("")])
                .args(args.split_whitespace().skip(1))
                .output()
                .map_err(|e| VoiceGateError::VirtualMic(format!("pactl load-module: {e}")))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(VoiceGateError::VirtualMic(format!(
                    "pactl load-module failed: {stderr}"
                )));
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let index: u32 = stdout.trim().parse().map_err(|_| {
                VoiceGateError::VirtualMic(format!(
                    "pactl load-module returned non-numeric index: {stdout}"
                ))
            })?;

            self.loaded_modules.push(index);
            Ok(index)
        }
    }

    impl VirtualMic for PulseVirtualMic {
        fn setup(&mut self) -> Result<String, VoiceGateError> {
            if which::which("pactl").is_err() {
                return Err(VoiceGateError::VirtualMic(
                    "pactl not found. Install PulseAudio: sudo apt install pulseaudio-utils".into(),
                ));
            }

            // Create the null sink
            self.load_module(
                "module-null-sink sink_name=voicegate_sink sink_properties=device.description=VoiceGate_Sink"
            )?;

            // Create the remap source (virtual mic)
            self.load_module(
                "module-remap-source master=voicegate_sink.monitor source_name=voicegate_mic source_properties=device.description=VoiceGate_Virtual_Microphone"
            )?;

            thread::sleep(Duration::from_millis(300));

            Ok(SINK_NAME.to_string())
        }

        fn teardown(&mut self) -> Result<(), VoiceGateError> {
            for &index in self.loaded_modules.iter().rev() {
                let _ = Command::new("pactl")
                    .args(["unload-module", &index.to_string()])
                    .output();
            }
            self.loaded_modules.clear();
            Ok(())
        }

        fn discord_device_name(&self) -> &str {
            "VoiceGate_Virtual_Microphone"
        }
    }

    impl Drop for PulseVirtualMic {
        fn drop(&mut self) {
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
