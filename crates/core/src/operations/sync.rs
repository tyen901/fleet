use crate::operations::progress::progress_channel;
use crate::operations::{local_files, OperationPublisher, OperationStage};
use fleet_domain::health::{RepoCheckFreshness, RepoCheckReport, SyncReport, VerificationKind};
use fleet_domain::{validated_repo_url, LocalFileHealth, Profile};
use fleet_inventory::FleetInventory;
use std::path::Path;
use std::sync::Arc;
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
    let input = tokio::select! {
        result = fleet_flux::load_swifty_materialization_input(
            repo_url,
            &repo_cache,
            &downloads,
        ) => result.map_err(|error| crate::ApiError::reported("sync_failed", "Could not download repository data. Try Sync again.", error))?,
        () = cancel.cancelled() => return Err(crate::ApiError::new("canceled", "Canceled")),
    };
    let revision = input.revision().map(ToOwned::to_owned);
    std::fs::create_dir_all(&dest).map_err(|error| {
        crate::ApiError::reported(
            "sync_failed",
            "Could not create the destination folder. Check folder permissions.",
            error,
        )
    })?;
    let inventory = Arc::new(
        FleetInventory::open(&dest, fleet_flux::swifty_profile_id()).map_err(|error| {
            crate::ApiError::reported(
                "inventory",
                "Could not prepare the local file inventory. Check folder permissions.",
                error,
            )
        })?,
    );
    let catalog = inventory.clone();
    let manifest = input.manifest().clone();
    tokio::task::spawn_blocking(move || catalog.register_manifest(&manifest))
        .await
        .map_err(|error| {
            crate::ApiError::reported(
                "inventory",
                "Could not prepare the local file inventory. Check folder permissions.",
                error,
            )
        })?
        .map_err(|error| {
            crate::ApiError::reported(
                "inventory",
                "Could not prepare the local file inventory. Check folder permissions.",
                error,
            )
        })?;

    publisher.stage(OperationStage::VerifyingInventory);
    let (observers, progress_receiver) = progress_channel(publisher.clone());
    let materialization =
        fleet_flux::materialize(&dest, inventory, input, cancel.clone(), observers);
    progress_receiver
        .observe(publisher.clone(), materialization)
        .await
        .map_err(|error| sync_error(&cancel, error))?;
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
        local: local_files::report(
            profile,
            VerificationKind::Materialized,
            LocalFileHealth::Clean,
        ),
    })
}

fn sync_error(cancel: &CancellationToken, error: anyhow::Error) -> crate::ApiError {
    if cancel.is_cancelled() || fleet_flux::is_cancellation(&error) {
        crate::ApiError::new("canceled", "Canceled")
    } else {
        let message = if fleet_flux::is_source_error(&error) {
            "Sync failed: Could not download a repository file. Try Sync again."
        } else {
            "Sync failed: Could not update local files. Check folder permissions and try again."
        };
        crate::ApiError::reported("sync_failed", message, error)
    }
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
        let error = anyhow::Error::new(flux::Error::with_source(flux::ErrorKind::Source, error))
            .context("materialize profile");
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
        for kind in [
            flux::ErrorKind::Cancelled,
            flux::ErrorKind::Source,
            flux::ErrorKind::Io,
        ] {
            let token = CancellationToken::new();
            let error = anyhow::Error::new(flux::Error::new(kind, "private diagnostic"));
            let published = sync_error(&token, error);
            if kind == flux::ErrorKind::Cancelled {
                assert_eq!(published.code, "canceled");
            } else {
                assert_eq!(published.code, "sync_failed");
                assert!(!published.message.contains("private diagnostic"));
                assert_eq!(
                    published.message.contains("download"),
                    kind == flux::ErrorKind::Source
                );
            }
        }
        let token = CancellationToken::new();
        token.cancel();
        assert_eq!(
            sync_error(&token, anyhow::anyhow!("in-flight HTTP failure")).code,
            "canceled"
        );
    }
}
