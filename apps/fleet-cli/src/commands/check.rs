use fleet_core::{
    Core, InventoryCheckReport, LocalStateHealth, OperationKind, RepoCheckFreshness,
    RepoCheckReport,
};

use super::flow_run::{
    run_inventory_check_session, run_repo_check_session, FlowOutput, FlowRunOptions,
};
use super::{load_profile, start_operation};

pub(crate) async fn run(core: &Core, profile_id: &str) -> anyhow::Result<()> {
    let profile = load_profile(core, profile_id).await?;
    let repo_report = run_repo_check_report(core, profile_id).await?;
    let inventory_report = run_inventory_check_report(core, profile_id).await?;
    print_check_report(
        &repo_report,
        &inventory_report,
        !profile.source.trim().is_empty(),
    );
    Ok(())
}

pub(crate) async fn run_repo_check_report(
    core: &Core,
    profile_id: &str,
) -> anyhow::Result<RepoCheckReport> {
    let profile = load_profile(core, profile_id).await?;
    let session_id = start_operation(
        core,
        profile.id.clone(),
        OperationKind::CheckRepo,
        "repo check",
    )
    .await?;

    run_repo_check_session(
        core,
        session_id,
        FlowRunOptions {
            output: FlowOutput::Quiet,
        },
    )
    .await
}

pub(crate) async fn run_inventory_check_report(
    core: &Core,
    profile_id: &str,
) -> anyhow::Result<InventoryCheckReport> {
    let profile = load_profile(core, profile_id).await?;
    let session_id = start_operation(
        core,
        profile.id.clone(),
        OperationKind::CheckInventory,
        "inventory check",
    )
    .await?;

    run_inventory_check_session(
        core,
        session_id,
        FlowRunOptions {
            output: FlowOutput::Quiet,
        },
    )
    .await
}

pub(crate) fn print_check_report(
    repo_report: &RepoCheckReport,
    inventory_report: &InventoryCheckReport,
    has_repo_source: bool,
) {
    println!("repo_check:");
    println!("  freshness: {:?}", repo_report.freshness);
    println!(
        "  local_revision: {}",
        repo_report.local_revision.as_deref().unwrap_or("none")
    );
    println!(
        "  remote_revision: {}",
        repo_report.remote_revision.as_deref().unwrap_or("unknown")
    );
    println!("  checked_at_unix_ms: {}", repo_report.checked_at_unix_ms);

    println!("inventory_check:");
    println!("  local_health: {:?}", inventory_report.local_health);
    println!(
        "  checked_at_unix_ms: {}",
        inventory_report.checked_at_unix_ms
    );
    println!(
        "  expected_missing_in_inventory: {}",
        inventory_report.expected_missing_in_inventory_count
    );
    println!(
        "  unexpected_paths: {}",
        inventory_report.unexpected_delete_paths.len()
    );
    for path in &inventory_report.unexpected_delete_paths {
        println!("    - {path}");
    }

    let has_update = matches!(repo_report.freshness, RepoCheckFreshness::UpdateAvailable);
    println!("update_available: {}", has_update);

    if matches!(
        inventory_report.local_health,
        LocalStateHealth::MissingDestination
            | LocalStateHealth::LocalStateMissing
            | LocalStateHealth::InventoryCorrupt
    ) && has_repo_source
    {
        let repair_reason = match &inventory_report.local_health {
            LocalStateHealth::MissingDestination => {
                "local folder missing; run sync to recreate it and reconcile"
            }
            _ => "run sync to repair inventory and reconcile",
        };
        println!("sync_repair_required: true ({repair_reason})");
    }

    if inventory_report.local_health == LocalStateHealth::LocalDrift
        && inventory_report.unexpected_delete_paths.is_empty()
    {
        println!(
            "local_drift_detected: true (modified/missing files likely; run sync to reconcile)"
        );
    }

    if has_repo_source && !inventory_report.unexpected_delete_paths.is_empty() {
        println!("sync_can_remove_unexpected_files: true");
    }
}
