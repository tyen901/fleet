use anyhow::{Context, Result};
use relative_path::RelativePathBuf;

use crate::formats::{is_legacy_srf, legacy_srf, strip_utf8_bom};
use crate::ModManifest;

pub fn parse_any(bytes: &[u8]) -> Result<ModManifest> {
    let bytes = strip_utf8_bom(bytes);

    let mut manifest = if is_legacy_srf(bytes) {
        let text = String::from_utf8_lossy(bytes);
        let trimmed = text.trim_start_matches('\u{feff}').trim_start();
        legacy_srf::parse(trimmed)?
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
