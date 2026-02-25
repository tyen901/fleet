use fleet_core::{Core, OperationKind};

use super::flow_run::{run_sync_session, FlowOutput, FlowRunOptions};
use super::load_profile;

pub async fn run(core: &Core, profile_id: &str, no_progress: bool) -> anyhow::Result<()> {
    let profile = load_profile(core, profile_id).await?;
    let mut session_id = None;
    for _ in 0..10 {
        match core
            .start_operation(profile.id.clone(), OperationKind::Sync)
            .await
        {
            Ok(id) => {
                session_id = Some(id);
                break;
            }
            Err(e) if e.message.contains("already running for this profile") => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(e) => return Err(anyhow::anyhow!("{}: {}", e.code, e.message)),
        }
    }
    let session_id = session_id
        .ok_or_else(|| anyhow::anyhow!("pipeline_error: timed out waiting to start sync"))?;

    let sum = run_sync_session(
        core,
        session_id,
        FlowRunOptions {
            output: FlowOutput::Progress { no_progress },
        },
    )
    .await?;

    println!("---");
    println!("done");
    println!("duration_ms: {}", sum.duration_ms);
    println!("bytes_downloaded: {}", sum.bytes_downloaded);
    println!("bytes_reused: {}", sum.bytes_reused);
    println!("files_finalized: {}", sum.files_finalized);

    Ok(())
}
