use fleet_core::{Core, LocalStateHealth, OperationKind};
use std::io::{self, Write};

use super::check::run_check_report;
use super::flow_run::{run_clean_session, FlowOutput, FlowRunOptions};
use super::start_operation;

fn prompt_confirm_clean(paths: &[String]) -> anyhow::Result<bool> {
    println!(
        "Clean will delete {} unexpected files and remove empty parent folders:",
        paths.len()
    );
    for path in paths {
        println!("  - {}", path);
    }
    print!("Proceed? [y/N]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let normalized = input.trim().to_ascii_lowercase();
    Ok(matches!(normalized.as_str(), "y" | "yes"))
}

pub async fn run(core: &Core, profile_id: &str, yes: bool) -> anyhow::Result<()> {
    let precheck = run_check_report(core, profile_id, false).await?;
    if precheck.unexpected_delete_paths.is_empty() {
        println!("---");
        println!("clean skipped");
        println!("No unexpected files found.");
        return Ok(());
    }

    if !yes && !prompt_confirm_clean(&precheck.unexpected_delete_paths)? {
        println!("Cleanup canceled.");
        return Ok(());
    }

    let session_id = start_operation(
        core,
        precheck.profile_id.clone(),
        OperationKind::Clean,
        "cleanup",
    )
    .await?;

    let report = run_clean_session(
        core,
        session_id,
        FlowRunOptions {
            output: FlowOutput::Quiet,
        },
    )
    .await?;

    println!("---");
    println!("clean done");
    println!(
        "remaining_unexpected_files: {}",
        report.unexpected_delete_paths.len()
    );
    println!("local_health: {:?}", report.local_health);
    if report.local_health == LocalStateHealth::LocalDrift
        && !report.unexpected_delete_paths.is_empty()
    {
        println!("Some unexpected files remain after cleanup.");
    }

    Ok(())
}
