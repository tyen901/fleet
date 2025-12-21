use anyhow::Result;
use async_trait::async_trait;

use crate::fetch::ModManifest;

#[derive(Clone, Debug, Default)]
pub struct RemoteCapabilities {
    pub supports_ranges: bool,
}

#[async_trait]
pub trait RemoteRepo: Send + Sync {
    async fn capabilities(&self) -> Result<RemoteCapabilities> {
        Ok(RemoteCapabilities::default())
    }

    async fn fetch_mod_manifest(&self, mod_id: &str) -> Result<ModManifest>;

    async fn fetch_file(&self, mod_id: &str, rel_path: &str) -> Result<RemoteStream>;
    async fn fetch_range(
        &self,
        mod_id: &str,
        rel_path: &str,
        offset: u64,
        len: u64,
    ) -> Result<RemoteStream>;
}

pub struct RemoteStream {
    inner: Box<dyn RemoteStreamImpl>,
}
impl RemoteStream {
    pub fn new(inner: Box<dyn RemoteStreamImpl>) -> Self {
        Self { inner }
    }
    pub async fn next_chunk(&mut self) -> Result<Option<bytes::Bytes>> {
        self.inner.next_chunk().await
    }
}

#[async_trait]
pub trait RemoteStreamImpl: Send {
    async fn next_chunk(&mut self) -> Result<Option<bytes::Bytes>>;
}
