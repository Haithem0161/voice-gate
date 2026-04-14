use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

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
    // Real cpal enumeration lands in step 7 of the Phase 1 morph.
    anyhow::bail!("devices subcommand not yet implemented (Phase 1, step 7)")
}

/// `voicegate run --passthrough` -- mic to virtual-mic loopback with no ML.
fn cmd_run(passthrough: bool) -> Result<()> {
    if !passthrough {
        anyhow::bail!(
            "Phase 1 only implements `run --passthrough`. \
             Gated mode lands in Phase 4."
        );
    }

    let _config = Config::load()?;

    // Real passthrough pipeline lands in step 7.
    anyhow::bail!("run --passthrough not yet implemented (Phase 1, step 7)")
}
