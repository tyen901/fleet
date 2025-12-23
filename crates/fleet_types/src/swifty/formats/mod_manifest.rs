use crate::swifty::model::ModManifest;
use serde_json::from_slice;

pub fn parse_any(bytes: &[u8]) -> anyhow::Result<ModManifest> {
    let m: ModManifest = from_slice(bytes)?;
    Ok(m)
}
