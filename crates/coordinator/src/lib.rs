pub mod events;

use camino::Utf8Path;
use futures_util::future::{BoxFuture, FutureExt};
use futures_util::stream::{FuturesUnordered, StreamExt};
use manifest_types::{ModManifest, RepoMod};
use remote_core::{RemoteRepo, RemoteSession};
use remote_http::HttpRemoteRepo;
use std::collections::{HashMap, HashSet};
use tokio::sync::{mpsc, Semaphore};
use walkdir::WalkDir;

#[derive(thiserror::Error, Debug)]
pub enum CoordinatorError {
    #[error("remote error: {0}")]
    Remote(#[from] remote_core::RemoteError),
    #[error("apply error: {0}")]
    Apply(#[from] sync_apply::ApplyError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("url error: {0}")]
    Url(String),
}

pub struct SyncOptions {
    pub apply: sync_apply::ApplyOptions,
    pub max_concurrent_manifest_fetches: usize,
}

impl Default for SyncOptions {
    fn default() -> Self {
        let par = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);
        Self {
            apply: sync_apply::ApplyOptions::default(),
            max_concurrent_manifest_fetches: par * 2,
        }
    }
}

pub async fn sync_checkout(
    repo_base_url: &str,
    checkout_root: &Utf8Path,
    opts: SyncOptions,
) -> Result<(), CoordinatorError> {
    sync_checkout_with_events(repo_base_url, checkout_root, opts, None).await
}

pub async fn sync_checkout_with_events(
    repo_base_url: &str,
    checkout_root: &Utf8Path,
    opts: SyncOptions,
    tx: Option<mpsc::Sender<crate::events::Event>>,
) -> Result<(), CoordinatorError> {
    let repo =
        HttpRemoteRepo::new(repo_base_url).map_err(|e| CoordinatorError::Url(format!("{e}")))?;
    let session = repo.open_session().await?;

    if let Some(tx) = &tx {
        let _ = tx.send(crate::events::Event::Started).await;
        let spec = session.repo_spec();
        let _ = tx
            .send(crate::events::Event::RepoFetched {
                repo_name: spec.repo_name.clone(),
                version: spec.version.clone(),
            })
            .await;
    }

    let required: Vec<RepoMod> = session
        .repo_spec()
        .required_mods
        .iter()
        .filter(|m| m.enabled)
        .cloned()
        .collect();

    let sem = std::sync::Arc::new(Semaphore::new(opts.max_concurrent_manifest_fetches));
    let session_ref = &session;
    let mut tasks: FuturesUnordered<
        BoxFuture<'_, Result<(String, ModManifest), remote_core::RemoteError>>,
    > = FuturesUnordered::new();

    for m in required {
        let sem = sem.clone();
        let mod_name = m.mod_name.clone();
        let tx = tx.clone();

        let fut = async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            if let Some(tx) = &tx {
                let _ = tx
                    .send(crate::events::Event::ModChecking {
                        mod_name: mod_name.clone(),
                    })
                    .await;
            }
            let manifest = session_ref.fetch_mod_manifest(&mod_name).await?;
            Ok::<(String, ModManifest), remote_core::RemoteError>((mod_name, manifest))
        };

        tasks.push(fut.boxed());
    }

    let mut manifests: HashMap<String, ModManifest> = HashMap::new();
    while let Some(res) = tasks.next().await {
        let (name, manifest) = res?;
        manifests.insert(name, manifest);
    }

    let mut plan = sync_plan::SyncPlan::default();
    for manifest in manifests.values() {
        plan.extend_mod_full(manifest);
        let delete_ops = build_delete_ops_for_mod(checkout_root, manifest)?;
        let delete_count = delete_ops.len();
        for op in delete_ops {
            plan.ops.push(op);
        }

        if let Some(tx) = &tx {
            let _ = tx
                .send(crate::events::Event::ModPlanned {
                    mod_name: manifest.name.clone(),
                    downloads: manifest.files.len(),
                    deletes: delete_count,
                })
                .await;
        }
    }

    let observer = tx.as_ref().map(|tx| ApplyEventAdapter { tx: tx.clone() });
    sync_apply::apply_plan_observed(
        &session,
        checkout_root,
        &plan,
        opts.apply,
        observer
            .as_ref()
            .map(|o| o as &dyn sync_apply::ApplyObserver),
    )
    .await?;

    if let Some(tx) = &tx {
        for manifest in manifests.values() {
            let _ = tx
                .send(crate::events::Event::ModApplied {
                    mod_name: manifest.name.clone(),
                })
                .await;
            let _ = tx
                .send(crate::events::Event::ModFinished {
                    mod_name: manifest.name.clone(),
                    checksum: manifest.checksum,
                })
                .await;
        }
        let _ = tx.send(crate::events::Event::Finished).await;
    }
    Ok(())
}

