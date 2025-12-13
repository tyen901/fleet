use anyhow::Result;
use std::sync::OnceLock;

static RUNTIME: OnceLock<std::result::Result<tokio::runtime::Runtime, String>> = OnceLock::new();

pub(crate) fn runtime() -> Result<&'static tokio::runtime::Runtime> {
    match RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().map_err(|e| e.to_string())) {
        Ok(rt) => Ok(rt),
        Err(message) => Err(anyhow::anyhow!(message.clone())),
    }
}
