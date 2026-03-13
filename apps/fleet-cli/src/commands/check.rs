use fleet_core::{
    AssessScope, Core, LocalStateHealth, OperationKind, ProfileStateReport, RemoteFreshnessState,
};

use super::flow_run::{run_assess_session, FlowOutput, FlowRunOptions};
use super::{load_profile, start_operation};

pub(crate) async fn run(core: &Core, profile_id: &str) -> anyhow::Result<()> {
    let profile = load_profile(core, profile_id).await?;
    let report = run_check_report(core, profile_id, true).await?;
    print_check_report(&report, !profile.source.trim().is_empty());
    Ok(())
}

pub(crate) async fn run_check_report(
    core: &Core,
    profile_id: &str,
    include_remote: bool,
) -> anyhow::Result<ProfileStateReport> {
    let profile = load_profile(core, profile_id).await?;
    let operation = if include_remote {
        OperationKind::Assess(AssessScope::Remote)
    } else {
        OperationKind::Assess(AssessScope::Local)
    };

    let action_label = if include_remote {
        "remote assessment"
    } else {
        "local assessment"
    };
    let session_id = start_operation(core, profile.id.clone(), operation, action_label).await?;

    run_assess_session(
        core,
        session_id,
        FlowRunOptions {
            output: FlowOutput::Quiet,
        },
    )
    .await
}

pub(crate) fn print_check_report(report: &ProfileStateReport, has_repo_source: bool) {
    println!(
        "profile check: local={:?} remote={:?} (checked_at_unix_ms={})",
        report.local_health, report.remote_freshness, report.checked_at_unix_ms
    );

    let has_update = matches!(
        report.remote_freshness,
        Some(RemoteFreshnessState::UpdateAvailable)
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

    if matches!(
        report.local_health,
        LocalStateHealth::LocalStateMissing | LocalStateHealth::InventoryCorrupt
    ) && has_repo_source
    {
        println!("sync_repair_required: true (run sync to repair inventory and reconcile)");
    }

    if report.local_health == LocalStateHealth::LocalDrift
        && report.unexpected_delete_paths.is_empty()
    {
        println!(
            "local_drift_detected: true (modified/missing files likely; run sync to reconcile)"
        );
    }

    if has_repo_source && !report.unexpected_delete_paths.is_empty() {
        println!("sync_can_remove_unexpected_files: true");
    }
}
