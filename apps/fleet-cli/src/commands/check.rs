use fleet_core::{
    Core, InventoryCheckReport, ManifestHealth, OperationKind, RepoCheckFreshness, RepoCheckReport,
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
    println!("  manifest_health: {:?}", inventory_report.manifest_health);
    println!(
        "  unexpected_health: {:?}",
        inventory_report.unexpected_health
    );
    println!(
        "  checked_at_unix_ms: {}",
        inventory_report.checked_at_unix_ms
    );
    println!("  missing_paths: {}", inventory_report.missing_paths_count);
    println!(
        "  modified_paths: {}",
        inventory_report.modified_paths_count
    );
    println!(
        "  unexpected_paths: {}",
        inventory_report.unexpected_paths.len()
    );
    for path in &inventory_report.unexpected_paths {
        println!("    - {path}");
    }

    let has_update = matches!(repo_report.freshness, RepoCheckFreshness::UpdateAvailable);
    println!("update_available: {}", has_update);

    if matches!(
        inventory_report.manifest_health,
        ManifestHealth::Missing
            | ManifestHealth::Different
            | ManifestHealth::MissingDestination
            | ManifestHealth::InventoryUnavailable
    ) && has_repo_source
    {
        let repair_reason = match &inventory_report.manifest_health {
            ManifestHealth::MissingDestination => {
                "local folder missing; run sync to recreate it and materialize files"
            }
            _ => "run sync to repair inventory and materialize files",
        };
        println!("sync_repair_required: true ({repair_reason})");
    }

    if matches!(
        inventory_report.manifest_health,
        ManifestHealth::Missing | ManifestHealth::Different
    ) && inventory_report.unexpected_paths.is_empty()
    {
        println!(
            "local_drift_detected: true (modified/missing files likely; run sync to materialize)"
        );
    }

    if has_repo_source && !inventory_report.unexpected_paths.is_empty() {
        println!("cleanup_available: true");
    }
}
