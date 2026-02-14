use fleet_core::{Core, FlowResult};

use super::flow_run::{run_flow_session, DeletePolicy, FlowOutput, FlowRunOptions};
use super::load_profile;

pub async fn run(core: &Core, profile_id: &str) -> anyhow::Result<()> {
    let profile = load_profile(core, profile_id).await?;

    let session_id = core
        .start_repair(profile.id.clone())
        .await
        .map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))?;

    match run_flow_session(
        core,
        session_id,
        FlowRunOptions {
            delete_policy: DeletePolicy::AlwaysConfirm,
            output: FlowOutput::Quiet,
        },
    )
    .await?
    {
        FlowResult::Repair(summary) => {
            println!("---");
            println!("repair done");
            println!("duration_ms: {}", summary.duration_ms);
            println!("files_reconciled: {}", summary.files_reconciled);
            println!("files_deleted: {}", summary.files_deleted);
            println!("files_skipped_delete: {}", summary.files_skipped_delete);
        }
        FlowResult::Sync(_) | FlowResult::Check(_) => {
            return Err(anyhow::anyhow!("unexpected flow result"));
        }
    }

    Ok(())
}
