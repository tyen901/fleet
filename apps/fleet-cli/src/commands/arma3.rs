use fleet_core::{ArmaLaunchResult, Core};
use tracing::{error, info};

fn print_built(out: &ArmaLaunchResult) {
    eprintln!("program: {}", out.program);
    eprintln!("args: {:?}", out.args);
    if let Some(pid) = out.pid {
        eprintln!("pid: {pid}");
    }
    info!(program = %out.program, ?out.args, pid = ?out.pid, "arma3 launch built");
}

pub async fn launch(
    core: &Core,
    profile_id: &str,
    dry_run: bool,
    args: Vec<String>,
) -> anyhow::Result<()> {
    let out = core
        .arma3_launch_by_profile_id(
            profile_id.to_string(),
            (!args.is_empty()).then_some(args),
            dry_run,
        )
        .await
        .map_err(|e| {
            error!(code = %e.code, message = %e.message, "arma3 launch failed");
            anyhow::anyhow!("{}: {}", e.code, e.message)
        })?;
    print_built(&out);
    Ok(())
}

pub async fn join(
    core: &Core,
    profile_id: &str,
    dry_run: bool,
    args: Vec<String>,
) -> anyhow::Result<()> {
    let out = core
        .arma3_join_by_profile_id(
            profile_id.to_string(),
            (!args.is_empty()).then_some(args),
            dry_run,
        )
        .await
        .map_err(|e| {
            error!(code = %e.code, message = %e.message, "arma3 join failed");
            anyhow::anyhow!("{}: {}", e.code, e.message)
        })?;
    print_built(&out);
    Ok(())
}
