use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use ringbuf::traits::{Consumer, Producer};
use tracing_subscriber::EnvFilter;

use voicegate::audio::capture::{list_input_devices, start_capture};
use voicegate::audio::output::{list_output_devices, start_output};
use voicegate::audio::resampler::{CaptureResampler, Resampler48to16, INPUT_CHUNK_SAMPLES};
use voicegate::audio::ring_buffer::{new_audio_ring, RING_CAPACITY_SAMPLES};
use voicegate::audio::virtual_mic::create_virtual_mic;
use voicegate::config::Config;
use voicegate::enrollment::enroll::EnrollmentSession;
use voicegate::enrollment::profile::Profile;
use voicegate::ml::embedding::EcapaTdnn;
use voicegate::ml::vad::SileroVad;
use voicegate::pipeline::processor::{PipelineProcessor, PipelineStatus};

#[derive(Parser)]
#[command(
    name = "voicegate",
    version,
    about = "Real-time speaker isolation for Discord"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List cpal input and output devices.
    Devices,

    /// Run the audio pipeline.
    Run {
        /// Passthrough mode: mic -> virtual mic with no ML processing.
        #[arg(long, conflicts_with = "headless")]
        passthrough: bool,

        /// Headless gated pipeline using a saved profile.
        #[arg(long)]
        headless: bool,

        /// Path to profile.bin. Defaults to Profile::default_path().
        #[arg(long, value_name = "PATH")]
        profile: Option<PathBuf>,

        /// Override input device.
        #[arg(long, value_name = "NAME")]
        input_device: Option<String>,
    },

    /// Enroll a voice. Exactly one of --wav, --mic, or --list-passages must be provided.
    Enroll {
        /// Read audio from a WAV file. Supports 16 kHz (passthrough) and
        /// 48 kHz (auto-downsampled) mono/stereo WAVs. Other rates must
        /// be converted to 16 kHz first via `ffmpeg -ac 1 -ar 16000`.
        #[arg(long, value_name = "PATH", conflicts_with_all = ["mic", "list_passages"])]
        wav: Option<PathBuf>,

        /// Record `N` seconds of live audio from the default (or --device) mic.
        /// Note: on Phase 3's dev hardware, cpal's ALSA backend may not be
        /// able to open the default mic at 48 kHz f32 due to G-015 (documented
        /// in phase-01.md). If --mic fails with an I/O error, use --wav with
        /// a pre-recorded WAV instead.
        #[arg(long, value_name = "SECONDS", conflicts_with_all = ["wav", "list_passages"])]
        mic: Option<u32>,

        /// Print the enrollment passage from assets/enrollment_passages.txt and exit.
        #[arg(long)]
        list_passages: bool,

        /// Override the output profile path. Defaults to the platform data dir.
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,

        /// Override the input device (only applies to --mic mode).
        #[arg(long, value_name = "NAME")]
        device: Option<String>,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("voicegate=info".parse()?))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Devices => cmd_devices(),
        Commands::Run {
            passthrough,
            headless,
            profile,
            input_device,
        } => {
            if passthrough {
                cmd_run_passthrough()
            } else if headless {
                cmd_run_headless(profile, input_device)
            } else {
                voicegate::gui::app::run().map_err(|e| anyhow::anyhow!("GUI error: {e}"))
            }
        }
        Commands::Enroll {
            wav,
            mic,
            list_passages,
            output,
            device,
        } => cmd_enroll(wav, mic, list_passages, output, device),
    }
}

/// `voicegate devices` -- enumerate cpal input and output devices.
fn cmd_devices() -> Result<()> {
    println!("Input devices:");
    for (name, is_default) in list_input_devices()? {
        let suffix = if is_default { " (default)" } else { "" };
        println!("  IN  {name}{suffix}");
    }

    println!("\nOutput devices:");
    for (name, is_default) in list_output_devices()? {
        let suffix = if is_default { " (default)" } else { "" };
        println!("  OUT {name}{suffix}");
    }
    Ok(())
}

