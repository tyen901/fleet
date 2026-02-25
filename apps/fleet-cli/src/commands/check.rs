use fleet_core::{
    Core, LocalHealthState, OperationKind, ProfileAssessmentReport, RemoteFreshnessState,
};

use super::flow_run::{run_check_session, FlowOutput, FlowRunOptions};
use super::load_profile;

pub(crate) async fn run(core: &Core, profile_id: &str) -> anyhow::Result<()> {
    let report = run_check_report(core, profile_id, true).await?;
    print_check_report(&report);
    Ok(())
}

pub(crate) async fn run_check_report(
    core: &Core,
    profile_id: &str,
    include_remote: bool,
) -> anyhow::Result<ProfileAssessmentReport> {
    let profile = load_profile(core, profile_id).await?;
    let operation = if include_remote {
        OperationKind::CheckRemote
    } else {
        OperationKind::CheckLocal
    };

    let mut session_id = None;
    for _ in 0..10 {
        match core.start_operation(profile.id.clone(), operation).await {
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

    let session_id = session_id.ok_or_else(|| {
        let scope = if include_remote { "remote" } else { "local" };
        anyhow::anyhow!("pipeline_error: timed out waiting to start {scope} check")
    })?;

    run_check_session(
        core,
        session_id,
        FlowRunOptions {
            output: FlowOutput::Quiet,
        },
    )
    .await
}

pub(crate) fn print_check_report(report: &ProfileAssessmentReport) {
    println!(
        "profile check: local={:?} remote={:?} (checked_at_unix_ms={})",
        report.local_health, report.remote_freshness, report.checked_at_unix_ms
    );

    let has_update = matches!(
        report.remote_freshness,
        RemoteFreshnessState::UpdateAvailable
    );
    println!("update_available: {}", has_update);

    if !report.unexpected_delete_paths.is_empty() {
        println!(
            "dirty_unexpected_files: {}",
            report.unexpected_delete_paths.len()
        );
        for path in &report.unexpected_delete_paths {
            println!("  - {}", path);
        }
    } else {
        println!("dirty_unexpected_files: 0");
    }

    if report.local_health == LocalHealthState::LocalDrift
        && report.unexpected_delete_paths.is_empty()
    {
        println!(
            "local_drift_detected: true (modified/missing files likely; run sync to reconcile)"
        );
    }
}
