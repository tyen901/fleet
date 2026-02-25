use fleet_core::{Core, LocalHealthState, OperationKind};
use std::io::{self, Write};

use super::check::run_check_report;
use super::flow_run::{run_clean_session, FlowOutput, FlowRunOptions};

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

    let mut session_id = None;
    for _ in 0..10 {
        match core
            .start_operation(precheck.profile_id.clone(), OperationKind::Clean)
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
        .ok_or_else(|| anyhow::anyhow!("pipeline_error: timed out waiting to start clean"))?;

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
    if report.local_health == LocalHealthState::LocalDrift
        && !report.unexpected_delete_paths.is_empty()
    {
        println!("Some unexpected files remain after cleanup.");
    }

    Ok(())
}
