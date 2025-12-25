use clap::Parser;
use fleet_app::services;

mod cli;
mod update;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Velopack startup
    velopack::VelopackApp::build().run();

    // Check args
    let args = cli::Args::parse();

    // Initialize Runtime
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = rt.handle().clone();

    // Initialize Services
    let (services, _warning) = services::open_default_with_recovery(handle)?;

    // Decide mode
    if args.cmd.is_none() || matches!(args.cmd, Some(cli::Cmd::Gui)) {
        // Run Tauri GUI
        // Note: tauri::Builder manages its own runtime, but we pass the tokio handle
        // into services which works fine.
        fleet::run(services);
        Ok(())
    } else {
        // Run CLI
        // You will need to adapt your existing CLI run loop to accept the pre-built services
        // or re-initialize them inside run_cli if necessary.
        // For strict architecture alignment, CLI should consume the same services instance.
        rt.block_on(async move { cli::run_cli(args, services).await })?;
        Ok(())
    }
}
