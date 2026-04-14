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
use voicegate::audio::ring_buffer::{new_audio_ring, RING_CAPACITY_SAMPLES};
use voicegate::audio::virtual_mic::create_virtual_mic;
use voicegate::config::Config;

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

    /// Run the audio pipeline. In Phase 1, only --passthrough is wired.
    Run {
        /// Passthrough mode: mic -> virtual mic with no ML processing.
        #[arg(long)]
        passthrough: bool,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("voicegate=info".parse()?))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Devices => cmd_devices(),
        Commands::Run { passthrough } => cmd_run(passthrough),
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
fn cmd_run(passthrough: bool) -> Result<()> {
    if !passthrough {
        anyhow::bail!("Phase 1 only implements `run --passthrough`. Gated mode lands in Phase 4.");
    }

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
    tracing::info!(device = %capture.device_name, "capture started");

    // 4. Start the output stream BEFORE the worker so the consumer is alive
    //    when the worker starts pushing. Order matters: we want the output
    //    callback primed to read from the output ring as soon as samples
    //    arrive from the worker.
    let output = start_output(&output_device_name, output_cons)?;
    tracing::info!(device = %output.device_name, "output started");

    // 5. Install Ctrl-C handler. On signal, flip the shutdown bool and let
    //    the worker loop break on its next iteration.
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_signal = shutdown.clone();
    ctrlc::set_handler(move || {
        tracing::info!("Ctrl-C received, shutting down");
        shutdown_signal.store(true, Ordering::SeqCst);
    })
    .map_err(|e| anyhow::anyhow!("install ctrl-c handler: {e}"))?;

    // 6. Spawn the passthrough worker. Moves the consumer/producer halves
    //    of both ring buffers so the worker fully owns the data path.
    let worker_shutdown = shutdown.clone();
    let frame_samples = config.audio.frame_size_samples();
    let worker = thread::spawn(move || {
        // Pre-allocate the scratch frame once, outside the hot loop.
        let mut scratch = vec![0.0f32; frame_samples];

        while !worker_shutdown.load(Ordering::Relaxed) {
            let n = input_cons.pop_slice(&mut scratch);
            if n == 0 {
                // Ring under-full; wait briefly for the capture callback to
                // push more. 500 us is short enough to stay responsive but
                // long enough to avoid burning 100% CPU.
                thread::sleep(Duration::from_micros(500));
                continue;
            }
            // Push as much as fits. If the output ring is full we drop;
            // the output callback will zero-fill, which sounds like a brief
            // tick but is better than blocking the worker.
            output_prod.push_slice(&scratch[..n]);
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
