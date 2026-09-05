use clap::{Parser, Subcommand};
use fleet_core::Core;
use tracing::{error, info};

mod commands;
mod ui;

#[derive(Parser)]
#[command(author, version = env!("FLEET_CLI_VERSION"), about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
    /// Enable debug logging (uses a debug filter when RUST_LOG is unset).
    #[arg(long, global = true)]
    pub debug: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Manage profiles
    Profile {
        #[command(subcommand)]
        command: ProfileCommands,
    },

    /// Check profile health and list dirty/update status
    Check { profile_id: String },

    /// Validate every managed file byte-for-byte
    Validate {
        profile_id: String,
        /// Disable progress bars/spinners (useful for clean debug logs).
        #[arg(long)]
        no_progress: bool,
    },

    /// Run a sync oneshot and print live progress
    Sync {
        profile_id: String,
        /// Disable progress bars/spinners (useful for clean debug logs).
        #[arg(long)]
        no_progress: bool,
    },

    /// Launch Arma 3 using synced mods from the profile destination
    Launch {
        profile_id: String,
        /// Do not spawn, just print the command
        #[arg(long)]
        dry_run: bool,
        /// Extra args passed through to Arma/Steam
        #[arg(last = true)]
        args: Vec<String>,
    },

    /// Join the first Swifty repo server using the profile's synced mods
    Join {
        profile_id: String,
        /// Do not spawn, just print the command
        #[arg(long)]
        dry_run: bool,
        /// Override default launch args entirely
        #[arg(last = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum ProfileCommands {
    List,
    Add {
        id: String,
        name: String,
        #[arg(long, default_value = "")]
        source: String,
        #[arg(long, default_value = "")]
        dest: String,
    },
    Remove {
        name: String,
    },
}

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    fleet_core::logging::init(fleet_core::logging::LoggingConfig {
        project_dir_name: "fleet-cli",
        file_prefix: "fleet-cli",
        debug_enabled: cfg!(debug_assertions) && cli.debug,
    })?;
    let args: Vec<String> = std::env::args().collect();
    info!(?args, "fleet-cli launched");
    let core = Core::new_in_current_runtime_for_command()?;
    let result = commands::dispatch(&core, cli.command).await;
    if let Err(ref err) = result {
        error!(error = %err, "fleet-cli failed");
    }
    result
}
