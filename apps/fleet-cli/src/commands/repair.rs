use fleet_core::{Core, OperationKind};

use super::flow_run::{run_repair_session, FlowOutput, FlowRunOptions};
use super::load_profile;

pub async fn run(core: &Core, profile_id: &str) -> anyhow::Result<()> {
    let profile = load_profile(core, profile_id).await?;

    let session_id = core
        .start_operation(profile.id.clone(), OperationKind::Repair)
        .await
        .map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))?;

    let summary = run_repair_session(
        core,
        session_id,
        FlowRunOptions {
            output: FlowOutput::Quiet,
        },
    )
    .await?;

    println!("---");
    println!("repair done");
    println!("duration_ms: {}", summary.duration_ms);
    println!("files_reconciled: {}", summary.files_reconciled);
    println!("files_deleted: {}", summary.files_deleted);

    Ok(())
}
