use crate::{CliScanStrategy, CliSyncMode};
use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use fleet_core::formats::RepositoryExternal;
use fleet_core::repo::Repository;
use fleet_pipeline::sync::{SyncMode, SyncOptions, SyncRequest};
use fleet_scanner::{ScanStats, Scanner};
use humansize::{format_size, DECIMAL};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::time::Duration;

pub async fn cmd_scan(
    path: Utf8PathBuf,
    output: Option<Utf8PathBuf>,
    strategy: CliScanStrategy,
) -> anyhow::Result<()> {
    println!(":: Scanning directory: {}", path);

    let strategy = match strategy {
        CliScanStrategy::Smart => fleet_scanner::ScanStrategy::SmartCache,
        CliScanStrategy::Force => fleet_scanner::ScanStrategy::ForceRehash,
    };

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );
    pb.enable_steady_tick(Duration::from_millis(100));

    let cb = {
        let pb = pb.clone();
        Box::new(move |stats: ScanStats| {
            pb.set_message(format!(
                "Scanned {} files ({})",
                stats.files_scanned,
                format_size(stats.bytes_processed, DECIMAL)
            ));
        })
    };

    let root = path.clone();
    let manifest = tokio::task::spawn_blocking(move || {
        Scanner::scan_directory(root.as_path(), strategy, Some(cb), None, None)
    })
    .await??;

    pb.finish_with_message("Scan complete.");

    let json = serde_json::to_string_pretty(&manifest)?;
    if let Some(out) = output {
        std::fs::write(&out, json)?;
        println!(":: Saved manifest to {}", out);
    } else {
        println!("{}", json);
    }

    Ok(())
}

pub async fn cmd_check(
    repo: String,
    local_path: Utf8PathBuf,
    mode: CliSyncMode,
) -> anyhow::Result<fleet_core::SyncPlan> {
    println!(":: Analyzing state...");
    println!("   Repo:  {}", repo);
    println!("   Local: {}", local_path);

    let client = fleet_infra::net::default_http_client().context("Failed to build HTTP client")?;
    let engine = fleet_pipeline::default_engine(client);

    let req = SyncRequest {
        repo_url: repo,
        local_root: local_path,
        mode: mode.into(),
        options: SyncOptions::default(),
        profile_id: None,
    };

    let plan = engine.plan(&req).await?;

    println!("\n:: Analysis Result");
    println!("   Pending Downloads: {}", plan.downloads.len());
    println!("   Pending Deletes:   {}", plan.deletes.len());
    println!("   Verified Files:    {}", plan.checks.len());

    Ok(plan)
}

pub async fn cmd_check_for_updates(repo: String, local_path: Utf8PathBuf) -> anyhow::Result<()> {
    println!(":: Checking for updates...");
    println!("   Repo:  {}", repo);
    println!("   Local: {}", local_path);

    let has_baseline = local_path.join(".fleet-local-manifest.json").exists()
        && local_path.join(".fleet-local-summary.json").exists();
    let mode = if has_baseline {
        SyncMode::FastCheck
    } else {
        SyncMode::SmartVerify
    };

    let client = fleet_infra::net::default_http_client().context("Failed to build HTTP client")?;
    let engine = fleet_pipeline::default_engine(client);

    let req = SyncRequest {
        repo_url: repo,
        local_root: local_path,
        mode,
        options: SyncOptions::default(),
        profile_id: None,
    };

    let plan = engine.plan(&req).await?;

    println!("\n:: Update Check Result");
    println!("   Pending Downloads: {}", plan.downloads.len());
    println!("   Pending Deletes:   {}", plan.deletes.len());

    if plan.downloads.is_empty() && plan.deletes.is_empty() {
        println!("   Status:            Up to date");
    } else {
        println!("   Status:            Updates available (run `sync`)");
    }

    Ok(())
}

pub async fn cmd_local_check(local_path: Utf8PathBuf) -> anyhow::Result<()> {
    println!(":: Local integrity check...");
    println!("   Local: {}", local_path);

    let has_baseline = local_path.join(".fleet-local-summary.json").exists();
    if !has_baseline {
        anyhow::bail!(
            "Unknown local state: missing `.fleet-local-summary.json` (run `repair` first)"
        );
    }

    let client = fleet_infra::net::default_http_client().context("Failed to build HTTP client")?;
    let engine = fleet_pipeline::default_engine(client);

    let req = SyncRequest {
        repo_url: String::new(),
        local_root: local_path,
        mode: SyncMode::MetadataOnly,
        options: SyncOptions::default(),
        profile_id: None,
    };

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );
    pb.enable_steady_tick(Duration::from_millis(100));

    let cb = {
        let pb = pb.clone();
        Box::new(move |stats: ScanStats| {
            pb.set_message(format!(
                "Scanned {} files ({})",
                stats.files_scanned,
                format_size(stats.bytes_processed, DECIMAL)
            ));
        })
    };

    let local_state = engine.scan_local_state(&req, Some(cb)).await?;
    pb.finish_with_message("Scan complete.");

    let plan = engine.compute_local_integrity_plan(&req, &local_state)?;

    println!("\n:: Local Integrity Result");
    println!("   Missing/Changed: {}", plan.downloads.len());
    println!("   Extra Files:     {}", plan.deletes.len());

    if plan.downloads.is_empty() && plan.deletes.is_empty() {
        println!("   Status:          Clean");
    } else {
        println!("   Status:          Dirty (run `sync` or investigate)");
    }

    Ok(())
}

