use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use fleet_download::DownloadService;
use flux::{ContentKey, FileSpec, Manifest, ProfileId, Segment, TargetPath};
use object_store::path::Path as ObjectPath;
use tracing::{debug, debug_span, error};

const SWIFTY_PROFILE_BYTES: [u8; 32] = *b"fleet-swifty-md5-profile-v1.0000";

pub fn swifty_profile_id() -> ProfileId {
    ProfileId(SWIFTY_PROFILE_BYTES)
}

pub struct MaterializationInput {
    pub(crate) manifest: Manifest,
    pub(crate) store_index: SwiftyStoreIndex,
    pub(crate) revision: Option<String>,
}

impl MaterializationInput {
    pub fn revision(&self) -> Option<&str> {
        self.revision.as_deref()
    }
}

pub(crate) struct SwiftyStoreIndex {
    pub(crate) base_url: String,
    pub(crate) objects: Vec<SwiftyStoreObject>,
}

pub(crate) struct SwiftyStoreObject {
    pub(crate) object_path: ObjectPath,
    pub(crate) parts: Vec<SwiftyStorePart>,
}

pub(crate) struct SwiftyStorePart {
    pub(crate) key: ContentKey,
    pub(crate) offset: u64,
}

pub async fn load_swifty_materialization_input(
    repo_url: &str,
    repo_cache_dir: &Path,
    downloads: &DownloadService,
) -> Result<MaterializationInput> {
    let span =
        debug_span!("sync.load_materialization_input_from_swifty_repo", repo_url = %repo_url);
    let _guard = span.enter();
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
    let mut input = swifty_repo_to_materialization_input(repo_url, &repo, &mods)
        .context("transform swifty repo -> flux materialization input")?;
    input.revision = revision;
    Ok(input)
}

pub fn load_cached_swifty_materialization_input(
    repo_url: &str,
    repo_cache_dir: &Path,
) -> Result<Option<MaterializationInput>> {
    let Some(cache) = swifty_repo::load_cached_repo_blocking(repo_cache_dir, repo_url)
        .with_context(|| format!("load swifty cache for {repo_url}"))?
    else {
        return Ok(None);
    };
    let revision = swifty_repo::repo_blob_revision(&cache);
    let mods = cache
        .mods
        .into_iter()
        .map(|(name, cached)| (name, cached.manifest))
        .collect::<BTreeMap<_, _>>();
    let mut input = swifty_repo_to_materialization_input(repo_url, &cache.repo, &mods)?;
    input.revision = revision;
    Ok(Some(input))
}

pub fn swifty_repo_to_materialization_input(
    repo_url: &str,
    repo: &swifty_artifacts::RepoSpec,
    mods: &BTreeMap<String, swifty_artifacts::SrfMod>,
) -> Result<MaterializationInput> {
    let base_url = url::Url::parse(repo_url)?
        .join("./")
        .context("resolve repo base url")?;
    let profile = swifty_profile_id();
    let mut files = Vec::new();
    let mut store_index = SwiftyStoreIndex {
        base_url: base_url.to_string(),
        objects: Vec::new(),
    };
    let mut files_seen = BTreeSet::<String>::new();
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
            if !files_seen.insert(rel.clone()) {
                anyhow::bail!("duplicate target path in Swifty metadata: {rel}");
            }
            let target_path = TargetPath::new(rel.clone())?;
            let mut segments = Vec::new();
            let mut parts = Vec::new();
            for part in file.parts.iter().filter(|part| part.length > 0) {
                let key = swifty_segment_key(part.checksum.as_bytes(), part.length)?;
                segments.push(Segment {
                    offset: part.start,
                    key: key.clone(),
                });
                parts.push(SwiftyStorePart {
                    key,
                    offset: part.start,
                });
            }
            files.push(FileSpec {
                path: target_path,
                length: file.length,
                segments,
            });
            store_index.objects.push(SwiftyStoreObject {
                object_path: ObjectPath::parse(&rel)?,
                parts,
            });
        }
    }
    let manifest = Manifest::new(profile, files, Vec::new())?;
    Ok(MaterializationInput {
        manifest,
        store_index,
        revision: None,
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

fn swifty_segment_key(raw_md5: &[u8; 16], length: u64) -> Result<ContentKey> {
    Ok(ContentKey::new(
        swifty_profile_id(),
        raw_md5.to_vec(),
        length,
    )?)
}

fn normalize_swifty_file_rel_path(mod_name: &str, raw_path: &str) -> Result<String> {
    let normalized = raw_path.replace('\\', "/");
    if normalized.starts_with('/') {
        anyhow::bail!(
            "invalid swifty mod.srf file path: mod={} path={}",
            mod_name,
            raw_path
        );
    }
    if normalized.len() >= 3 {
        let bytes = normalized.as_bytes();
        if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/' {
            anyhow::bail!(
                "invalid swifty mod.srf file path: mod={} path={}",
                mod_name,
                raw_path
            );
        }
    }
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            anyhow::bail!(
                "invalid swifty mod.srf file path traversal: mod={} path={}",
                mod_name,
                raw_path
            );
        }
        parts.push(part);
    }
    if parts.is_empty() {
        anyhow::bail!(
            "invalid swifty mod.srf file path is empty: mod={} path={}",
            mod_name,
            raw_path
        );
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::swifty_repo_to_materialization_input;
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
        let input =
            swifty_repo_to_materialization_input("https://example.com/repo.json", &repo, &mods)
                .expect("convert");
        assert_eq!(input.manifest.files().len(), 1);
        assert_eq!(
            input.manifest.files()[0].path.as_str(),
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
