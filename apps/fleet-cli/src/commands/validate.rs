use fleet_core::{Core, OperationKind};

use super::flow_run::{run_validation_session, FlowOutput, FlowRunOptions};
use super::{load_profile, start_operation};

pub async fn run(core: &Core, profile_id: &str, no_progress: bool) -> anyhow::Result<()> {
    let profile = load_profile(core, profile_id).await?;
    let session_id = start_operation(core, profile.id, OperationKind::Validate, "validate").await?;
    let report = run_validation_session(
        core,
        session_id,
        FlowRunOptions {
            output: FlowOutput::Progress { no_progress },
        },
    )
    .await?;

    println!("---");
    println!("validated");
    println!("local_health: {:?}", report.health);
    println!("missing_paths: {}", report.missing_paths_count);
    println!("modified_paths: {}", report.modified_paths_count);
    Ok(())
}
