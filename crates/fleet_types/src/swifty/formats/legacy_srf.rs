// Minimal legacy SRF parsing for compatibility tests.
use crate::swifty::model::ModManifest;

pub fn parse(_bytes: &[u8]) -> anyhow::Result<ModManifest> {
    Err(anyhow::anyhow!("legacy SRF parsing not implemented"))
}