pub async fn cmd_repair(repo: String, local_path: Utf8PathBuf) -> anyhow::Result<()> {
    println!(":: Repairing local state...");
    println!("   Repo:  {}", repo);
    println!("   Local: {}", local_path);

    let client = fleet_infra::net::default_http_client().context("Failed to build HTTP client")?;
    let engine = fleet_pipeline::default_engine(client);

    let req = SyncRequest {
        repo_url: repo,
        local_root: local_path,
        mode: SyncMode::SmartVerify,
        options: SyncOptions::default(),
        profile_id: None,
    };

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );
    pb.enable_steady_tick(Duration::from_millis(100));

    let cb = {
        let pb = pb.clone();
        Box::new(move |stats: ScanStats| {
            pb.set_message(format!(
                "Verified {} files ({})",
                stats.files_scanned,
                format_size(stats.bytes_processed, DECIMAL)
            ));
        })
    };

    let _ = engine.scan_local_state(&req, Some(cb)).await?;
    pb.finish_with_message("Local scan complete.");

    println!(":: Fetching remote manifest...");
    let remote = engine.fetch_remote_state(&req).await?;

    engine.persist_remote_snapshot(&req.local_root, &remote.manifest)?;

    println!(":: Repair complete.");
    println!("   Wrote `.fleet-local-manifest.json` and `.fleet-local-summary.json`");

    Ok(())
}

pub async fn cmd_sync(
    repo: String,
    path: Utf8PathBuf,
    mode: CliSyncMode,
    threads: usize,
    limit_mb: Option<u64>,
    cache_dir: Option<Utf8PathBuf>,
) -> anyhow::Result<fleet_pipeline::SyncResult> {
    println!(":: Synchronizing...");
    println!("   Target: {}", path);

    let client = fleet_infra::net::default_http_client().context("Failed to build HTTP client")?;
    let engine = fleet_pipeline::default_engine(client);

    let options = SyncOptions {
        max_threads: threads.clamp(1, 32),
        rate_limit_bytes: limit_mb.map(|mb| mb * 1024 * 1024),
        cache_root: cache_dir,
    };

    let req = SyncRequest {
        repo_url: repo,
        local_root: path,
        mode: mode.into(),
        options,
        profile_id: None,
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let engine_handle = tokio::spawn(async move { engine.plan_and_execute(&req, Some(tx)).await });

    let m = MultiProgress::new();
    let sty_main = ProgressStyle::with_template(
        "[{elapsed_precise}] {bar:40.cyan/blue} {bytes}/{total_bytes} {bytes_per_sec} ETA {eta} {msg}",
    )
    .unwrap()
    .progress_chars("=>-");

    let pb_main = m.add(ProgressBar::new(0));
    pb_main.set_style(sty_main);
    pb_main.set_message("Planning...");

    let mut total_bytes = 0u64;
    let mut downloaded_bytes = 0u64;
    let mut files_done = 0u64;
    let mut files_total = 0u64;

    while let Some(ev) = rx.recv().await {
        use fleet_infra::net::DownloadEvent;
        match ev {
            DownloadEvent::Started {
                total_bytes: size, ..
            } => {
                total_bytes = total_bytes.saturating_add(size);
                files_total = files_total.saturating_add(1);
                pb_main.set_length(total_bytes);
                pb_main.set_message(format!("Downloading {} files", files_total));
            }
            DownloadEvent::Progress { bytes_delta, .. } => {
                downloaded_bytes = downloaded_bytes.saturating_add(bytes_delta);
                pb_main.set_position(downloaded_bytes);
            }
            DownloadEvent::Completed { .. } => {
                files_done = files_done.saturating_add(1);
                pb_main.set_message(format!("Downloading {}/{} files", files_done, files_total));
            }
        }
    }

    let result = engine_handle.await??;
    pb_main.finish_with_message("Sync Complete");

    Ok(result)
}

/// Helper: Resolve mod paths by reading repo.json from the target directory.
pub fn resolve_mods_from_dir(local_root: &Utf8PathBuf) -> Result<Vec<Utf8PathBuf>> {
    let repo_json_path = local_root.join("repo.json");
    if !repo_json_path.exists() {
        // Fallback: treat the directory as a plain "@mod" folder root.
        // This supports users who keep a curated mod directory without a repo.json manifest.
        let mut mods = Vec::new();
        for entry in std::fs::read_dir(local_root)
            .with_context(|| format!("Failed to read directory {}", local_root))?
            .flatten()
        {
            let ft = match entry.file_type() {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !ft.is_dir() {
                continue;
            }

            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with('@') {
                continue;
            }

            let utf = match Utf8PathBuf::from_path_buf(entry.path()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            mods.push(utf);
        }

        mods.sort();
        return Ok(mods);
    }

    let content = std::fs::read_to_string(&repo_json_path)
        .with_context(|| format!("Failed to read {}", repo_json_path))?;

    // Parse via RepositoryExternal first (more permissive) then map into Repository.
    let repo_ext: RepositoryExternal =
        serde_json::from_str(&content).context("Failed to parse repo.json")?;
    let repo: Repository = repo_ext.into();

    let mut mods = Vec::new();
    for m in repo.required_mods {
        mods.push(local_root.join(&m.mod_name));
    }
    for m in repo.optional_mods {
        if m.enabled {
            mods.push(local_root.join(&m.mod_name));
        }
    }

    Ok(mods)
}
