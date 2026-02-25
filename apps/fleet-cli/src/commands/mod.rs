mod arma3;
mod check;
mod clean;
mod flow_run;
mod profile;
mod repair;
mod sync;

use crate::Commands;
use fleet_core::Core;

pub async fn dispatch(core: &Core, command: Commands) -> anyhow::Result<()> {
    match command {
        Commands::Check { profile_id } => check::run(core, &profile_id).await,
        Commands::Profile { command } => profile::run(core, command).await,
        Commands::Clean { profile_id, yes } => clean::run(core, &profile_id, yes).await,
        Commands::Repair { profile_id } => repair::run(core, &profile_id).await,
        Commands::Sync {
            profile_id,
            no_progress,
        } => sync::run(core, &profile_id, no_progress).await,
        Commands::Launch {
            profile_id,
            dry_run,
            args,
        } => arma3::launch(core, &profile_id, dry_run, args).await,
        Commands::Join {
            profile_id,
            dry_run,
            args,
        } => arma3::join(core, &profile_id, dry_run, args).await,
    }
}

async fn load_profile(core: &Core, profile_id: &str) -> anyhow::Result<fleet_core::Profile> {
    core.load_profile(&profile_id.to_string()).await
}
