#[derive(thiserror::Error, Debug)]
pub enum WireError {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn parse_repo_spec_json(bytes: &[u8]) -> Result<crate::model::RepoSpec, WireError> {
    Ok(serde_json::from_slice(bytes)?)
}

pub fn parse_mod_manifest_json(bytes: &[u8]) -> Result<crate::model::ModManifest, WireError> {
    Ok(serde_json::from_slice(bytes)?)
}

pub fn emit_repo_spec_json(v: &crate::model::RepoSpec) -> Result<Vec<u8>, WireError> {
    Ok(serde_json::to_vec_pretty(v)?)
}

pub fn emit_mod_manifest_json(v: &crate::model::ModManifest) -> Result<Vec<u8>, WireError> {
    Ok(serde_json::to_vec_pretty(v)?)
}
