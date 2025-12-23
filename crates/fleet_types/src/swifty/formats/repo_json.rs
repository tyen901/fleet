use crate::swifty::model::RepoSpec;
use serde_json::from_slice;

pub fn parse(bytes: &[u8]) -> anyhow::Result<RepoSpec> {
    let spec: RepoSpec = from_slice(bytes)?;
    Ok(spec)
}
