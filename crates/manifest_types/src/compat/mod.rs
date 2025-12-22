use anyhow::{Context, Result};
use relative_path::RelativePathBuf;

use crate::ModManifest;

mod legacy_srf;

pub fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    const BOM: &[u8] = b"\xEF\xBB\xBF";
    if bytes.starts_with(BOM) {
        &bytes[BOM.len()..]
    } else {
        bytes
    }
}

pub fn parse_repo_spec(bytes: &[u8]) -> Result<crate::RepoSpec> {
    let bytes = strip_utf8_bom(bytes);
    serde_json::from_slice(bytes).context("parse repo.json")
}

/// Parse a mod manifest from either:
/// - JSON (manifest.json or JSON SRF)
/// - legacy SRF text starting with "ADDON"
///
/// Normalization guarantees:
/// - BOM is stripped
/// - paths use forward slashes
/// - files are sorted deterministically by rel path (string compare)
pub fn parse_mod_manifest_any(bytes: &[u8]) -> Result<ModManifest> {
    let bytes = strip_utf8_bom(bytes);

    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();

    let mut manifest = if trimmed.starts_with("ADDON") {
        legacy_srf::parse_legacy_srf(trimmed)?
    } else {
        serde_json::from_slice::<ModManifest>(bytes).context("parse JSON mod manifest")?
    };

    for f in &mut manifest.files {
        let norm = f.path.as_str().replace('\\', "/");
        f.path = RelativePathBuf::from(norm);
    }

    manifest
        .files
        .sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));

    Ok(manifest)
}
