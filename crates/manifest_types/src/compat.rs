use anyhow::{Context, Result};
use relative_path::RelativePathBuf;

use crate::{FileManifest, Md5Digest, ModManifest, PartManifest, RepoSpec};

pub fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    const BOM: &[u8] = b"\xEF\xBB\xBF";
    if bytes.starts_with(BOM) {
        &bytes[BOM.len()..]
    } else {
        bytes
    }
}

pub fn parse_repo_spec(bytes: &[u8]) -> Result<RepoSpec> {
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
        parse_legacy_srf(trimmed)?
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

fn parse_legacy_srf(text: &str) -> Result<ModManifest> {
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));

    let first = lines.next().context("legacy srf: missing first line")?;
    let mut parts = first.split(':');

    let magic = parts.next().unwrap_or("");
    if magic != "ADDON" {
        anyhow::bail!("legacy srf: expected ADDON, got {magic}");
    }

    let name = parts
        .next()
        .context("legacy srf: missing mod name")?
        .to_string();

    let file_count: usize = parts
        .next()
        .context("legacy srf: missing file_count")?
        .parse()
        .context("legacy srf: file_count parse")?;

    let checksum_hex = parts.next().context("legacy srf: missing checksum")?;
    let checksum = Md5Digest::parse_hex(checksum_hex)
        .map_err(|e| anyhow::anyhow!("legacy srf: checksum parse: {e}"))?;

    let mut files = Vec::with_capacity(file_count);

    for _ in 0..file_count {
        let line = lines.next().context("legacy srf: missing file line")?;
        let mut sp = line.split(':');

        let _file_type = sp.next().context("legacy srf: missing file type")?;
        let raw_path = sp.next().context("legacy srf: missing file path")?;

        let length: u64 = sp
            .next()
            .context("legacy srf: missing file length")?
            .parse()
            .context("legacy srf: file length parse")?;

        let part_count: usize = sp
            .next()
            .context("legacy srf: missing part count")?
            .parse()
            .context("legacy srf: part count parse")?;

        let file_checksum_hex = sp.next().context("legacy srf: missing file checksum")?;

        let norm_path = raw_path.replace('\\', "/");
        let path = RelativePathBuf::from(norm_path);

        let checksum_file = Md5Digest::parse_hex(file_checksum_hex)
            .map_err(|e| anyhow::anyhow!("legacy srf: file checksum parse: {e}"))?;

        let mut file_parts = Vec::with_capacity(part_count);

        for _ in 0..part_count {
            let pline = lines.next().context("legacy srf: missing part line")?;
            let mut pp = pline.split(':');

            let _part_name = pp.next().context("legacy srf: missing part name")?;

            let start: u64 = pp
                .next()
                .context("legacy srf: missing part start")?
                .parse()
                .context("legacy srf: part start parse")?;

            let plen: u64 = pp
                .next()
                .context("legacy srf: missing part length")?
                .parse()
                .context("legacy srf: part length parse")?;

            let pchk_hex = pp.next().context("legacy srf: missing part checksum")?;
            let pchk = Md5Digest::parse_hex(pchk_hex)
                .map_err(|e| anyhow::anyhow!("legacy srf: part checksum parse: {e}"))?;

            file_parts.push(PartManifest {
                start,
                length: plen,
                checksum: pchk,
            });
        }

        files.push(FileManifest {
            path,
            length,
            checksum: checksum_file,
            parts: file_parts,
        });
    }

    Ok(ModManifest {
        name,
        checksum,
        files,
    })
}
