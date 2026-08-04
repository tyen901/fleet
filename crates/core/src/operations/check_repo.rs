use crate::operations::{OperationNoticeLevel, OperationPublisher, OperationStage};
use fleet_domain::health::{RepoCheckFreshness, RepoCheckReport};
use fleet_domain::{AppSettings, Profile, ProfileSourceKind};
use std::path::Path;

pub(crate) async fn check_repo(
    profile: &Profile,
    settings: &AppSettings,
    state_root: &Path,
    publisher: OperationPublisher,
) -> Result<RepoCheckReport, crate::ApiError> {
    publisher.stage(OperationStage::Validating);
    let repo_url = match profile.validated_source_kind() {
        Ok(ProfileSourceKind::Http(repo_url)) => repo_url,
        Err(_) => {
            publisher.notice(
                OperationNoticeLevel::Error,
                Some("invalid_profile".to_string()),
                "profile source is invalid".to_string(),
            );
            publisher.stage(OperationStage::Finalizing);
            return Ok(error_report(profile, None));
        }
    };
    publisher.stage(OperationStage::LoadingExpectedState);
    let repo_cache_dir = fleet_domain::repo_cache_dir(state_root, &profile.id);
    let store = swifty_repo::FsRepoCacheStore::new(repo_cache_dir.clone());
    let downloads = fleet_download::DownloadService::new_default();
    let local_revision = swifty_repo::load_cached_repo_blocking(&repo_cache_dir, repo_url)
        .ok()
        .flatten()
        .and_then(|cache| swifty_repo::repo_blob_revision(&cache));
    let resolver = swifty_repo::DefaultModSrfResolver;
    let report = match swifty_repo::sync_repo_metadata(
        repo_url, &store, &resolver, &downloads, None,
    )
    .await
    {
        Ok(sync) => {
            let remote_revision = swifty_repo::load_cached_repo_blocking(&repo_cache_dir, repo_url)
                .ok()
                .flatten()
                .and_then(|cache| swifty_repo::repo_blob_revision(&cache));
            Ok(RepoCheckReport {
                profile_id: profile.id.clone(),
                local_revision,
                remote_revision,
                freshness: match sync.freshness {
                    swifty_repo::RepoFreshness::Unknown => RepoCheckFreshness::Unknown,
                    swifty_repo::RepoFreshness::UpToDate => RepoCheckFreshness::UpToDate,
                    swifty_repo::RepoFreshness::UpdateAvailable => {
                        RepoCheckFreshness::UpdateAvailable
                    }
                },
                checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
            })
        }
        Err(_) => {
            let _ = settings;
            publisher.notice(
                OperationNoticeLevel::Warn,
                Some("repo_probe_failed".to_string()),
                "repo freshness probe failed".to_string(),
            );
            Ok(error_report(profile, local_revision))
        }
    }?;
    publisher.stage(OperationStage::Finalizing);
    Ok(report)
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
