use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use fleet_download::{DownloadEventSink, DownloadService};
use flux::{
    ManifestHeader, ManifestRecord, OpaqueSegmentIdentity, ProfileFingerprint, SegmentKey,
    TargetPath, ValidatedManifest, ValidationSpec,
};
use object_store::path::Path as ObjectPath;
use tracing::{debug, debug_span, error};
use uuid::Uuid;

const SWIFTY_PROFILE_BYTES: [u8; 32] = *b"fleet-swifty-md5-profile-v1.0000";

pub fn swifty_profile_fingerprint() -> ProfileFingerprint {
    ProfileFingerprint::new(SWIFTY_PROFILE_BYTES)
}

#[derive(Clone)]
pub struct MaterializationInput {
    pub manifest: ValidatedManifest,
    pub store_index: SwiftyStoreIndex,
    pub total_bytes: u64,
    pub file_count: usize,
}

#[derive(Clone, Default)]
pub struct SwiftyStoreIndex {
    pub objects: Vec<SwiftyStoreObject>,
}

#[derive(Clone)]
pub struct SwiftyStoreObject {
    pub target_path: TargetPath,
    pub source_url: String,
    pub object_path: ObjectPath,
    pub parts: Vec<SwiftyStorePart>,
}

#[derive(Clone)]
pub struct SwiftyStorePart {
    pub key: SegmentKey,
    pub validation: ValidationSpec,
    pub object_range: Range<u64>,
    pub target_range: Range<u64>,
}

pub async fn load_swifty_materialization_input(
    repo_url: &str,
    repo_cache_dir: &Path,
    downloads: &DownloadService,
    sink: Option<DownloadEventSink>,
) -> Result<MaterializationInput> {
    let span =
        debug_span!("sync.load_materialization_input_from_swifty_repo", repo_url = %repo_url);
    let _g = span.enter();

    let store = swifty_repo::FsRepoCacheStore::new(repo_cache_dir.to_path_buf());
    let resolver = swifty_repo::DefaultModSrfResolver;

    let started_at = Instant::now();
    let swifty_repo::RepoSyncResult { repo, mods, .. } =
        swifty_repo::sync_repo_metadata(repo_url, &store, &resolver, downloads, sink.clone())
            .await
            .with_context(|| format!("sync swifty repo metadata {repo_url}"))?;
    debug!(
        elapsed_ms = started_at.elapsed().as_millis(),
        mods = mods.len(),
        required = repo.required_mods.len(),
        optional = repo.optional_mods.len(),
        "synced swifty repo metadata"
    );

    swifty_repo_to_materialization_input(repo_url, &repo, &mods)
        .context("transform swifty repo -> flux materialization input")
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

    let mods = cache
        .mods
        .into_iter()
        .map(|(name, cached)| (name, cached.manifest))
        .collect::<BTreeMap<_, _>>();
    swifty_repo_to_materialization_input(repo_url, &cache.repo, &mods).map(Some)
}

pub fn swifty_repo_to_materialization_input(
    repo_url: &str,
    repo: &swifty_artifacts::RepoSpec,
    mods: &BTreeMap<String, swifty_artifacts::SrfMod>,
) -> Result<MaterializationInput> {
    let base_url = url::Url::parse(repo_url)
        .context("parse repo url")?
        .join("./")
        .context("resolve repo base url")?;
    let profile = swifty_profile_fingerprint();
    let mut records = vec![ManifestRecord::Header(ManifestHeader {
        manifest_id: Uuid::new_v4(),
        profile,
    })];
    let mut store_index = SwiftyStoreIndex::default();
    let mut total_bytes = 0_u64;
    let mut file_count = 0_usize;
    let mut files_seen = BTreeSet::<String>::new();

    let ordered_mod_names = repo
        .required_mods
        .iter()
        .chain(repo.optional_mods.iter())
        .map(|m| m.mod_name.to_ascii_lowercase())
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
            let target_path = TargetPath::new(&rel)?;
            let source_url = base_url
                .join(&rel)
                .with_context(|| format!("join mod file url {rel}"))?
                .to_string();
            records.push(ManifestRecord::File {
                path: target_path.clone(),
                len: file.length,
            });

            let mut parts = Vec::new();
            for part in file.parts.iter().filter(|part| part.length > 0) {
                let key = swifty_segment_key(part.checksum.as_bytes(), part.length)?;
                let validation = ValidationSpec {
                    profile,
                    key: key.clone(),
                    len: part.length,
                };
                let target_range = part.start..part.start + part.length;
                records.push(ManifestRecord::Segment {
                    path: target_path.clone(),
                    range: target_range.clone(),
                    key: key.clone(),
                    validation: validation.clone(),
                });
                parts.push(SwiftyStorePart {
                    key,
                    validation,
                    object_range: part.start..part.start + part.length,
                    target_range,
                });
            }
            total_bytes = total_bytes.saturating_add(file.length);
            file_count += 1;
            store_index.objects.push(SwiftyStoreObject {
                target_path,
                source_url,
                object_path: ObjectPath::parse(&rel)?,
                parts,
            });
        }
    }

    let manifest = flux::validate_manifest(records, profile)?;
    Ok(MaterializationInput {
        manifest,
        store_index,
        total_bytes,
        file_count,
    })
}

