use clap::{Parser, Subcommand};
use fleet_app::services::FleetServices;

use crate::update;

#[derive(Parser, Debug)]
#[command(name = "fleet", version, about = "Fleet CLI/GUI")]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
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
pub enum ProfileCmd {
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
pub struct SyncArgs {
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

    #[arg(long)]
    delete_unexpected: bool,
}

#[derive(Parser, Debug, Clone)]
pub struct LaunchArgs {
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    path: Option<std::path::PathBuf>,
    #[arg(long, default_value = "")]
    extra_args: String,
}

pub async fn run_cli(args: Args, services: FleetServices) -> Result<(), Box<dyn std::error::Error>> {
    let cmd = match args.cmd {
        Some(c) => c,
        None => return Ok(()), // Should be handled by main to launch GUI
    };

    let data = services.data;

    match cmd {
        Cmd::Gui => {
            // No-op here; handled in main
            Ok(())
        }

        Cmd::RegistryPath => {
            // Use service to be clean
            // DataService doesn't expose path directly in our trait, 
            // but we can just use the direct lib for this specific debug command 
            // or rely on snapshot warning if we really wanted purity.
            // For back-compat, we call the static method.
            let app = fleet_app::FleetApp::open_default()?;
            println!("{}", app.registry_path());
            Ok(())
        }

        Cmd::Profile { cmd } => {
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

                ProfileCmd::Add { name, repo_url, path, select, arma3_extra_args } => {
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

                ProfileCmd::Edit { id, name, repo_url, path, select, arma3_extra_args } => {
                    let update = fleet_app::ProfileUpdate {
                        name,
                        repo_url,
                        checkout_root: path,
                        select: if select { Some(true) } else { None },
                        arma3_extra_args,
                        arma3_enabled_mods: None,
                    };
                    data.update_profile(&id, update)?;
                    Ok(())
                }

                ProfileCmd::Remove { id, yes } => {
                    if !yes { return Err("refusing to remove without --yes".into()); }
                    data.delete_profile(&id)?;
                    Ok(())
                }

                ProfileCmd::Select { id } => {
                    data.select_profile(&id)?;
                    Ok(())
                }

                ProfileCmd::Init => {
                    // Logic remains in app for now or we expose init via service
                    let mut app = fleet_app::FleetApp::open_default()?;
                    app.init_registry()?;
                    Ok(())
                }

                ProfileCmd::Path => {
                    let app = fleet_app::FleetApp::open_default()?;
                    println!("{}", app.registry_path());
                    Ok(())
                }
            }
        }

        Cmd::Sync(sa) => {
            let sync = services.sync;

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
                unexpected_paths: if sa.delete_unexpected {
                    fleet_app::UnexpectedPathPolicy::Delete
                } else {
                    fleet_app::UnexpectedPathPolicy::Prompt
                },
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
            if let Some(path) = launch.path {
                data.launch_arma3_for_path(&path, &launch.extra_args)?;
                Ok(())
            } else if let Some(profile) = launch.profile {
                // Ignore extra_args for profile launch in CLI for parity with GUI service
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