/// `voicegate run --passthrough` -- mic to virtual-mic loopback with no ML.
fn cmd_run_passthrough() -> Result<()> {
    let config = Config::load()?;
    tracing::info!(
        input_device = %config.audio.input_device,
        frame_samples = config.audio.frame_size_samples(),
        "starting passthrough"
    );

    // 1. Set up the virtual mic and learn which cpal output device to write to.
    let mut vmic = create_virtual_mic();
    let output_device_name = vmic
        .setup()
        .map_err(|e| anyhow::anyhow!("virtual mic setup: {e}"))?;
    tracing::info!(
        output_device = %output_device_name,
        discord_device = %vmic.discord_device_name(),
        "virtual mic ready"
    );

    // 2. Allocate input and output ring buffers. 3 seconds of headroom each.
    let (input_prod, mut input_cons) = new_audio_ring(RING_CAPACITY_SAMPLES);
    let (mut output_prod, output_cons) = new_audio_ring(RING_CAPACITY_SAMPLES);

    // 3. Start the capture stream (pushes into input ring).
    let input_requested = config.audio.input_device.as_str();
    let capture = start_capture(Some(input_requested), input_prod)?;
    let capture_rate = capture.sample_rate;
    tracing::info!(device = %capture.device_name, rate = capture_rate, "capture started");

    // 4. Start the output stream.
    let output = start_output(&output_device_name, output_cons)?;
    tracing::info!(device = %output.device_name, "output started");

    // 5. Install Ctrl-C handler.
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_signal = shutdown.clone();
    ctrlc::set_handler(move || {
        tracing::info!("Ctrl-C received, shutting down");
        shutdown_signal.store(true, Ordering::SeqCst);
    })
    .map_err(|e| anyhow::anyhow!("install ctrl-c handler: {e}"))?;

    // 6. Spawn the passthrough worker. If the capture rate is not 48kHz,
    //    resample before pushing to the output ring.
    let worker_shutdown = shutdown.clone();
    let needs_resample = capture_rate != 48_000;
    let worker = thread::spawn(move || {
        let mut scratch = vec![0.0f32; 4096];
        let mut pre_resampler = if needs_resample {
            Some(CaptureResampler::new(capture_rate, 48_000).expect("pre-resampler"))
        } else {
            None
        };
        let mut resample_buf = Vec::with_capacity(8192);

        while !worker_shutdown.load(Ordering::Relaxed) {
            let n = input_cons.pop_slice(&mut scratch);
            if n == 0 {
                thread::sleep(Duration::from_micros(500));
                continue;
            }
            if let Some(ref mut resampler) = pre_resampler {
                resample_buf.clear();
                if let Err(e) = resampler.process(&scratch[..n], &mut resample_buf) {
                    tracing::error!("pre-resample error: {e}");
                    continue;
                }
                output_prod.push_slice(&resample_buf);
            } else {
                output_prod.push_slice(&scratch[..n]);
            }
        }
        tracing::info!("passthrough worker exiting");
    });

    println!(
        "VoiceGate passthrough is live. Point Discord at {:?} and speak into the mic.",
        vmic.discord_device_name()
    );
    println!("Press Ctrl-C to stop.");

    // 7. Main thread parks until shutdown is signaled. The worker thread
    //    is doing the real work; we just wait.
    while !shutdown.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(100));
    }

    // 8. Join the worker so we know it stopped touching the ring buffers
    //    before we drop the streams.
    if let Err(e) = worker.join() {
        tracing::warn!(?e, "worker thread panicked");
    }

    // 9. Drop the streams (stops callbacks), THEN tear down the virtual mic.
    //    Order matters: if we tore down first, the output callback would
    //    still be writing to a device that no longer exists.
    drop(capture);
    drop(output);

    if let Err(e) = vmic.teardown() {
        tracing::warn!(%e, "virtual mic teardown failed");
    }

    tracing::info!("shutdown complete");
    Ok(())
}

