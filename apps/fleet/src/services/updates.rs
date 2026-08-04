use std::sync::OnceLock;

use semver::Version;
use velopack::{sources, UpdateCheck, UpdateManager, UpdateOptions};

const UPDATE_URL: &str = "https://github.com/tyen901/fleet/releases/latest/download";
static INSTALLED_VERSION: OnceLock<String> = OnceLock::new();

pub fn resolve_feed_url() -> Result<String, String> {
    if std::env::var("FLEET_DISABLE_UPDATES")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        return Err("Updates are disabled.".to_string());
    }

    if let Some(url) = std::env::var("FLEET_UPDATE_FEED")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        return Ok(url);
    }

    if let Some(url) = std::env::var("FLEET_UPDATE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        return Ok(url);
    }

    Ok(UPDATE_URL.to_string())
}

pub fn build_version_string() -> &'static str {
    option_env!("FLEET_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

pub fn installed_version_string() -> String {
    INSTALLED_VERSION
        .get_or_init(resolve_installed_version_string)
        .clone()
}

fn resolve_installed_version_string() -> String {
    if is_development_runtime() {
        return build_version_string().to_string();
    }

    UpdateManager::new(sources::NoneSource {}, None, None)
        .map(|um| um.get_current_version_as_string())
        .unwrap_or_else(|_| build_version_string().to_string())
}

pub fn current_build_allows_update_checks() -> bool {
    runtime_allows_update_checks(installed_version_string, is_development_runtime())
}

fn runtime_allows_update_checks(
    installed_version: impl FnOnce() -> String,
    development_runtime: bool,
) -> bool {
    if development_runtime {
        return false;
    }

    build_allows_update_checks(&installed_version(), false)
}

pub fn check_for_updates(feed_url: &str) -> Result<Option<String>, String> {
    if feed_url.trim().is_empty() || feed_url == "disabled" {
        return Err("Updates are disabled.".to_string());
    }
    if !current_build_allows_update_checks() {
        return Ok(None);
    }

    let options = UpdateOptions::default();
    let um = UpdateManager::new(sources::HttpSource::new(feed_url), Some(options), None)
        .map_err(|e| e.to_string())?;

    match um.check_for_updates().map_err(|e| e.to_string())? {
        UpdateCheck::UpdateAvailable(update) => Ok(Some(update.TargetFullRelease.Version)),
        UpdateCheck::NoUpdateAvailable | UpdateCheck::RemoteIsEmpty => Ok(None),
    }
}

pub fn download_apply_and_restart(feed_url: &str) -> Result<(), String> {
    if feed_url.trim().is_empty() || feed_url == "disabled" {
        return Err("Updates are disabled.".to_string());
    }
    if !current_build_allows_update_checks() {
        return Ok(());
    }

    let options = UpdateOptions::default();
    let um = UpdateManager::new(sources::HttpSource::new(feed_url), Some(options), None)
        .map_err(|e| e.to_string())?;

    let update = match um.check_for_updates().map_err(|e| e.to_string())? {
        UpdateCheck::UpdateAvailable(update) => update,
        UpdateCheck::NoUpdateAvailable | UpdateCheck::RemoteIsEmpty => return Ok(()),
    };

    um.download_updates(&update, None)
        .map_err(|e| e.to_string())?;
    um.apply_updates_and_restart(&update)
        .map_err(|e| e.to_string())?;

    Ok(())
}

fn is_development_version(version: &str) -> bool {
    let value = version.trim();
    if value.is_empty() {
        return false;
    }
    if value.contains("-dirty") {
        return true;
    }

    let Some(raw) = value
        .split_whitespace()
        .next()
        .and_then(|head| head.strip_prefix('v').or(Some(head)))
    else {
        return false;
    };

    Version::parse(raw).is_ok_and(|version| !version.pre.is_empty())
}

fn is_development_runtime() -> bool {
    cfg!(debug_assertions)
}

fn build_allows_update_checks(version: &str, development_runtime: bool) -> bool {
    !development_runtime && !is_development_version(version)
}

#[cfg(test)]
mod tests {
    use super::{build_allows_update_checks, is_development_version, runtime_allows_update_checks};

    #[test]
    fn release_versions_allow_update_checks() {
        assert!(!is_development_version("1.0.0"));
        assert!(!is_development_version("v1.0.0"));
        assert!(!is_development_version("1.0.0 (abc1234)"));
    }

    #[test]
    fn development_versions_skip_update_checks() {
        assert!(is_development_version("1.0.0-44"));
        assert!(is_development_version("v1.0.0-rc.1"));
        assert!(is_development_version("v1.0.0-dev.1"));
        assert!(is_development_version("v1.0.0-dirty"));
    }

    #[test]
    fn development_runtime_skips_update_checks() {
        assert!(!build_allows_update_checks("1.0.0", true));
    }

    #[test]
    fn development_runtime_does_not_resolve_installed_version() {
        assert!(!runtime_allows_update_checks(
            || panic!("installed version should not be resolved"),
            true,
        ));
    }

    #[test]
    fn release_runtime_with_release_version_allows_update_checks() {
        assert!(build_allows_update_checks("1.0.0", false));
    }
}
