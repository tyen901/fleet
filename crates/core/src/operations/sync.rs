use crate::operations::progress;
use crate::operations::{local_files, OperationPublisher, OperationStage};
use fleet_domain::health::{RepoCheckFreshness, RepoCheckReport, SyncReport, VerificationKind};
use fleet_domain::{validated_repo_url, LocalFileHealth, Profile};
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub(crate) async fn sync(
    profile: &Profile,
    state_root: &Path,
    publisher: OperationPublisher,
    cancel: CancellationToken,
) -> Result<SyncReport, crate::ApiError> {
    publisher.stage(OperationStage::Validating);
    let dest = profile.dest_path().map_err(|error| {
        crate::ApiError::reported(
            "invalid_profile",
            "Choose a destination folder in profile settings.",
            error,
        )
    })?;
    let repo_url = validated_repo_url(&profile.source).map_err(|_| {
        crate::ApiError::new(
            "invalid_profile",
            "Enter a valid repository URL in profile settings.",
        )
    })?;
    let repo_cache = fleet_domain::repo_cache_dir(state_root, &profile.id);

    publisher.stage(OperationStage::LoadingExpectedState);
    let downloads = fleet_download::DownloadService::new_default();
    let (request, revision) = tokio::select! {
        result = fleet_flux::load_request(
            &dest,
            repo_url,
            &repo_cache,
            &downloads,
        ) => result.map_err(|error| crate::ApiError::reported("sync_failed", "Couldn't load the sync source.", error))?,
        () = cancel.cancelled() => return Err(crate::ApiError::new("canceled", "Canceled")),
    };
    publisher.stage(OperationStage::VerifyingInventory);
    let options = flux::Options {
        cancellation: cancel.clone(),
        ..Default::default()
    };
    let prepared = progress::scanning(&publisher, |bytes| {
        fleet_flux::prepare(request, cancel.clone(), bytes)
    })
    .await
    .map_err(|error| inventory_error(&cancel, error))?;
    let verification = if let Some(prepared) = prepared {
        let outcome = progress::observe(&publisher, prepared.materialize(options))
            .await
            .map_err(|error| inventory_error(&cancel, error))?;
        tracing::info!(
            operation = "run_success",
            deleted_entries = outcome.deleted_entries,
            written_bytes = outcome.written_bytes,
            fetched_bytes = outcome.fetched_bytes,
            reused_bytes = outcome.reused_bytes,
            kept_files = outcome.kept_files,
            "materialized profile"
        );
        VerificationKind::Materialized
    } else {
        VerificationKind::Fast
    };
    publisher.stage(OperationStage::Finalizing);
    Ok(SyncReport {
        profile_id: profile.id.clone(),
        repo: RepoCheckReport {
            profile_id: profile.id.clone(),
            local_revision: revision.clone(),
            remote_revision: revision,
            freshness: RepoCheckFreshness::UpToDate,
            checked_at_unix_ms: fleet_domain::time::now_unix_ms(),
        },
        local: local_files::report(profile, verification, LocalFileHealth::Clean),
    })
}

fn inventory_error(
    cancel: &CancellationToken,
    error: fleet_flux::InspectionError,
) -> crate::ApiError {
    if let fleet_flux::InspectionError::Operation(operation) = error {
        return sync_error(cancel, operation);
    }
    crate::ApiError::reported(
        "sync_failed",
        "Sync failed: Could not access the local file inventory.",
        error,
    )
}

fn sync_error(cancel: &CancellationToken, error: flux::Error) -> crate::ApiError {
    if cancel.is_cancelled() || matches!(error, flux::Error::Cancelled) {
        return crate::ApiError::new("canceled", "Canceled");
    }
    let message = match error {
        flux::Error::Source(_) => {
            "Sync failed: Could not download a repository file. Try Sync again."
        }
        flux::Error::InsufficientSpace { .. } => {
            "Sync failed: Free up space in the destination and try again."
        }
        _ => "Sync failed: Could not update local files. Check folder permissions and try again.",
    };
    crate::ApiError::reported("sync_failed", message, error)
}

#[cfg(test)]
mod tests {
    use super::sync_error;
    use object_store::{ClientOptions, ObjectStoreExt};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn http_error_body_is_logged_but_never_published_to_the_ui() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body =
            "<!doctype html><html><body>Cloudflare could not retrieve this object.</body></html>";
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            loop {
                line.clear();
                assert!(stream.read_line(&mut line).await.unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            stream
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
        });
        let store = object_store::http::HttpBuilder::new()
            .with_url(format!("http://{address}"))
            .with_client_options(ClientOptions::new().with_allow_http(true))
            .build()
            .unwrap();
        let error = store
            .get(&object_store::path::Path::from("addons/terrain.pbo"))
            .await
            .unwrap_err();
        server.await.unwrap();
        let error = flux::Error::Source(error.to_string());
        let log = tempfile::NamedTempFile::new().unwrap();
        let writer = log.reopen().unwrap();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.try_clone().unwrap())
            .finish();
        let published = tracing::subscriber::with_default(subscriber, || {
            sync_error(&CancellationToken::new(), error)
        });
        assert_eq!(published.code, "sync_failed");
        assert_eq!(
            published.message,
            "Sync failed: Could not download a repository file. Try Sync again."
        );
        let serialized = serde_json::to_string(&published).unwrap();
        assert!(!serialized.contains("html"));
        assert!(!serialized.contains(&address.to_string()));
        assert!(!serialized.contains("Generic HTTP error"));
        let diagnostic = std::fs::read_to_string(log.path()).unwrap();
        assert!(diagnostic.contains("400 Bad Request"));
        assert!(diagnostic.contains(body));
        assert!(diagnostic.contains("terrain.pbo"));
    }

    #[test]
    fn cancellation_is_distinct_from_source_and_local_failures() {
        for (error, code, is_download) in [
            (flux::Error::Cancelled, "canceled", false),
            (
                flux::Error::Source("private diagnostic".into()),
                "sync_failed",
                true,
            ),
            (
                flux::Error::Io(std::io::Error::other("private diagnostic")),
                "sync_failed",
                false,
            ),
        ] {
            let published = sync_error(&CancellationToken::new(), error);
            assert_eq!(published.code, code);
            assert_eq!(published.message.contains("download"), is_download);
            assert!(!published.message.contains("private diagnostic"));
        }
        let token = CancellationToken::new();
        token.cancel();
        assert_eq!(
            sync_error(&token, flux::Error::Source("in-flight HTTP failure".into())).code,
            "canceled"
        );
    }
}