/// `voicegate run --headless` -- gated pipeline with speaker verification.
fn cmd_run_headless(
    profile_path: Option<PathBuf>,
    input_device_override: Option<String>,
) -> Result<()> {
    let config = Config::load()?;

    let profile_file = match profile_path {
        Some(p) => p,
        None => {
            if config.enrollment.profile_path != "auto" {
                PathBuf::from(&config.enrollment.profile_path)
            } else {
                Profile::default_path()?
            }
        }
    };
    tracing::info!(profile = %profile_file.display(), "loading profile");
    let profile = Profile::load(&profile_file)?;

    let silero_path = resolve_model_path(&config.vad.model_path)?;
    let wespeaker_path = resolve_model_path(&config.verification.model_path)?;
    let vad = SileroVad::load(&silero_path)?;
    let ecapa = EcapaTdnn::load(&wespeaker_path)?;

    let status = Arc::new(PipelineStatus::default());
    let mut pipeline = PipelineProcessor::new(&config, profile, vad, ecapa, status.clone())?;

    let mut vmic = create_virtual_mic();
    let output_device_name = vmic
        .setup()
        .map_err(|e| anyhow::anyhow!("virtual mic setup: {e}"))?;
    tracing::info!(
        output_device = %output_device_name,
        discord_device = %vmic.discord_device_name(),
        "virtual mic ready"
    );

    let (input_prod, mut input_cons) = new_audio_ring(RING_CAPACITY_SAMPLES);
    let (mut output_prod, output_cons) = new_audio_ring(RING_CAPACITY_SAMPLES);

    let input_dev = input_device_override
        .as_deref()
        .unwrap_or(&config.audio.input_device);
    let capture = start_capture(Some(input_dev), input_prod)?;
    let capture_rate = capture.sample_rate;
    tracing::info!(device = %capture.device_name, rate = capture_rate, "capture started");

    let output = start_output(&output_device_name, output_cons)?;
    tracing::info!(device = %output.device_name, "output started");

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_signal = shutdown.clone();
    ctrlc::set_handler(move || {
        tracing::info!("Ctrl-C received, shutting down");
        shutdown_signal.store(true, Ordering::SeqCst);
    })
    .map_err(|e| anyhow::anyhow!("install ctrl-c handler: {e}"))?;

    let worker_shutdown = shutdown.clone();
    let frame_samples = config.audio.frame_size_samples();
    let needs_resample = capture_rate != 48_000;
    let worker = thread::spawn(move || {
        let mut pre_resampler = if needs_resample {
            Some(CaptureResampler::new(capture_rate, 48_000).expect("pre-resampler"))
        } else {
            None
        };
        let mut resample_buf: Vec<f32> = Vec::with_capacity(8192);
        let mut frame48k_accum: Vec<f32> = Vec::with_capacity(frame_samples * 2);
        let mut raw_scratch = vec![0.0f32; 4096];
        let mut frame = vec![0.0f32; frame_samples];

        while !worker_shutdown.load(Ordering::Relaxed) {
            if let Some(ref mut resampler) = pre_resampler {
                // Read raw samples from input ring, resample to 48kHz,
                // accumulate, then process in frame_samples chunks.
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
                }
            } else {
                // Direct 48kHz path
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
        tracing::info!("pipeline worker exiting");
    });

    println!(
        "VoiceGate headless pipeline is live. Point Discord at {:?} and speak into the mic.",
        vmic.discord_device_name()
    );
    println!("Press Ctrl-C to stop.");

    while !shutdown.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(100));
    }

    if let Err(e) = worker.join() {
        tracing::warn!(?e, "worker thread panicked");
    }

    drop(capture);
    drop(output);

    if let Err(e) = vmic.teardown() {
        tracing::warn!(%e, "virtual mic teardown failed");
    }

    tracing::info!("shutdown complete");
    Ok(())
}

// --- Enrollment ------------------------------------------------------------

