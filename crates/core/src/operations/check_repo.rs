use fleet_domain::health::{RepoCheckFreshness, RepoCheckReport};
use fleet_domain::{validated_repo_url, Profile};
use std::path::Path;

pub(crate) async fn check_repo(profile: &Profile, state_root: &Path) -> RepoCheckReport {
    let repo_url = match validated_repo_url(&profile.source) {
        Ok(repo_url) => repo_url,
        Err(_) => {
            return error_report(profile, None);
        }
    };
    let repo_cache_dir = fleet_domain::repo_cache_dir(state_root, &profile.id);
    let store = swifty_repo::FsRepoCacheStore::new(repo_cache_dir.clone());
    let downloads = fleet_download::DownloadService::new_default();
    match swifty_repo::probe_repo_freshness(repo_url, &store, &downloads).await {
        Ok(probe) => RepoCheckReport {
            profile_id: profile.id.clone(),
            local_revision: probe.local_revision,
            remote_revision: probe.remote_revision,
            freshness: match probe.freshness {
                swifty_repo::RepoFreshness::Unknown => RepoCheckFreshness::Unknown,
                swifty_repo::RepoFreshness::UpToDate => RepoCheckFreshness::UpToDate,
                swifty_repo::RepoFreshness::UpdateAvailable => RepoCheckFreshness::UpdateAvailable,
                swifty_repo::RepoFreshness::Error => RepoCheckFreshness::Error,
            },
            checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        },
        Err(_) => error_report(profile, None),
    }
}

fn error_report(profile: &Profile, local_revision: Option<String>) -> RepoCheckReport {
    RepoCheckReport {
        profile_id: profile.id.clone(),
        local_revision,
        remote_revision: None,
        freshness: RepoCheckFreshness::Error,
        checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
    }
}
