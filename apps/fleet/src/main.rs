// apps/fleet/src/main.rs
use clap::{Parser, Subcommand};

mod update;

#[derive(Parser, Debug)]
#[command(name = "fleet", version, about = "Fleet CLI/GUI")]
struct Args {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    Gui,
    Profile {
        #[command(subcommand)]
        cmd: ProfileCmd,
    },
    Sync(SyncArgs),
    Launch(LaunchArgs),
    Update(update::UpdateArgs),
    RegistryPath,
}

#[derive(Subcommand, Debug)]
enum ProfileCmd {
    List {
        #[arg(long)]
        json: bool,
    },
    Show {
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        repo_url: String,
        #[arg(long)]
        path: String,
        #[arg(long, default_value_t = true)]
        select: bool,
        #[arg(long, default_value = "")]
        arma3_extra_args: String,
    },
    Edit {
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        repo_url: Option<String>,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        select: bool,
        #[arg(long)]
        arma3_extra_args: Option<String>,
    },
    Remove {
        id: String,
        #[arg(long)]
        yes: bool,
    },
    Select {
        id: String,
    },
    Init,
    Path,
}

#[derive(Parser, Debug, Clone)]
struct SyncArgs {
    #[arg(long)]
    profile: Option<String>,

    #[arg(long, default_value_t = 256)]
    full_download_part_threshold: usize,

    #[arg(long, default_value_t = 0.60)]
    full_download_byte_ratio_threshold: f64,

    #[arg(long)]
    max_concurrent_files: Option<usize>,

    #[arg(long)]
    max_concurrent_range_requests: Option<usize>,

    #[arg(long, default_value_t = 1024 * 1024)]
    io_buffer_bytes: usize,

    #[arg(long, default_value_t = true)]
    use_index: bool,

    #[arg(long, default_value = "repair")]
    mode: String,
}

#[derive(Parser, Debug, Clone)]
struct LaunchArgs {
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    path: Option<std::path::PathBuf>,
    #[arg(long, default_value = "")]
    extra_args: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Must be the first thing to run; it may restart/exit the process for install/update tasks.
    velopack::VelopackApp::build().run();

    let args = Args::parse();

    if args.cmd.is_none() || matches!(args.cmd, Some(Cmd::Gui)) {
        return run_gui();
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move { run_cli(args).await })?;

    Ok(())
}