fn cmd_enroll(
    wav: Option<PathBuf>,
    mic: Option<u32>,
    list_passages: bool,
    output: Option<PathBuf>,
    device: Option<String>,
) -> Result<()> {
    // Exactly one of the three modes must be specified. clap's
    // conflicts_with_all already prevents multiple, but we still need to
    // reject the "none" case.
    if list_passages {
        return cmd_enroll_list_passages();
    }

    match (wav, mic) {
        (Some(path), None) => cmd_enroll_wav(path, output),
        (None, Some(seconds)) => cmd_enroll_mic(seconds, output, device),
        (None, None) => anyhow::bail!(
            "must specify exactly one of --wav <path>, --mic <seconds>, or --list-passages"
        ),
        (Some(_), Some(_)) => unreachable!("clap conflicts_with_all prevents this"),
    }
}

fn cmd_enroll_list_passages() -> Result<()> {
    let path = resolve_asset_path("enrollment_passages.txt")?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    print!("{text}");
    Ok(())
}

fn cmd_enroll_wav(wav_path: PathBuf, output: Option<PathBuf>) -> Result<()> {
    println!("Reading WAV file: {}", wav_path.display());
    let audio_16k = read_wav_as_16k_mono(&wav_path)?;
    let duration_s = audio_16k.len() as f32 / 16_000.0;
    println!("Loaded {duration_s:.1} s of 16 kHz mono audio");

    // Load models. Resolution order: env var override -> executable-relative
    // -> repo-relative (dev mode).
    let silero_path = resolve_model_path("silero_vad.onnx")?;
    let wespeaker_path = resolve_model_path("wespeaker_resnet34_lm.onnx")?;
    let vad = SileroVad::load(&silero_path)?;
    let ecapa = EcapaTdnn::load(&wespeaker_path)?;
    let mut session = EnrollmentSession::new(vad, ecapa);

    session.push_audio(&audio_16k);
    if !session.is_ready() {
        tracing::warn!(
            duration_s,
            "enrollment audio is shorter than the recommended 20 s minimum; \
             finalize() may fail if fewer than 5 speech segments are found"
        );
    }

    let centroid = session.finalize()?;
    let profile = Profile::new(centroid);

    let out_path = resolve_profile_output(output)?;
    profile.save(&out_path)?;
    println!("Profile saved: {}", out_path.display());
    Ok(())
}

fn cmd_enroll_mic(seconds: u32, output: Option<PathBuf>, device: Option<String>) -> Result<()> {
    println!("Recording for {seconds} seconds. Read the passage aloud:");
    println!();
    let passage = std::fs::read_to_string(resolve_asset_path("enrollment_passages.txt")?)
        .unwrap_or_else(|_| {
            String::from("(passage file missing -- speak naturally for the duration)\n")
        });
    print!("{passage}");
    println!();

    // Load models up front so the user gets feedback if the ORT runtime
    // or model files are missing, before we start hitting the mic.
    let silero_path = resolve_model_path("silero_vad.onnx")?;
    let wespeaker_path = resolve_model_path("wespeaker_resnet34_lm.onnx")?;
    let vad = SileroVad::load(&silero_path)?;
    let ecapa = EcapaTdnn::load(&wespeaker_path)?;
    let mut session = EnrollmentSession::new(vad, ecapa);

    // Set up the capture ring buffer and stream. Mic enrollment does NOT
    // touch the virtual microphone -- we only need the input half of the
    // audio pipeline.
    let (input_prod, mut input_cons) = new_audio_ring(RING_CAPACITY_SAMPLES);
    let device_ref = device.as_deref();
    let capture = start_capture(device_ref, input_prod)?;
    tracing::info!(device = %capture.device_name, "enroll mic capture started");

    // Resampler: cpal captures at 48 kHz, enrollment needs 16 kHz.
    let mut resampler = Resampler48to16::new()?;
    let mut scratch = vec![0.0f32; INPUT_CHUNK_SAMPLES];

    let start = std::time::Instant::now();
    let target_duration = Duration::from_secs(u64::from(seconds));
    let mut last_progress_tick = std::time::Instant::now();
    print!("Recording: ");
    std::io::Write::flush(&mut std::io::stdout()).ok();

    while start.elapsed() < target_duration {
        let n = input_cons.pop_slice(&mut scratch);
        if n == 0 {
            thread::sleep(Duration::from_micros(500));
            continue;
        }
        if n == INPUT_CHUNK_SAMPLES {
            // Full frame -- resample to 16 kHz and push to enrollment.
            let out = resampler.process_block(&scratch)?;
            session.push_audio(out);
        }
        // Progress dot every 1 s.
        if last_progress_tick.elapsed() >= Duration::from_secs(1) {
            print!(".");
            std::io::Write::flush(&mut std::io::stdout()).ok();
            last_progress_tick = std::time::Instant::now();
        }
    }
    println!();

    // Stop the capture stream explicitly so no more callbacks fire while
    // we finalize.
    drop(capture);

    let centroid = session.finalize()?;
    let profile = Profile::new(centroid);

    let out_path = resolve_profile_output(output)?;
    profile.save(&out_path)?;
    println!("Profile saved: {}", out_path.display());
    Ok(())
}

