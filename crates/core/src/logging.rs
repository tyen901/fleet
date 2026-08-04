use chrono::{Duration, Utc};
use directories::ProjectDirs;
use std::io::LineWriter;
use std::sync::Mutex;
use std::sync::OnceLock;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::fmt::time::UtcTime;
use tracing_subscriber::prelude::*;

static LOG_INITIALIZED: OnceLock<()> = OnceLock::new();

const TAO_WINDOWS_EVENT_LOOP_TARGET: &str = "tao::platform_impl::platform::event_loop::runner";

fn target_filter(level: tracing::Level) -> Targets {
    Targets::new()
        .with_default(LevelFilter::from_level(level))
        .with_target(TAO_WINDOWS_EVENT_LOOP_TARGET, LevelFilter::ERROR)
}

#[derive(Clone, Copy, Debug)]
pub struct LoggingConfig {
    pub project_dir_name: &'static str,
    pub file_prefix: &'static str,
    pub debug_enabled: bool,
}

pub fn init(config: LoggingConfig) -> anyhow::Result<()> {
    if LOG_INITIALIZED.get().is_some() {
        return Ok(());
    }

    let log_dir = if let Some(dir) = std::env::var_os("FLEET_LOG_DIR") {
        std::path::PathBuf::from(dir)
    } else {
        let proj = ProjectDirs::from("com", "fleet", config.project_dir_name)
            .ok_or_else(|| anyhow::anyhow!("failed to resolve log directory"))?;
        proj.data_dir().join("logs")
    };
    std::fs::create_dir_all(&log_dir)?;
    prune_old_logs(&log_dir);

    let level = if config.debug_enabled {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    let filename = format!(
        "{}-{}.log",
        config.file_prefix,
        Utc::now().format("%Y%m%d-%H%M%S")
    );
    let file_path = log_dir.join(filename);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)?;
    // `LineWriter` flushes on newline; tracing-subscriber writes one event per line.
    let file_writer = Mutex::new(LineWriter::new(file));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_timer(UtcTime::rfc_3339())
        .with_target(true)
        .compact();

    let stdout_layer = tracing_subscriber::fmt::layer().compact();
    // Tao's Windows runner repairs out-of-order lifecycle boundaries before logging its
    // diagnostic warnings. Dioxus owns that event loop, so Fleet cannot act on those messages.
    // Keep errors from the runner while preventing the repaired transitions from flooding logs.
    let target_filter = target_filter(level);

    tracing_subscriber::registry()
        .with(target_filter)
        .with(file_layer)
        .with(stdout_layer)
        .with(sentry::integrations::tracing::layer())
        .try_init()
        .map_err(anyhow::Error::new)?;

    let _ = LOG_INITIALIZED.set(());
    Ok(())
}

fn prune_old_logs(log_dir: &std::path::Path) {
    let cutoff = Utc::now() - Duration::days(30);
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let modified = chrono::DateTime::<Utc>::from(modified);
        if modified < cutoff {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{target_filter, TAO_WINDOWS_EVENT_LOOP_TARGET};

    #[test]
    fn tao_windows_runner_warnings_are_filtered_without_hiding_errors() {
        let filter = target_filter(tracing::Level::INFO);

        assert!(!filter.would_enable(TAO_WINDOWS_EVENT_LOOP_TARGET, &tracing::Level::WARN));
        assert!(filter.would_enable(TAO_WINDOWS_EVENT_LOOP_TARGET, &tracing::Level::ERROR));
        assert!(filter.would_enable("fleet_core", &tracing::Level::INFO));
    }
}
