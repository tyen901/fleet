use crate::remote::{RemoteCapabilities, RemoteRepo};
use crate::safe_path::{validate_mod_id, validate_rel_path};
use anyhow::{Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone, Debug)]
pub struct ModManifest {
    pub mod_id: String,
    pub files: Vec<FileEntry>,
}

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub rel_path: String,
    pub size: u64,
    pub file_checksum: Vec<u8>,
    pub parts: Vec<FilePart>,
}

#[derive(Clone, Debug)]
pub struct FilePart {
    pub offset: u64,
    pub len: u64,
    pub checksum: Vec<u8>,
}

pub struct FetchResult {
    pub capabilities: RemoteCapabilities,
    pub manifests: Vec<ModManifest>,
}

pub async fn fetch_all(
    remote: Arc<dyn RemoteRepo>,
    enabled_mods: &[String],
    max_concurrency: usize,
) -> Result<FetchResult> {
    for mod_id in enabled_mods {
        validate_mod_id(mod_id).with_context(|| format!("invalid mod_id {mod_id}"))?;
    }

    let caps = remote.capabilities().await.unwrap_or_default();

    let sem = Arc::new(Semaphore::new(max_concurrency.max(1)));
    let mut tasks = FuturesUnordered::new();

    for mod_id in enabled_mods {
        let remote = remote.clone();
        let permit = sem.clone().acquire_owned().await?;
        let mod_id = mod_id.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = permit;
            let mut manifest = remote
                .fetch_mod_manifest(&mod_id)
                .await
                .with_context(|| format!("fetch manifest for {mod_id}"))?;
            if manifest.mod_id != mod_id {
                anyhow::bail!(
                    "manifest mod_id mismatch (requested {}, got {})",
                    mod_id,
                    manifest.mod_id
                );
            }
            normalize_manifest(&mut manifest);
            validate_manifest(&manifest)?;
            Ok::<ModManifest, anyhow::Error>(manifest)
        }));
    }

    let mut manifests = Vec::new();
    while let Some(res) = tasks.next().await {
        manifests.push(res??);
    }

    manifests.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));

    Ok(FetchResult {
        capabilities: caps,
        manifests,
    })
}

fn validate_manifest(manifest: &ModManifest) -> Result<()> {
    validate_mod_id(&manifest.mod_id)?;
    for file in &manifest.files {
        validate_rel_path(&file.rel_path)
            .with_context(|| format!("invalid rel_path {}", file.rel_path))?;
    }
    Ok(())
}

fn normalize_manifest(manifest: &mut ModManifest) {
    for file in &mut manifest.files {
        file.rel_path = file.rel_path.replace('\\', "/");
    }
}
