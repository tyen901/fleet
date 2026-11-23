use std::path::Path;
use std::time::Instant;
use std::{collections::BTreeMap, sync::Arc};

use crate::{
    profile::{SwiftyVerifier, PROFILE_ID},
    source::Repository,
};
use anyhow::{Context, Result};
use fleet_download::DownloadService;
use flux::{Goal, Key, ProtectedNames, Range, Recipe, Request, Source};
use object_store::path::Path as ObjectPath;
use relative_path::{Component, RelativePath};
use tracing::{debug, error};

pub(super) const FLEET_DIRECTORY: &str = ".fleet";

pub async fn load_request(
    target: &Path,
    repo_url: &str,
    repo_cache_dir: &Path,
    downloads: &DownloadService,
) -> Result<(Request, Option<String>)> {
    let store = swifty_repo::FsRepoCacheStore::new(repo_cache_dir.to_path_buf());
    let started_at = Instant::now();
    let swifty_repo::RepoSyncResult {
        repo,
        mods,
        revision,
        ..
    } = swifty_repo::sync_repo_metadata(repo_url, &store, downloads)
        .await
        .with_context(|| format!("sync swifty repo metadata {repo_url}"))?;
    debug!(
        elapsed_ms = started_at.elapsed().as_millis(),
        mods = mods.len(),
        required = repo.required_mods.len(),
        optional = repo.optional_mods.len(),
        "synced swifty repo metadata"
    );
    let target = target.to_owned();
    let repo_url = repo_url.to_owned();
    let request = tokio::task::spawn_blocking(move || {
        request(&target, &repo, &mods, Some(Repository::new(&repo_url)?))
    })
    .await?
    .context("convert Swifty manifest to a Flux request")?;
    Ok((request, revision))
}

pub async fn cached_request(
    target: &Path,
    repo_url: &str,
    repo_cache_dir: &Path,
) -> Result<Option<Request>> {
    let target = target.to_owned();
    let repo_url = repo_url.to_owned();
    let repo_cache_dir = repo_cache_dir.to_owned();
    tokio::task::spawn_blocking(move || {
        let Some(cache) = swifty_repo::load_cached_repo_blocking(&repo_cache_dir, &repo_url)
            .with_context(|| format!("load swifty cache for {repo_url}"))?
        else {
            return Ok(None);
        };
        let mods = cache
            .mods
            .into_iter()
            .map(|(name, cached)| (name, cached.manifest))
            .collect::<BTreeMap<_, _>>();
        Ok(Some(request(&target, &cache.repo, &mods, None)?))
    })
    .await?
}

fn request(
    target: &Path,
    repo: &swifty_artifacts::RepoSpec,
    mods: &BTreeMap<String, swifty_artifacts::SrfMod>,
    mut repository: Option<Repository>,
) -> Result<Request> {
    let mut files = Vec::new();
    let ordered_mod_names = repo
        .required_mods
        .iter()
        .chain(repo.optional_mods.iter())
        .filter(|mod_m| mod_m.enabled)
        .map(|mod_m| mod_m.mod_name.to_ascii_lowercase())
        .collect::<Vec<_>>();

    for name in ordered_mod_names {
        let Some(mod_m) = mods.get(&name) else {
            error!(mod_name = name.as_str(), "missing mod manifest in cache");
            anyhow::bail!("missing mod manifest for {name}");
        };
        for file in &mod_m.files {
            validate_swifty_file(mod_m, file)?;
            let file_rel = normalize_swifty_file_rel_path(&mod_m.name, &file.path)?;
            let rel = format!("{}/{}", mod_m.name, file_rel);
            let object = ObjectPath::parse(&rel)?;
            let mut keys = Vec::with_capacity(file.parts.len());
            for part in file.parts.iter().filter(|part| part.length > 0) {
                let key = Key::new(part.checksum.as_bytes().to_vec(), part.length)?;
                if let Some(source) = &mut repository {
                    source.insert(
                        key.clone(),
                        Range {
                            object: object.clone(),
                            offset: part.start,
                            etag: None,
                        },
                    );
                }
                keys.push(key);
            }
            files.push((RelativePath::new(&rel).into(), Recipe::new(keys)?));
        }
    }
    let goal = Goal::new(PROFILE_ID, files, vec![])?
        .with_protected_names(ProtectedNames::new([FLEET_DIRECTORY])?)?;
    Ok(Request {
        target: target.to_owned(),
        goal,
        verifier: Arc::new(SwiftyVerifier),
        local: Default::default(),
        sources: repository
            .into_iter()
            .map(|s| Arc::new(s) as Arc<dyn Source>)
            .collect(),
    })
}