pub fn expected_file_paths(input: &MaterializationInput) -> BTreeSet<String> {
    input
        .manifest
        .files
        .iter()
        .map(|file| file.path.as_str().to_string())
        .collect()
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
    } else if file.parts.iter().any(|p| p.length > 0) {
        anyhow::bail!(
            "invalid swifty parts for empty file: mod={} path={}",
            mod_m.name,
            file.path
        );
    }
    Ok(())
}

fn swifty_segment_key(raw_md5: &[u8; 16], len: u64) -> Result<SegmentKey> {
    Ok(SegmentKey::new(
        swifty_profile_fingerprint(),
        OpaqueSegmentIdentity::new(raw_md5.to_vec())?,
        len,
    )?)
}

fn normalize_swifty_file_rel_path(mod_name: &str, raw_path: &str) -> Result<String> {
    let normalized = fleet_domain::normalize_rel_slashes(raw_path);
    if normalized.starts_with('/') {
        anyhow::bail!(
            "invalid swifty mod.srf file path (must be relative): mod={} path={}",
            mod_name,
            raw_path
        );
    }
    if normalized.len() >= 3 {
        let bytes = normalized.as_bytes();
        if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/' {
            anyhow::bail!(
                "invalid swifty mod.srf file path (must be relative): mod={} path={}",
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
                "invalid swifty mod.srf file path (parent traversal not allowed): mod={} path={}",
                mod_name,
                raw_path
            );
        }
        parts.push(part);
    }
    if parts.is_empty() {
        anyhow::bail!(
            "invalid swifty mod.srf file path (must not be empty): mod={} path={}",
            mod_name,
            raw_path
        );
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::swifty_repo_to_materialization_input;

    #[test]
    fn swifty_conversion_produces_manifest_and_store_sidecar() {
        let part = swifty_part("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "addons/a.pbo_5", 0, 5);
        let checksum = swifty_artifacts::file_md5_from_parts(std::slice::from_ref(&part));
        let repo = repo_spec("@mod");
        let mods = BTreeMap::from([(
            "@mod".to_string(),
            swifty_artifacts::SrfMod {
                name: "@mod".to_string(),
                checksum: swifty_artifacts::Md5Digest::default(),
                files: vec![swifty_artifacts::SrfFile {
                    path: "addons\\a.pbo".to_string(),
                    length: 5,
                    checksum,
                    r#type: None,
                    parts: vec![part],
                }],
            },
        )]);

        let input =
            swifty_repo_to_materialization_input("https://example.com/repo.json", &repo, &mods)
                .expect("convert");

        assert_eq!(input.file_count, 1);
        assert_eq!(input.total_bytes, 5);
        assert_eq!(input.manifest.files[0].path.as_str(), "@mod/addons/a.pbo");
        assert_eq!(input.manifest.files[0].segments.len(), 1);
        assert_eq!(input.store_index.objects.len(), 1);
        let object = &input.store_index.objects[0];
        assert_eq!(object.target_path.as_str(), "@mod/addons/a.pbo");
        assert_eq!(object.object_path.as_ref(), "@mod/addons/a.pbo");
        assert_eq!(object.source_url, "https://example.com/@mod/addons/a.pbo");
        assert_eq!(object.parts[0].object_range, 0..5);
        assert_eq!(
            object.parts[0].key.identity.bytes(),
            swifty_artifacts::Md5Digest::parse_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .expect("md5")
                .as_bytes()
        );
    }

    fn repo_spec(mod_name: &str) -> swifty_artifacts::RepoSpec {
        swifty_artifacts::RepoSpec {
            repo_name: "test".to_string(),
            checksum: "deadbeef".to_string(),
            required_mods: vec![swifty_artifacts::RepoMod {
                mod_name: mod_name.to_string(),
                checksum: swifty_artifacts::Md5Digest::default(),
                enabled: true,
            }],
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
