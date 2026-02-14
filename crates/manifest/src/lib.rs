use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use fleet_download::{DownloadEventSink, DownloadService};
use flux_manifest::{ManifestEntry, ManifestVersion};
use flux_types::{SegmentSpec, Signature, SourceRef};
use tracing::{debug, debug_span, error};

pub use flux_manifest::DesiredManifest;

pub struct ManifestStats {
    pub total_download_bytes: u64,
}

pub fn manifest_stats(m: &DesiredManifest) -> ManifestStats {
    let total_download_bytes = m
        .entries
        .iter()
        .filter_map(|entry| match entry {
            ManifestEntry::File(f) => Some(f.size_bytes),
            _ => None,
        })
        .sum::<u64>();

    ManifestStats {
        total_download_bytes,
    }
}

pub async fn load_desired_manifest(
    repo_url: &str,
    repo_cache_dir: &Path,
    downloads: &DownloadService,
    sink: Option<DownloadEventSink>,
) -> Result<DesiredManifest> {
    let span = debug_span!("sync.load_desired_manifest_from_swifty_repo", repo_url = %repo_url);
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

    let started_at = Instant::now();
    let manifest = swifty_repo_to_flux_desired_manifest(repo_url, &repo, &mods)
        .context("transform swifty repo -> flux DesiredManifest")?;
    debug!(
        elapsed_ms = started_at.elapsed().as_millis(),
        "transformed swifty repo -> flux DesiredManifest"
    );
    Ok(manifest)
}

fn swifty_repo_to_flux_desired_manifest(
    repo_url: &str,
    repo: &swifty_artifacts::RepoSpec,
    mods: &BTreeMap<String, swifty_artifacts::SrfMod>,
) -> Result<DesiredManifest> {
    let base_url = url::Url::parse(repo_url)
        .context("parse repo url")?
        .join("./")
        .context("resolve repo base url")?;

    let prune_paths: Vec<PathBuf> = Vec::new();

    let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
    let mut files_by_path: BTreeMap<PathBuf, ManifestEntry> = BTreeMap::new();

    let ordered_mod_names: Vec<String> = repo
        .required_mods
        .iter()
        .chain(repo.optional_mods.iter())
        .map(|m| m.mod_name.to_ascii_lowercase())
        .collect::<Vec<_>>();
    debug!(
        required = repo.required_mods.len(),
        optional = repo.optional_mods.len(),
        mods_loaded = mods.len(),
        "swifty repo -> flux manifest mod counts"
    );

    for name in ordered_mod_names {
        let Some(mod_m) = mods.get(&name) else {
            error!(mod_name = name.as_str(), "missing mod manifest in cache");
            anyhow::bail!("missing mod manifest for {name}");
        };

        for file in &mod_m.files {
            let file_rel = normalize_swifty_file_rel_path(&mod_m.name, &file.path)?;

            if file.length > 0 {
                swifty_artifacts::validate_parts_swifty_strict(
                    &file.path,
                    file.length,
                    &file.parts,
                )
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

            let rel = format!("{}/{}", mod_m.name, file_rel);
            let rel_path = PathBuf::from(&rel);
            let source_url = base_url
                .join(&rel)
                .with_context(|| format!("join mod file url {rel}"))?;

            let segments: Vec<SegmentSpec> = file
                .parts
                .iter()
                .filter(|part| part.length > 0)
                .map(|part| SegmentSpec {
                    source: SourceRef::Http {
                        url: Arc::from(source_url.to_string()),
                    },
                    src_offset: part.start,
                    len: part.length,
                    signature: Signature {
                        scheme: Arc::from("md5"),
                        value_hex: Arc::from(part.checksum.to_hex_upper()),
                        size_bytes: part.length,
                    },
                })
                .collect();

            if file.length > 0 {
                let sum: u64 = segments.iter().map(|s| s.len).sum();
                if sum != file.length {
                    anyhow::bail!(
                        "invalid swifty part coverage after dropping 0-len parts: mod={} path={} file_len={} seg_sum={}",
                        mod_m.name,
                        file.path,
                        file.length,
                        sum
                    );
                }
            }

            let entry = ManifestEntry::File(flux_manifest::ManifestFile {
                rel_path: rel_path.clone(),
                size_bytes: file.length,
                segments,
                mode: None,
                mtime_ns: None,
            });

            add_parent_dirs(&mut dirs, &rel_path);
            files_by_path.insert(rel_path, entry);
        }
    }

    let mut entries: Vec<ManifestEntry> = Vec::new();
    for d in dirs {
        entries.push(ManifestEntry::Dir { rel_path: d });
    }
    for (_, f) in files_by_path {
        entries.push(f);
    }

    let manifest = DesiredManifest {
        version: ManifestVersion::V1,
        entries,
        prune_paths,
    };
    if let Err(err) = flux_manifest::validate_desired_manifest(&manifest) {
        error!(error = %err, "validate transformed desired manifest failed");
        return Err(err).context("validate transformed desired manifest");
    }
    Ok(manifest)
}

fn add_parent_dirs(out: &mut BTreeSet<PathBuf>, rel_path: &Path) {
    let p = rel_path.to_string_lossy().replace('\\', "/");
    if !p.contains('/') {
        return;
    }

    let parts: Vec<&str> = p.split('/').collect();
    if parts.len() < 2 {
        return;
    }

    let mut acc = PathBuf::new();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        if !part.is_empty() {
            acc.push(part);
            out.insert(acc.clone());
        }
    }
}

