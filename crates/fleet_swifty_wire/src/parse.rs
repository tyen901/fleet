#[derive(thiserror::Error, Debug)]
pub enum WireError {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("legacy text srf: {0}")]
    LegacyText(#[from] crate::legacy_srf_text::LegacyTextSrfError),
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    const BOM: &[u8] = b"\xEF\xBB\xBF";
    bytes.strip_prefix(BOM).unwrap_or(bytes)
}

pub enum ModSrfWire {
    Json(crate::srf_json::SrfJsonMod),
    LegacyText(crate::legacy_srf_text::LegacyTextMod),
}

pub fn parse_repo_spec_json(bytes: &[u8]) -> Result<crate::model::RepoSpec, WireError> {
    Ok(serde_json::from_slice(strip_utf8_bom(bytes))?)
}

pub fn parse_mod_srf(bytes: &[u8]) -> Result<ModSrfWire, WireError> {
    let bytes = strip_utf8_bom(bytes);

    if crate::legacy_srf_text::is_legacy_text_srf(bytes) {
        Ok(ModSrfWire::LegacyText(
            crate::legacy_srf_text::parse_legacy_text_srf(bytes)?,
        ))
    } else {
        Ok(ModSrfWire::Json(serde_json::from_slice(bytes)?))
    }
}

pub fn emit_repo_spec_json(v: &crate::model::RepoSpec) -> Result<Vec<u8>, WireError> {
    Ok(serde_json::to_vec_pretty(v)?)
}