// --- Helpers ---------------------------------------------------------------

/// Read a WAV file of any int or float format, any mono/stereo/multi-channel
/// layout, at 16 kHz or 48 kHz, and return a 16 kHz mono f32 Vec in
/// `[-1, 1]` range.
///
/// Other sample rates return a clear error with an ffmpeg hint. Phase 3
/// only supports the two rates we actually use (capture is 48 kHz, fixtures
/// are 16 kHz). A more general resampler path is Phase 4+ territory.
fn read_wav_as_16k_mono(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| anyhow::anyhow!("open {}: {e}", path.display()))?;
    let spec = reader.spec();
    tracing::info!(
        path = %path.display(),
        channels = spec.channels,
        sample_rate = spec.sample_rate,
        bits = spec.bits_per_sample,
        sample_format = ?spec.sample_format,
        "reading WAV"
    );

    // Read samples as interleaved f32.
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<f32>, _>>()
            .map_err(|e| anyhow::anyhow!("read f32 samples: {e}"))?,
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample;
            let scale = (1i64 << (bits - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<Vec<f32>, _>>()
                .map_err(|e| anyhow::anyhow!("read int samples: {e}"))?
        }
    };

    // Deinterleave: mono stream is the average across channels.
    let channels = spec.channels as usize;
    let mono: Vec<f32> = if channels == 1 {
        interleaved
    } else {
        let n_frames = interleaved.len() / channels;
        let mut out = Vec::with_capacity(n_frames);
        for frame_idx in 0..n_frames {
            let mut sum = 0.0f32;
            for ch in 0..channels {
                sum += interleaved[frame_idx * channels + ch];
            }
            out.push(sum / channels as f32);
        }
        out
    };

    // Resample to 16 kHz if needed.
    match spec.sample_rate {
        16_000 => Ok(mono),
        48_000 => {
            // Use the same Resampler48to16 that the live pipeline uses.
            let mut resampler = Resampler48to16::new()?;
            let mut output = Vec::with_capacity(mono.len() / 3 + 1024);
            let mut offset = 0;
            while offset + INPUT_CHUNK_SAMPLES <= mono.len() {
                let block = &mono[offset..offset + INPUT_CHUNK_SAMPLES];
                let out = resampler.process_block(block)?;
                output.extend_from_slice(out);
                offset += INPUT_CHUNK_SAMPLES;
            }
            // Tail samples (<1536) are dropped. For a 29 s clip that's at
            // most 1535 samples = 96 ms, which is shorter than the VAD
            // chunk size and would be discarded anyway.
            Ok(output)
        }
        other => anyhow::bail!(
            "unsupported WAV sample rate {other} Hz. Phase 3 only handles 16 kHz and 48 kHz. \
             Convert first with: ffmpeg -i {} -ac 1 -ar 16000 <output.wav>",
            path.display()
        ),
    }
}

fn resolve_asset_path(name: &str) -> Result<PathBuf> {
    voicegate::resolve_asset_path(name)
}

fn resolve_model_path(name: &str) -> Result<PathBuf> {
    voicegate::resolve_model_path(name)
}

/// Resolve the output profile path. Uses the explicit CLI arg first; falls
/// back to `Profile::default_path()`.
fn resolve_profile_output(cli_arg: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = cli_arg {
        return Ok(p);
    }
    Profile::default_path()
}
