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
    println!("local_health: {:?}", report.local_health);
    println!("remote_freshness: {:?}", report.remote_freshness);
    println!(
        "remaining_unexpected_files: {}",
        report.unexpected_delete_paths.len()
    );

    Ok(())
}
