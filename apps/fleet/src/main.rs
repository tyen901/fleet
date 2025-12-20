use clap::{Parser, Subcommand};

mod arma3;
mod gui;
mod registry;

#[derive(Parser, Debug)]
struct CliArgs {
    #[command(subcommand)]
    command: Option<CliCommand>,
    #[command(flatten)]
    sync: SyncArgs,
}

#[derive(Subcommand, Debug)]
enum CliCommand {
    Launch(LaunchArgs),
}

#[derive(Parser, Debug, Default, Clone)]
struct SyncArgs {
    #[clap(long)]
    repo_url: String,
    #[clap(long)]
    path: std::path::PathBuf,

    #[clap(long, default_value_t = 256)]
    full_download_part_threshold: usize,

    #[clap(long, default_value_t = 0.60)]
    full_download_byte_ratio_threshold: f64,

    #[clap(long)]
    max_concurrent_files: Option<usize>,

    #[clap(long)]
    max_concurrent_range_requests: Option<usize>,

    #[clap(long, default_value_t = 1024 * 1024)]
    io_buffer_bytes: usize,
}

#[derive(Parser, Debug, Clone)]
struct LaunchArgs {
    #[clap(long)]
    profile: Option<String>,
    #[clap(long)]
    path: Option<std::path::PathBuf>,
    #[clap(long, default_value = "")]
    extra_args: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();

    if argv.len() == 1 {
        return gui::run_gui();
    }

    if argv.get(1).and_then(|s| s.to_str()) == Some("-cli") {
        let cli = CliArgs::parse_from(argv.iter().take(1).chain(argv.iter().skip(2)));
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        return rt.block_on(run_cli(cli));
    }

    eprintln!("Usage:");
    eprintln!("  fleet               # launch UI");
    eprintln!("  fleet -cli [options]# run CLI");
    eprintln!();
    eprintln!("Example:");
    eprintln!("  fleet -cli --repo-url https://example/ --path /tmp/fleet");
    Ok(())
}

async fn run_cli(args: CliArgs) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        Some(CliCommand::Launch(launch)) => run_launch(launch).await,
        None => run_sync(args.sync).await,
    }
}

async fn run_sync(args: SyncArgs) -> Result<(), Box<dyn std::error::Error>> {
    use camino::Utf8PathBuf;
    use tokio::sync::mpsc;

    let repo_url = registry::normalize_repo_url(&args.repo_url);
    let opts = apply_options_from_args(&args);

    let checkout_root = Utf8PathBuf::from_path_buf(args.path.clone()).map_err(|_| {
        let err: Box<dyn std::error::Error> = "checkout path must be valid UTF-8".into();
        err
    })?;

    let (tx, mut rx) = mpsc::channel::<coordinator::events::Event>(1024);

    let sync_task = tokio::spawn({
        let checkout_root = checkout_root.clone();
        let repo_url = repo_url.clone();
        async move {
            coordinator::sync_checkout_with_events(
                &repo_url,
                &checkout_root,
                coordinator::SyncOptions {
                    apply: opts,
                    ..coordinator::SyncOptions::default()
                },
                Some(tx),
            )
            .await
        }
    });

    while let Some(ev) = rx.recv().await {
        println!("{ev:?}");
        if matches!(ev, coordinator::events::Event::Finished) {
            break;
        }
    }

    sync_task.await??;
    Ok(())
}

async fn run_launch(args: LaunchArgs) -> Result<(), Box<dyn std::error::Error>> {
    let (base_path, extra_args, enabled_mods) = if let Some(path) = args.path {
        (path, args.extra_args, Vec::new())
    } else if let Some(profile_id) = args.profile {
        let reg_path = registry::registry_path()?;
        let reg = registry::load_registry(&reg_path)?;
        let profile = reg
            .profiles
            .into_iter()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| format!("profile not found: {profile_id}"))?;

        let extra = if args.extra_args.is_empty() {
            profile.arma3.extra_args
        } else {
            args.extra_args
        };

        (
            std::path::PathBuf::from(profile.checkout_root),
            extra,
            profile.arma3.enabled_mods,
        )
    } else {
        return Err("launch requires --profile or --path".into());
    };

    let url = arma3::build_arma3_steam_url(&base_path, &enabled_mods, &extra_args)?;
    arma3::launch_arma3_via_steam(url)?;
    Ok(())
}

fn apply_options_from_args(args: &SyncArgs) -> sync_apply::ApplyOptions {
    let mut apply = sync_apply::ApplyOptions {
        full_download_part_threshold: args.full_download_part_threshold,
        full_download_byte_ratio_threshold: args.full_download_byte_ratio_threshold.clamp(0.0, 1.0),
        io_buffer_bytes: args.io_buffer_bytes.max(64 * 1024),
        ..sync_apply::ApplyOptions::default()
    };

    if let Some(v) = args.max_concurrent_files {
        apply.max_concurrent_files = v.max(1);
    }
    if let Some(v) = args.max_concurrent_range_requests {
        apply.max_concurrent_range_requests = v.max(1);
    }

    apply
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

        let apply = apply_options_from_args(&args);
        assert_eq!(apply.full_download_part_threshold, 32);
        assert_eq!(apply.full_download_byte_ratio_threshold, 1.0);
        assert_eq!(apply.max_concurrent_files, 1);
        assert_eq!(apply.max_concurrent_range_requests, 4);
        assert_eq!(apply.io_buffer_bytes, 64 * 1024);
    }

    #[test]
    fn normalize_repo_url_adds_trailing_slash() {
        assert_eq!(
            registry::normalize_repo_url("https://host/path"),
            "https://host/path/"
        );
        assert_eq!(
            registry::normalize_repo_url("https://host/path/"),
            "https://host/path/"
        );
    }
}
