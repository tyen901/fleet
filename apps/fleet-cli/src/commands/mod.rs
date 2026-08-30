mod arma3;
mod check;
mod flow_run;
mod profile;
mod sync;
mod validate;

use crate::Commands;
use fleet_core::{ApiError, Core, OperationKind, ProfileId};

pub async fn dispatch(core: &Core, command: Commands) -> anyhow::Result<()> {
    match command {
        Commands::Check { profile_id } => check::run(core, &profile_id).await,
        Commands::Validate {
            profile_id,
            no_progress,
        } => validate::run(core, &profile_id, no_progress).await,
        Commands::Profile { command } => profile::run(core, command).await,
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

async fn start_operation(
    core: &Core,
    profile_id: ProfileId,
    operation: OperationKind,
    action_label: &str,
) -> anyhow::Result<u64> {
    core.start_operation(profile_id, operation)
        .await
        .map_err(|err| map_start_operation_error(err, action_label))
}

fn map_start_operation_error(err: ApiError, action_label: &str) -> anyhow::Error {
    if err.code == "profile_busy" {
        return anyhow::anyhow!("profile_busy: cannot start {action_label}");
    }

    anyhow::anyhow!("{}: {}", err.code, err.message)
}
