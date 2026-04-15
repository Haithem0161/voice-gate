use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ringbuf::traits::{Consumer, Producer};

use crate::audio::capture::{list_input_devices, start_capture, CaptureStream};
use crate::audio::output::{start_output, OutputStream};
use crate::audio::resampler::{Resampler48to16, INPUT_CHUNK_SAMPLES};
use crate::audio::ring_buffer::{new_audio_ring, RING_CAPACITY_SAMPLES};
use crate::audio::virtual_mic::{create_virtual_mic, VirtualMic};
use crate::config::Config;
use crate::enrollment::enroll::EnrollmentSession;
use crate::enrollment::profile::Profile;
use crate::ml::embedding::EcapaTdnn;
use crate::ml::vad::SileroVad;
use crate::pipeline::processor::{PipelineProcessor, PipelineStatus};

struct RunningHandles {
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    _capture: CaptureStream,
    _output: OutputStream,
    vmic: Box<dyn VirtualMic>,
}

pub struct AppController {
    pub status: Arc<PipelineStatus>,
    pub config: Arc<RwLock<Config>>,
    running: Option<RunningHandles>,
}

pub struct StatusSnapshot {
    pub similarity: f32,
    pub gate_state: u8,
    pub vad_active: bool,
    pub bypass_mode: u8,
    pub is_running: bool,
}

