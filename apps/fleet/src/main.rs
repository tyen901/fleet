use clap::{Parser, Subcommand};

mod gui;

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
    RegistryPath,
}

#[derive(Subcommand, Debug)]
enum ProfileCmd {
    List { #[arg(long)] json: bool },
    Show { id: Option<String>, #[arg(long)] json: bool },
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        repo_url: String,
        #[arg(long)]
        path: String,
        #[arg(long, default_value_t = true)]
        select: bool,
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
    },
    Remove { id: String, #[arg(long)] yes: bool },
    Select { id: String },
    Init,
    Path,
}

#[derive(Parser, Debug, Clone)]
struct SyncArgs {
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    repo_url: Option<String>,
    #[arg(long)]
    path: Option<std::path::PathBuf>,

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

    #[arg(long)]
    json_events: bool,
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
    let args = Args::parse();

    if args.cmd.is_none() {
        return gui::run_gui();
    }

    if matches!(args.cmd, Some(Cmd::Gui)) {
        return gui::run_gui();
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move { run_cli(args).await })?;

    Ok(())
}

async fn run_cli(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = fleet_app::FleetApp::open_default()?;

    match args.cmd.unwrap() {
        Cmd::Gui => {
            gui::run_gui()?;
        }
        Cmd::RegistryPath => {
            println!("{}", app.registry_path());
        }
        Cmd::Profile { cmd } => match cmd {
            ProfileCmd::List { json } => {
                let profiles = app.list_profiles();
                if json {
                    println!("{}", serde_json::to_string_pretty(&profiles)?);
                } else {
                    for p in profiles {
                        println!("{}  {}  {}  {}", p.id, p.name, p.repo_url, p.checkout_root);
                    }
                }
            }
            ProfileCmd::Show { id, json } => {
                let profile = if let Some(id) = id {
                    app.get_profile(&id)
                } else {
                    app.selected_profile()
                };
                let Some(profile) = profile else {
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
            }
            ProfileCmd::Add {
                name,
                repo_url,
                path,
                select,
            } => {
                let profile = app.add_profile(&name, &repo_url, &path, select)?;
                println!("{}", profile.id);
            }
            ProfileCmd::Edit {
                id,
                name,
                repo_url,
                path,
                select,
            } => {
                let update = fleet_app::ProfileUpdate {
                    name,
                    repo_url,
                    checkout_root: path,
                    select: if select { Some(true) } else { None },
                    arma3_extra_args: None,
                };
                app.update_profile(&id, update)?;
            }
            ProfileCmd::Remove { id, yes } => {
                if !yes {
                    return Err("refusing to remove without --yes".into());
                }
                app.remove_profile(&id)?;
            }
            ProfileCmd::Select { id } => {
                app.select_profile(&id)?;
            }
            ProfileCmd::Init => {
                app.init_registry()?;
            }
            ProfileCmd::Path => {
                println!("{}", app.registry_path());
            }
        },
        Cmd::Sync(sa) => {
            let tuning = fleet_app::SyncTuning {
                full_download_part_threshold: sa.full_download_part_threshold,
                full_download_byte_ratio_threshold: sa.full_download_byte_ratio_threshold,
                max_concurrent_files: sa.max_concurrent_files,
                max_concurrent_range_requests: sa.max_concurrent_range_requests,
                io_buffer_bytes: sa.io_buffer_bytes,
                use_index: sa.use_index,
            };

            let (ev_tx, mut ev_rx) = tokio::sync::mpsc::channel::<coordinator::events::Event>(
                2048,
            );

            if sa.repo_url.is_some() ^ sa.path.is_some() {
                return Err("--repo-url and --path must be provided together".into());
            }

            let handle = tokio::runtime::Handle::current();

            let mut job = if let (Some(repo_url), Some(path)) = (sa.repo_url.as_deref(), sa.path) {
                let checkout = camino::Utf8PathBuf::from_path_buf(path)
                    .map_err(|_| "checkout path must be valid UTF-8")?;
                app.spawn_sync(repo_url, &checkout, handle.clone(), tuning, None, ev_tx)?
            } else if let Some(profile_id) = sa.profile {
                app.select_profile(&profile_id)?;
                app.spawn_sync_selected(handle.clone(), tuning, ev_tx)?
            } else {
                app.spawn_sync_selected(handle.clone(), tuning, ev_tx)?
            };

            let done_rx = job
                .take_done_rx()
                .ok_or("sync job missing completion channel")?;

            while let Some(ev) = ev_rx.recv().await {
                if sa.json_events {
                    let v = serde_json::json!({ "debug": format!("{ev:?}") });
                    println!("{}", serde_json::to_string(&v)?);
                } else {
                    println!("{ev:?}");
                }
                if matches!(ev, coordinator::events::Event::Finished) {
                    break;
                }
            }

            match done_rx.await? {
                Ok(()) => {}
                Err(e) => return Err(Box::<dyn std::error::Error>::from(e.to_string())),
            }
        }
        Cmd::Launch(launch) => {
            if let Some(path) = launch.path {
                app.launch_arma3_for_path(&path, &launch.extra_args)?;
            } else if let Some(profile) = launch.profile {
                app.launch_arma3_for_profile(&profile, Some(launch.extra_args))?;
            } else {
                return Err("launch requires --profile or --path".into());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_map_into_apply_options() {
        let args = SyncArgs::parse_from([
            "fleet",
            "--repo-url",
            "https://example.test/",
            "--path",
            "/tmp/fleet",
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
        };

        assert_eq!(tuning.full_download_part_threshold, 32);
        assert_eq!(tuning.full_download_byte_ratio_threshold, 1.5);
        assert_eq!(tuning.max_concurrent_files, Some(0));
        assert_eq!(tuning.max_concurrent_range_requests, Some(4));
        assert_eq!(tuning.io_buffer_bytes, 1024);
    }
}