fn normalize_swifty_file_rel_path(mod_name: &str, raw_path: &str) -> Result<String> {
    let normalized = raw_path.replace('\\', "/");
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
    use super::{normalize_swifty_file_rel_path, swifty_repo_to_flux_desired_manifest};
    use flux_manifest::ManifestEntry;
    use std::collections::BTreeMap;

    fn md5(hex: &str) -> swifty_artifacts::Md5Digest {
        swifty_artifacts::Md5Digest::parse_hex(hex).expect("valid md5")
    }

    fn base_repo_spec(mod_name: &str) -> swifty_artifacts::RepoSpec {
        swifty_artifacts::RepoSpec {
            repo_name: "test".to_string(),
            checksum: "deadbeef".to_string(),
            required_mods: vec![swifty_artifacts::RepoMod {
                mod_name: mod_name.to_string(),
                checksum: md5("00000000000000000000000000000000"),
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

    fn srf_mod(mod_name: &str, file_paths: &[&str]) -> swifty_artifacts::SrfMod {
        swifty_artifacts::SrfMod {
            name: mod_name.to_string(),
            checksum: md5("00000000000000000000000000000000"),
            files: file_paths
                .iter()
                .map(|path| swifty_artifacts::SrfFile {
                    path: (*path).to_string(),
                    length: 0,
                    checksum: md5("00000000000000000000000000000000"),
                    r#type: None,
                    parts: vec![],
                })
                .collect(),
        }
    }

    #[test]
    fn normalizes_dot_prefixed_windows_style_paths() {
        let normalized =
            normalize_swifty_file_rel_path("ace", ".\\addons\\ace_main.pbo").expect("normalize");
        assert_eq!(normalized, "addons/ace_main.pbo");
    }

    #[test]
    fn normalizes_redundant_separators() {
        let normalized =
            normalize_swifty_file_rel_path("ace", "addons//weapons///x.pbo").expect("normalize");
        assert_eq!(normalized, "addons/weapons/x.pbo");
    }

    #[test]
    fn rejects_parent_traversal_segments() {
        let err = normalize_swifty_file_rel_path("ace", "../x.pbo").expect_err("must fail");
        assert!(err.to_string().contains("parent traversal"));

        let err = normalize_swifty_file_rel_path("ace", "addons/../x.pbo").expect_err("must fail");
        assert!(err.to_string().contains("parent traversal"));
    }

    #[test]
    fn manifest_rel_paths_are_canonicalized_for_swifty_files() {
        let repo = base_repo_spec("ace");
        let mut mods = BTreeMap::new();
        mods.insert(
            "ace".to_string(),
            srf_mod("ace", &[".\\addons\\a.pbo", "addons//weapons///b.pbo"]),
        );

        let manifest =
            swifty_repo_to_flux_desired_manifest("https://example.com/repo.json", &repo, &mods)
                .expect("manifest");

        let file_paths: Vec<String> = manifest
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ManifestEntry::File(file) => Some(file.rel_path.to_string_lossy().to_string()),
                _ => None,
            })
            .collect();

        assert!(file_paths.contains(&"ace/addons/a.pbo".to_string()));
        assert!(file_paths.contains(&"ace/addons/weapons/b.pbo".to_string()));
        for rel in file_paths {
            assert!(!rel.contains("./"), "path was not canonicalized: {rel}");
            assert!(!rel.contains("//"), "path was not canonicalized: {rel}");
            assert!(!rel.contains('\\'), "path was not canonicalized: {rel}");
        }
    }
}
