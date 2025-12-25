use clap::Parser;
use fleet_app::services;

mod cli;
mod update;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    velopack::VelopackApp::build().run();

    let args = cli::Args::parse();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = rt.handle().clone();

    let (services, _warning) = services::open_default_with_recovery(handle)?;

    if args.cmd.is_none() || matches!(args.cmd, Some(cli::Cmd::Gui)) {
        fleet_lib::run(services);
        Ok(())
    } else {
        rt.block_on(async move { cli::run_cli(args, services).await })?;
        Ok(())
    }
}