fn validate_swifty_file(
    mod_m: &swifty_artifacts::SrfMod,
    file: &swifty_artifacts::SrfFile,
) -> Result<()> {
    if file.length > 0 {
        swifty_artifacts::validate_parts_swifty_strict(&file.path, file.length, &file.parts)
            .with_context(|| {
                format!(
                    "invalid swifty parts: mod={} path={}",
                    mod_m.name, file.path
                )
            })?;
        let derived = swifty_artifacts::file_md5_from_parts(&file.parts);
        if derived.as_bytes() != file.checksum.as_bytes() {
            anyhow::bail!(
                "invalid swifty file checksum: mod={} path={} expected={} derived={}",
                mod_m.name,
                file.path,
                file.checksum.to_hex_upper(),
                derived.to_hex_upper()
            );
        }
    } else if file.parts.iter().any(|part| part.length > 0) {
        anyhow::bail!(
            "invalid swifty parts for empty file: mod={} path={}",
            mod_m.name,
            file.path
        );
    }
    Ok(())
}

fn normalize_swifty_file_rel_path(mod_name: &str, raw_path: &str) -> Result<String> {
    let normalized = raw_path.replace('\\', "/");
    let path = RelativePath::new(&normalized);
    anyhow::ensure!(
        !normalized.starts_with('/')
            && path.file_name().is_some()
            && !path.components().any(|c| c == Component::ParentDir),
        "invalid Swifty file path in {mod_name}: {raw_path}"
    );
    Ok(path.normalize().into_string())
}

#[cfg(test)]
mod tests {
    use super::request;
    use std::collections::BTreeMap;

    #[test]
    fn conversion_materializes_only_enabled_mod_roots() {
        let part = swifty_part("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "addons/a.pbo_5", 0, 5);
        let checksum = swifty_artifacts::file_md5_from_parts(std::slice::from_ref(&part));
        let repo = swifty_artifacts::RepoSpec {
            repo_name: "test".to_string(),
            checksum: "deadbeef".to_string(),
            required_mods: vec![repo_mod("@enabled", true), repo_mod("@disabled", false)],
            optional_mods: vec![],
            icon_image_path: None,
            icon_image_checksum: None,
            repo_image_path: None,
            repo_image_checksum: None,
            required_dlcs: vec![],
            client_parameters: String::new(),
            repo_basic_authentication: None,
            version: String::new(),
            servers: vec![],
        };
        let mods = BTreeMap::from([
            ("@enabled".to_string(), mod_manifest("@enabled", checksum)),
            ("@disabled".to_string(), mod_manifest("@disabled", checksum)),
        ]);
        let input = request(std::path::Path::new("target"), &repo, &mods, None).expect("convert");
        assert_eq!(input.goal.files().len(), 1);
        assert_eq!(
            input.goal.files().first_key_value().unwrap().0.as_str(),
            "@enabled/addons/a.pbo_5"
        );
    }

    fn repo_mod(name: &str, enabled: bool) -> swifty_artifacts::RepoMod {
        swifty_artifacts::RepoMod {
            mod_name: name.to_string(),
            checksum: swifty_artifacts::Md5Digest::default(),
            enabled,
        }
    }

    fn mod_manifest(name: &str, checksum: swifty_artifacts::Md5Digest) -> swifty_artifacts::SrfMod {
        swifty_artifacts::SrfMod {
            name: name.to_string(),
            checksum: swifty_artifacts::Md5Digest::default(),
            files: vec![swifty_artifacts::SrfFile {
                path: "addons/a.pbo_5".to_string(),
                length: 5,
                checksum,
                r#type: None,
                parts: vec![swifty_part(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "addons/a.pbo_5",
                    0,
                    5,
                )],
            }],
        }
    }

    fn swifty_part(
        checksum: &str,
        path: &str,
        start: u64,
        length: u64,
    ) -> swifty_artifacts::SrfPart {
        swifty_artifacts::SrfPart {
            path: path.to_string(),
            start,
            length,
            checksum: swifty_artifacts::Md5Digest::parse_hex(checksum).expect("md5"),
        }
    }
}