impl AppController {
    pub fn new(config: Config) -> Self {
        Self {
            status: Arc::new(PipelineStatus::default()),
            config: Arc::new(RwLock::new(config)),
            running: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn status_snapshot(&self) -> StatusSnapshot {
        StatusSnapshot {
            similarity: f32::from_bits(self.status.similarity.load(Ordering::Relaxed)),
            gate_state: self.status.gate_state.load(Ordering::Relaxed),
            vad_active: self.status.vad_active.load(Ordering::Relaxed) != 0,
            bypass_mode: self.status.bypass_mode.load(Ordering::Relaxed),
            is_running: self.is_running(),
        }
    }

    pub fn start(&mut self, profile: Profile) -> anyhow::Result<()> {
        if self.running.is_some() {
            self.stop()?;
        }

        let config = self.config.read().unwrap().clone();

        let silero_path = crate::resolve_model_path(&config.vad.model_path)?;
        let wespeaker_path = crate::resolve_model_path(&config.verification.model_path)?;
        let vad = SileroVad::load(&silero_path)?;
        let ecapa = EcapaTdnn::load(&wespeaker_path)?;

        self.status = Arc::new(PipelineStatus::default());
        let mut pipeline =
            PipelineProcessor::new(&config, profile, vad, ecapa, self.status.clone())?;

        let (input_prod, mut input_cons) = new_audio_ring(RING_CAPACITY_SAMPLES);
        let (mut output_prod, output_cons) = new_audio_ring(RING_CAPACITY_SAMPLES);
        let (mut monitor_prod, monitor_cons) = new_audio_ring(RING_CAPACITY_SAMPLES);

        // Capture BEFORE virtual mic setup so PipeWire links to the real mic.
        let input_dev = config.audio.input_device.clone();
        let capture = start_capture(Some(&input_dev), input_prod)?;
        let capture_rate = capture.sample_rate;

        let mut vmic = create_virtual_mic();
        let output_device_name = vmic
            .setup()
            .map_err(|e| anyhow::anyhow!("virtual mic setup: {e}"))?;
        let output = start_output(&output_device_name, output_cons)?;

        // Monitor output: plays gated audio through default speakers when enabled.
        // Always create the stream; silence when monitor_enabled is false because
        // the worker won't push samples.
        let _monitor_output = start_output("default", monitor_cons).ok();

        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = shutdown.clone();
        let frame_samples = config.audio.frame_size_samples();
        let needs_resample = capture_rate != 48_000;
        let worker_status = self.status.clone();

        let worker = thread::spawn(move || {
            let mut pre_resampler = if needs_resample {
                Some(
                    crate::audio::resampler::CaptureResampler::new(capture_rate, 48_000)
                        .expect("pre-resampler"),
                )
            } else {
                None
            };
            let mut resample_buf: Vec<f32> = Vec::with_capacity(8192);
            let mut frame48k_accum: Vec<f32> = Vec::with_capacity(frame_samples * 2);
            let mut raw_scratch = vec![0.0f32; 4096];
            let mut frame = vec![0.0f32; frame_samples];

            while !worker_shutdown.load(Ordering::Relaxed) {
                if let Some(ref mut resampler) = pre_resampler {
                    let n = input_cons.pop_slice(&mut raw_scratch);
                    if n == 0 {
                        thread::sleep(Duration::from_micros(500));
                        continue;
                    }
                    resample_buf.clear();
                    if let Err(e) = resampler.process(&raw_scratch[..n], &mut resample_buf) {
                        tracing::error!("pre-resample error: {e}");
                        continue;
                    }
                    frame48k_accum.extend_from_slice(&resample_buf);
                    while frame48k_accum.len() >= frame_samples {
                        frame.copy_from_slice(&frame48k_accum[..frame_samples]);
                        frame48k_accum.drain(..frame_samples);
                        if let Err(e) = pipeline.process_frame(&mut frame) {
                            tracing::error!("pipeline error: {e}");
                            frame.fill(0.0);
                        }
                        output_prod.push_slice(&frame);
                        if worker_status.monitor_enabled.load(Ordering::Relaxed) {
                            monitor_prod.push_slice(&frame);
                        }
                    }
                } else {
                    let mut got = 0;
                    while got < frame_samples {
                        if worker_shutdown.load(Ordering::Relaxed) {
                            return;
                        }
                        let n = input_cons.pop_slice(&mut frame[got..]);
                        got += n;
                        if n == 0 {
                            thread::sleep(Duration::from_micros(500));
                        }
                    }
                    if let Err(e) = pipeline.process_frame(&mut frame) {
                        tracing::error!("pipeline error: {e}");
                        frame.fill(0.0);
                    }
                    output_prod.push_slice(&frame);
                }
            }
        });

        self.running = Some(RunningHandles {
            shutdown,
            worker: Some(worker),
            _capture: capture,
            _output: output,
            vmic,
        });

        Ok(())
    }

    pub fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(mut handles) = self.running.take() {
            handles.shutdown.store(true, Ordering::SeqCst);
            if let Some(worker) = handles.worker.take() {
                let _ = worker.join();
            }
            // Streams dropped here (capture + output)
            if let Err(e) = handles.vmic.teardown() {
                tracing::warn!(%e, "virtual mic teardown failed");
            }
        }
        Ok(())
    }

    pub fn load_default_profile(&self) -> anyhow::Result<Profile> {
        let config = self.config.read().unwrap();
        let path = if config.enrollment.profile_path != "auto" {
            PathBuf::from(&config.enrollment.profile_path)
        } else {
            Profile::default_path()?
        };
        Profile::load(&path).map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn enroll_from_mic(
        &self,
        seconds: u32,
        progress: Arc<AtomicU32>,
        cancel: Arc<AtomicBool>,
    ) -> anyhow::Result<Profile> {
        let config = self.config.read().unwrap().clone();

        let silero_path = crate::resolve_model_path(&config.vad.model_path)?;
        let wespeaker_path = crate::resolve_model_path(&config.verification.model_path)?;
        let vad = SileroVad::load(&silero_path)?;
        let ecapa = EcapaTdnn::load(&wespeaker_path)?;
        let mut session = EnrollmentSession::new(vad, ecapa);

        let (input_prod, mut input_cons) = new_audio_ring(RING_CAPACITY_SAMPLES);
        let capture = start_capture(Some(&config.audio.input_device), input_prod)?;

        let mut resampler = Resampler48to16::new()?;
        let mut scratch = vec![0.0f32; INPUT_CHUNK_SAMPLES];

        let start = std::time::Instant::now();
        let target_duration = Duration::from_secs(u64::from(seconds));

        while start.elapsed() < target_duration && !cancel.load(Ordering::Relaxed) {
            let n = input_cons.pop_slice(&mut scratch);
            if n == 0 {
                thread::sleep(Duration::from_micros(500));
                continue;
            }
            if n == INPUT_CHUNK_SAMPLES {
                let out = resampler.process_block(&scratch)?;
                session.push_audio(out);
            }
            progress.store(start.elapsed().as_secs() as u32, Ordering::Relaxed);
        }

        drop(capture);

        let centroid = session.finalize()?;
        let profile = Profile::new(centroid);

        let out_path = if config.enrollment.profile_path != "auto" {
            PathBuf::from(&config.enrollment.profile_path)
        } else {
            Profile::default_path()?
        };
        profile.save(&out_path)?;

        Ok(profile)
    }

    pub fn set_bypass(&self, mode: u8) {
        self.status.bypass_mode.store(mode, Ordering::Relaxed);
    }

    pub fn set_monitor(&self, enabled: bool) {
        self.status
            .monitor_enabled
            .store(enabled, Ordering::Relaxed);
    }

    /// Snapshot the input waveform buffer for GUI display. Returns a Vec
    /// of downsampled samples (every 3rd sample from 48kHz = ~16kHz display).
    pub fn waveform_in_snapshot(&self) -> Vec<f32> {
        self.status
            .waveform_in
            .try_lock()
            .map(|wf| wf.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Snapshot the output (gated) waveform buffer for GUI display.
    pub fn waveform_out_snapshot(&self) -> Vec<f32> {
        self.status
            .waveform_out
            .try_lock()
            .map(|wf| wf.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn input_devices(&self) -> Vec<(String, bool)> {
        list_input_devices().unwrap_or_default()
    }
}
