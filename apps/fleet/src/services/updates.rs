use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;
use velopack::{sources, UpdateCheck, UpdateManager, UpdateOptions};

const STABLE_UPDATE_URL: &str = "https://github.com/tyen901/fleet/releases/latest/download";
const DEV_RELEASES_BASE: &str = "https://github.com/tyen901/fleet/releases/download";
const DEV_TAGS_API: &str = "https://api.github.com/repos/tyen901/fleet/tags?per_page=100";

pub fn update_feed_url_hint(channel: &str) -> String {
    if std::env::var("FLEET_DISABLE_UPDATES")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        return "disabled".to_string();
    }

    if let Some(url) = std::env::var("FLEET_UPDATE_FEED")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        return url;
    }

    if let Some(url) = std::env::var("FLEET_UPDATE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        return url;
    }

    if channel == "dev" {
        "resolving dev tag…".to_string()
    } else {
        STABLE_UPDATE_URL.to_string()
    }
}

pub fn resolve_feed_url(channel: &str) -> Result<String, String> {
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

    if channel == "dev" {
        let tag = resolve_latest_dev_tag()?;
        Ok(format!("{DEV_RELEASES_BASE}/{tag}"))
    } else {
        Ok(STABLE_UPDATE_URL.to_string())
    }
}

pub fn velopack_channel(user_channel: &str) -> String {
    let normalized = user_channel.trim().to_lowercase();
    let suffix = if normalized.is_empty() {
        "stable".to_string()
    } else {
        normalized
    };
    let os = if cfg!(target_os = "windows") {
        "win"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    };
    format!("{os}-{suffix}")
}

pub fn build_version_string() -> &'static str {
    option_env!("FLEET_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

pub fn installed_version_string() -> String {
    UpdateManager::new(sources::NoneSource {}, None, None)
        .map(|um| um.get_current_version_as_string())
        .unwrap_or_else(|_| build_version_string().to_string())
}

pub fn check_for_updates(feed_url: &str, channel: &str) -> Result<Option<String>, String> {
    if feed_url.trim().is_empty() || feed_url == "disabled" {
        return Err("Updates are disabled.".to_string());
    }

    let options = UpdateOptions {
        ExplicitChannel: Some(velopack_channel(channel)),
        ..UpdateOptions::default()
    };
    let um = UpdateManager::new(sources::HttpSource::new(feed_url), Some(options), None)
        .map_err(|e| e.to_string())?;

    match um.check_for_updates().map_err(|e| e.to_string())? {
        UpdateCheck::UpdateAvailable(update) => Ok(Some(update.TargetFullRelease.Version)),
        UpdateCheck::NoUpdateAvailable | UpdateCheck::RemoteIsEmpty => Ok(None),
    }
}

pub fn download_apply_and_restart(feed_url: &str, channel: &str) -> Result<(), String> {
    if feed_url.trim().is_empty() || feed_url == "disabled" {
        return Err("Updates are disabled.".to_string());
    }

    let options = UpdateOptions {
        ExplicitChannel: Some(velopack_channel(channel)),
        ..UpdateOptions::default()
    };
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

#[derive(Deserialize)]
struct TagRef {
    name: String,
}

fn resolve_latest_dev_tag() -> Result<String, String> {
    let client = Client::builder()
        .user_agent("fleet-update-check")
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(DEV_TAGS_API).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("tag lookup failed: HTTP {}", resp.status()));
    }

    let tags: Vec<TagRef> = resp.json().map_err(|e| e.to_string())?;
    resolve_latest_dev_tag_name(tags.into_iter().map(|tag| tag.name))
}

fn resolve_latest_dev_tag_name<I>(tags: I) -> Result<String, String>
where
    I: IntoIterator<Item = String>,
{
    select_latest_dev_tag(tags).ok_or_else(|| "no dev tags found".to_string())
}

fn select_latest_dev_tag<I>(tags: I) -> Option<String>
where
    I: IntoIterator<Item = String>,
{
    let mut best: Option<(Version, String)> = None;
    for tag in tags {
        let Some(version) = parse_ci_dev_tag_version(&tag) else {
            continue;
        };
        match &best {
            Some((current, _)) if version <= *current => {}
            _ => best = Some((version, tag)),
        }
    }
    best.map(|(_, tag)| tag)
}

fn parse_ci_dev_tag_version(tag: &str) -> Option<Version> {
    let raw_tag = tag.strip_prefix("dev/")?;
    let ci_version = raw_tag.strip_prefix('v')?;
    Version::parse(ci_version).ok()
}

#[cfg(test)]
mod tests {
    use super::resolve_latest_dev_tag_name;

    #[test]
    fn release_dev_tag_outranks_prerelease_dev_tag() {
        let tag = resolve_latest_dev_tag_name(vec![
            "dev/v1.0.0-44".to_string(),
            "dev/v1.0.0".to_string(),
        ])
        .expect("expected valid dev tag");
        assert_eq!(tag, "dev/v1.0.0");
    }

    #[test]
    fn higher_semver_wins_across_minor_release_lines() {
        let tag = resolve_latest_dev_tag_name(vec![
            "dev/v1.0.0-999".to_string(),
            "dev/v1.0.1-1".to_string(),
        ])
        .expect("expected valid dev tag");
        assert_eq!(tag, "dev/v1.0.1-1");
    }

    #[test]
    fn ignores_non_matching_tags() {
        let tag = resolve_latest_dev_tag_name(vec![
            "v1.0.0".to_string(),
            "dev/1.0.0".to_string(),
            "dev/v1.0".to_string(),
            "dev/v1.0.0".to_string(),
        ])
        .expect("expected valid dev tag");
        assert_eq!(tag, "dev/v1.0.0");
    }

    #[test]
    fn returns_error_when_no_valid_dev_tags_exist() {
        let err = resolve_latest_dev_tag_name(vec![
            "v1.0.0".to_string(),
            "dev/1.0.0".to_string(),
            "dev/v1.0".to_string(),
        ])
        .expect_err("expected missing tag error");
        assert_eq!(err, "no dev tags found");
    }
}
