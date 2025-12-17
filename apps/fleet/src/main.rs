use clap::Parser;

mod gui;
mod registry;

#[derive(Parser, Debug)]
struct CliArgs {
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

fn apply_options_from_args(args: &CliArgs) -> sync_apply::ApplyOptions {
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
        let args = CliArgs::parse_from([
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
