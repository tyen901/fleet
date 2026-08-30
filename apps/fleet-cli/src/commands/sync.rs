use fleet_core::{Core, OperationKind};

use super::flow_run::{run_sync_session, FlowOutput, FlowRunOptions};
use super::{load_profile, start_operation};

pub async fn run(core: &Core, profile_id: &str, no_progress: bool) -> anyhow::Result<()> {
    let profile = load_profile(core, profile_id).await?;
    let session_id = start_operation(core, profile.id.clone(), OperationKind::Sync, "sync").await?;

    let report = run_sync_session(
        core,
        session_id,
        FlowRunOptions {
            output: FlowOutput::Progress { no_progress },
        },
    )
    .await?;

    println!("---");
    println!("done");
    println!("manifest_health: {:?}", report.inventory.manifest_health);
    println!("repo_freshness: {:?}", report.repo.freshness);
    println!(
        "unexpected_health: {:?}",
        report.inventory.unexpected_health
    );

    Ok(())
}
