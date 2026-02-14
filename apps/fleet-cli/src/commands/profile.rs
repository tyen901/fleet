use crate::ProfileCommands;
use fleet_core::{Core, FlowResult};

use super::flow_run::{run_flow_session, DeletePolicy, FlowOutput, FlowRunOptions};
use super::load_profile;

pub async fn run(core: &Core, command: ProfileCommands) -> anyhow::Result<()> {
    match command {
        ProfileCommands::List => {
            let cfg = core.list_profiles().await?;
            println!("{:<20} {:<20} {:<30}", "ID", "NAME", "SOURCE");
            for p in cfg.profiles {
                println!("{:<20} {:<20} {:<30}", p.id, p.name, p.source);
            }
        }
        ProfileCommands::Check { profile_id } => {
            let profile = load_profile(core, &profile_id).await?;
            let session_id = core
                .start_check(profile.id.clone())
                .await
                .map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))?;
            let report = match run_flow_session(
                core,
                session_id,
                FlowRunOptions {
                    delete_policy: DeletePolicy::AlwaysReject,
                    output: FlowOutput::Quiet,
                },
            )
            .await?
            {
                FlowResult::Check(report) => report,
                FlowResult::Sync(_) | FlowResult::Repair(_) => {
                    return Err(anyhow::anyhow!("unexpected flow result"));
                }
            };
            println!(
                "profile check: local={:?} remote={:?} (checked_at_unix_ms={})",
                report.local_health, report.remote_freshness, report.checked_at_unix_ms
            );
        }
        ProfileCommands::Add {
            id,
            name,
            source,
            dest,
        } => {
            let profile = fleet_core::Profile {
                id: id.clone(),
                name,
                source,
                destination: dest,
                ..Default::default()
            };
            let saved = core.save_profile(profile).await?;
            println!("Profile '{}' created.", saved.id);
        }
        ProfileCommands::Remove { name } => {
            core.delete_profile(&name).await?;
            println!("Profile '{}' removed.", name);
        }
    }
    Ok(())
}