fn build_delete_ops_for_mod(
    checkout_root: &Utf8Path,
    manifest: &manifest_types::ModManifest,
) -> Result<Vec<sync_plan::Op>, std::io::Error> {
    let mod_root = checkout_root.join(&manifest.name);
    if !mod_root.exists() {
        return Ok(vec![]);
    }

    let expected: HashSet<String> = manifest
        .files
        .iter()
        .map(|f| f.path.as_str().replace('\\', "/"))
        .collect();

    let mut ops = Vec::new();
    for entry in WalkDir::new(mod_root.as_std_path())
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let abs = entry.path();
        let rel_os = abs.strip_prefix(mod_root.as_std_path()).unwrap();
        let rel = rel_os.to_string_lossy().replace('\\', "/");

        let file_name = entry.file_name().to_string_lossy();
        if rel.starts_with(".fleet/")
            || file_name.starts_with(".fleet_tmp_")
            || file_name.starts_with(".fleet_stage_")
        {
            continue;
        }

        if !expected.contains(&rel) {
            ops.push(sync_plan::Op::DeleteFile {
                mod_name: manifest.name.clone(),
                rel_path: relative_path::RelativePathBuf::from(rel),
            });
        }
    }

    Ok(ops)
}

struct ApplyEventAdapter {
    tx: tokio::sync::mpsc::Sender<crate::events::Event>,
}

impl sync_apply::ApplyObserver for ApplyEventAdapter {
    fn on_event(&self, ev: sync_apply::ApplyEvent) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            use crate::events::Event;
            match ev {
                sync_apply::ApplyEvent::TransferPlanned { total_bytes } => {
                    let _ = tx.send(Event::TransferPlanned { total_bytes }).await;
                }
                sync_apply::ApplyEvent::TransferProgress {
                    transferred_bytes,
                    total_bytes,
                } => {
                    let _ = tx
                        .send(Event::TransferProgress {
                            transferred_bytes,
                            total_bytes,
                        })
                        .await;
                }
                sync_apply::ApplyEvent::FileStarted {
                    mod_name,
                    rel_path,
                    total_bytes,
                    resume_from,
                } => {
                    let _ = tx
                        .send(Event::FileStarted {
                            mod_name,
                            rel_path,
                            total_bytes,
                            resume_from,
                        })
                        .await;
                }
                sync_apply::ApplyEvent::FileProgress {
                    mod_name,
                    rel_path,
                    downloaded_bytes,
                    total_bytes,
                } => {
                    let _ = tx
                        .send(Event::FileProgress {
                            mod_name,
                            rel_path,
                            downloaded_bytes,
                            total_bytes,
                        })
                        .await;
                }
                sync_apply::ApplyEvent::FileVerified {
                    mod_name,
                    rel_path,
                    checksum: _,
                } => {
                    let _ = tx.send(Event::FileVerified { mod_name, rel_path }).await;
                }
                sync_apply::ApplyEvent::FileDeleted { mod_name, rel_path } => {
                    let _ = tx.send(Event::FileDeleted { mod_name, rel_path }).await;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn apply_event_adapter_maps_all_events() {
        use sync_apply::ApplyObserver;

        let (tx, mut rx) = mpsc::channel(8);
        let adapter = ApplyEventAdapter { tx };

        adapter.on_event(sync_apply::ApplyEvent::FileStarted {
            mod_name: "mod".to_string(),
            rel_path: relative_path::RelativePathBuf::from("a.bin"),
            total_bytes: 10,
            resume_from: 0,
        });
        match rx.recv().await {
            Some(events::Event::FileStarted { .. }) => {}
            other => panic!("expected FileStarted, got {other:?}"),
        }

        adapter.on_event(sync_apply::ApplyEvent::FileProgress {
            mod_name: "mod".to_string(),
            rel_path: relative_path::RelativePathBuf::from("a.bin"),
            downloaded_bytes: 5,
            total_bytes: 10,
        });
        match rx.recv().await {
            Some(events::Event::FileProgress { .. }) => {}
            other => panic!("expected FileProgress, got {other:?}"),
        }

        adapter.on_event(sync_apply::ApplyEvent::FileVerified {
            mod_name: "mod".to_string(),
            rel_path: relative_path::RelativePathBuf::from("a.bin"),
            checksum: manifest_types::Md5Digest::default(),
        });
        match rx.recv().await {
            Some(events::Event::FileVerified { .. }) => {}
            other => panic!("expected FileVerified, got {other:?}"),
        }

        adapter.on_event(sync_apply::ApplyEvent::FileDeleted {
            mod_name: "mod".to_string(),
            rel_path: relative_path::RelativePathBuf::from("a.bin"),
        });
        match rx.recv().await {
            Some(events::Event::FileDeleted { .. }) => {}
            other => panic!("expected FileDeleted, got {other:?}"),
        }
    }

    #[test]
    fn build_delete_ops_finds_stale_files() {
        let temp = tempfile::tempdir().unwrap();
        let checkout = Utf8Path::from_path(temp.path()).unwrap();

        let mod_root = checkout.join("@mod");
        std::fs::create_dir_all(mod_root.as_std_path()).unwrap();
        let stale = mod_root.join("old.bin");
        std::fs::write(stale.as_std_path(), b"stale").unwrap();

        let manifest = manifest_types::ModManifest {
            name: "@mod".to_string(),
            checksum: manifest_types::Md5Digest::default(),
            files: vec![manifest_types::FileManifest {
                path: relative_path::RelativePathBuf::from("keep.bin"),
                length: 0,
                checksum: manifest_types::Md5Digest::default(),
                parts: vec![],
            }],
        };

        let ops = build_delete_ops_for_mod(checkout, &manifest).unwrap();
        assert!(ops
            .iter()
            .any(|op| matches!(op, sync_plan::Op::DeleteFile { .. })));
    }
}