async fn run_cli(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    match args.cmd.unwrap() {
        Cmd::Gui => run_gui(),

        // NOTE: kept as a minimal direct FleetApp call for compatibility.
        // If you want strict service-only CLI, add DataService::registry_path().
        Cmd::RegistryPath => {
            let app = fleet_app::FleetApp::open_default()?;
            println!("{}", app.registry_path());
            Ok(())
        }

        Cmd::Profile { cmd } => {
            let handle = tokio::runtime::Handle::current();
            let (services, _warning) = fleet_app::services::open_default_with_recovery(handle)?;
            let data = services.data;

            match cmd {
                ProfileCmd::List { json } => {
                    let snap = data.snapshot();
                    if json {
                        println!("{}", serde_json::to_string_pretty(&snap.profiles)?);
                    } else {
                        for p in &snap.profiles {
                            println!("{}  {}  {}  {}", p.id, p.name, p.repo_url, p.checkout_root);
                        }
                    }
                    Ok(())
                }

                ProfileCmd::Show { id, json } => {
                    let snap = data.snapshot();
                    let prof = if let Some(id) = id {
                        snap.profiles.iter().find(|p| p.id == id).cloned()
                    } else if let Some(sel) = &snap.selected_id {
                        snap.profiles.iter().find(|p| &p.id == sel).cloned()
                    } else {
                        None
                    };

                    let Some(profile) = prof else {
                        return Err("no such profile".into());
                    };

                    if json {
                        println!("{}", serde_json::to_string_pretty(&profile)?);
                    } else {
                        println!("id: {}", profile.id);
                        println!("name: {}", profile.name);
                        println!("repo: {}", profile.repo_url);
                        println!("path: {}", profile.checkout_root);
                        println!("last_sync_unix_s: {:?}", profile.last_sync_unix_s);
                    }
                    Ok(())
                }

                ProfileCmd::Add {
                    name,
                    repo_url,
                    path,
                    select,
                    arma3_extra_args,
                } => {
                    let created_id = data.create_profile(fleet_app::ProfileCreate {
                        name,
                        repo_url,
                        checkout_root: path,
                        select,
                        arma3_extra_args,
                        arma3_enabled_mods: Vec::new(),
                    })?;
                    println!("{created_id}");
                    Ok(())
                }

                ProfileCmd::Edit {
                    id,
                    name,
                    repo_url,
                    path,
                    select,
                    arma3_extra_args,
                } => {
                    let update = fleet_app::ProfileUpdate {
                        name,
                        repo_url,
                        checkout_root: path,
                        select: if select { Some(true) } else { None },
                        arma3_extra_args: arma3_extra_args.clone(),
                        arma3_enabled_mods: None,
                    };
                    data.update_profile(&id, update)?;
                    Ok(())
                }

                ProfileCmd::Remove { id, yes } => {
                    if !yes {
                        return Err("refusing to remove without --yes".into());
                    }
                    data.delete_profile(&id)?;
                    Ok(())
                }

                ProfileCmd::Select { id } => {
                    data.select_profile(&id)?;
                    Ok(())
                }

                ProfileCmd::Init => {
                    // Minimal approach: init remains on FleetApp today. If you want service-only,
                    // add DataService::init_registry().
                    let mut app = fleet_app::FleetApp::open_default()?;
                    app.init_registry()?;
                    Ok(())
                }

                ProfileCmd::Path => {
                    // Same as RegistryPath; kept for compatibility.
                    let app = fleet_app::FleetApp::open_default()?;
                    println!("{}", app.registry_path());
                    Ok(())
                }
            }
        }

        Cmd::Sync(sa) => {
            let handle = tokio::runtime::Handle::current();
            let (services, _warning) =
                fleet_app::services::open_default_with_recovery(handle.clone())?;
            let data = services.data.clone();
            let sync = services.sync.clone();

            // If a profile id is provided, select it via the data service.
            if let Some(profile_id) = &sa.profile {
                data.select_profile(profile_id)?;
            }

            let mode = match sa.mode.as_str() {
                "repair" => fleet_app::SyncMode::Repair,
                "fresh" => fleet_app::SyncMode::SyncFresh,
                "check" => fleet_app::SyncMode::Check,
                _ => return Err(format!("invalid mode: {}", sa.mode).into()),
            };

            let tuning = fleet_app::SyncTuning {
                full_download_part_threshold: sa.full_download_part_threshold,
                full_download_byte_ratio_threshold: sa.full_download_byte_ratio_threshold,
                max_concurrent_files: sa.max_concurrent_files,
                max_concurrent_range_requests: sa.max_concurrent_range_requests,
                io_buffer_bytes: sa.io_buffer_bytes,
                use_index: sa.use_index,
                emit_progress: true,
                mode,
                ..Default::default()
            };

            sync.start(mode, tuning)?;

            loop {
                let snap = sync.snapshot();
                eprintln!("progress: {:3}% {}", snap.percent, snap.status_line);

                if snap.finished {
                    if let Some(err) = &snap.error {
                        return Err(err.clone().into());
                    }
                    break;
                }

                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }

            Ok(())
        }

        Cmd::Launch(launch) => {
            let handle = tokio::runtime::Handle::current();
            let (services, _warning) = fleet_app::services::open_default_with_recovery(handle)?;
            let data = services.data;

            if let Some(path) = launch.path {
                data.launch_arma3_for_path(&path, &launch.extra_args)?;
                Ok(())
            } else if let Some(profile) = launch.profile {
                // If user supplied extra args, this currently isn’t threaded through the service.
                // Keep behavior consistent by using path-based launch for custom args, or update
                // FleetApp/Service later to support overrides.
                //
                // For now: launch the profile’s configured args.
                let _ = launch.extra_args;
                data.launch_arma3_for_profile(&profile)?;
                Ok(())
            } else {
                Err("launch requires --profile or --path".into())
            }
        }

        Cmd::Update(ua) => {
            update::run(&ua)?;
            Ok(())
        }
    }
}

fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    fleet_ui::run().map_err(|e| e.to_string().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_map_into_apply_options() {
        let args = SyncArgs::parse_from([
            "fleet",
            "--full-download-part-threshold",
            "32",
            "--full-download-byte-ratio-threshold",
            "1.5",
            "--max-concurrent-files",
            "0",
            "--max-concurrent-range-requests",
            "4",
            "--io-buffer-bytes",
            "1024",
        ]);

        let tuning = fleet_app::SyncTuning {
            full_download_part_threshold: args.full_download_part_threshold,
            full_download_byte_ratio_threshold: args.full_download_byte_ratio_threshold,
            max_concurrent_files: args.max_concurrent_files,
            max_concurrent_range_requests: args.max_concurrent_range_requests,
            io_buffer_bytes: args.io_buffer_bytes,
            use_index: true,
            ..Default::default()
        };

        assert_eq!(tuning.full_download_part_threshold, 32);
        assert_eq!(tuning.full_download_byte_ratio_threshold, 1.5);
        assert_eq!(tuning.max_concurrent_files, Some(0));
        assert_eq!(tuning.max_concurrent_range_requests, Some(4));
        assert_eq!(tuning.io_buffer_bytes, 1024);
    }
}
