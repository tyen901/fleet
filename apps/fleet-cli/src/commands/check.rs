use fleet_core::{
    CheckReport, Core, LocalFileHealth, LocalFileReport, OperationKind, RepoCheckFreshness,
    RepoCheckReport,
};

use super::flow_run::{run_check_session, FlowOutput, FlowRunOptions};
use super::{load_profile, start_operation};

pub(crate) async fn run(core: &Core, profile_id: &str) -> anyhow::Result<()> {
    let report = run_check(core, profile_id).await?;
    print_check_report(&report.repo, &report.local);
    Ok(())
}

pub(crate) async fn run_check(core: &Core, profile_id: &str) -> anyhow::Result<CheckReport> {
    let profile = load_profile(core, profile_id).await?;
    let session_id = start_operation(core, profile.id, OperationKind::Check, "check").await?;
    run_check_session(
        core,
        session_id,
        FlowRunOptions {
            output: FlowOutput::Quiet,
        },
    )
    .await
}

pub(crate) fn print_check_report(repo: &RepoCheckReport, local: &LocalFileReport) {
    println!("repo_check:");
    println!("  freshness: {:?}", repo.freshness);
    println!(
        "  local_revision: {}",
        repo.local_revision.as_deref().unwrap_or("none")
    );
    println!(
        "  remote_revision: {}",
        repo.remote_revision.as_deref().unwrap_or("unknown")
    );
    println!("  checked_at_unix_ms: {}", repo.checked_at_unix_ms);

    println!("local_check:");
    println!("  verification: {:?}", local.verification);
    println!("  health: {:?}", local.health);
    println!("  checked_at_unix_ms: {}", local.checked_at_unix_ms);
    println!(
        "update_available: {}",
        matches!(repo.freshness, RepoCheckFreshness::UpdateAvailable)
    );
    if matches!(
        local.health,
        LocalFileHealth::Missing
            | LocalFileHealth::Dirty
            | LocalFileHealth::MissingDestination
            | LocalFileHealth::ExpectedStateUnavailable
            | LocalFileHealth::InventoryUnavailable
    ) {
        println!("sync_required: true");
    }
    if matches!(
        local.health,
        LocalFileHealth::Missing | LocalFileHealth::Dirty
    ) {
        println!("local_files_dirty: true");
    }
}
