use fleet_core::{Core, FlowResult, SyncSummary};

use super::flow_run::{run_flow_session, DeletePolicy, FlowOutput, FlowRunOptions};
use super::load_profile;

pub async fn run(
    core: &Core,
    profile_id: &str,
    no_progress: bool,
    no_delete: bool,
) -> anyhow::Result<()> {
    let profile = load_profile(core, profile_id).await?;
    let session_id = core
        .start_sync(profile.id.clone())
        .await
        .map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))?;

    let sum: SyncSummary = match run_flow_session(
        core,
        session_id,
        FlowRunOptions {
            delete_policy: if no_delete {
                DeletePolicy::AlwaysReject
            } else {
                DeletePolicy::Prompt
            },
            output: FlowOutput::Progress { no_progress },
        },
    )
    .await?
    {
        FlowResult::Sync(summary) => summary,
        FlowResult::Repair(_) | FlowResult::Check(_) => {
            return Err(anyhow::anyhow!("unexpected flow result"));
        }
    };

    println!("---");
    println!("done");
    println!("duration_ms: {}", sum.duration_ms);
    println!("bytes_downloaded: {}", sum.bytes_downloaded);
    println!("bytes_reused: {}", sum.bytes_reused);
    println!("files_finalized: {}", sum.files_finalized);

    Ok(())
}
