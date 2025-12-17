use async_trait::async_trait;
use bytes::Bytes;
use futures_core::Stream;
use manifest_types::{ModManifest, RepoSpec};
use relative_path::RelativePath;
use std::pin::Pin;

pub type ByteStream =
    Pin<Box<dyn Stream<Item = Result<Bytes, RemoteError>> + Send + Sync + 'static>>;

#[derive(thiserror::Error, Debug)]
pub enum RemoteError {
    #[error("http error: {0}")]
    Http(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("deserialize error: {0}")]
    Deserialize(String),
    #[error("io error: {0}")]
    Io(String),
}

#[async_trait]
pub trait RemoteRepo: Send + Sync {
    type Session: RemoteSession;

    async fn open_session(&self) -> Result<Self::Session, RemoteError>;
}

#[async_trait]
pub trait RemoteSession: Send + Sync {
    fn repo_spec(&self) -> &RepoSpec;

    async fn fetch_mod_manifest(&self, mod_name: &str) -> Result<ModManifest, RemoteError>;

    async fn fetch_range(
        &self,
        mod_name: &str,
        rel_path: &RelativePath,
        start: u64,
        length: u64,
    ) -> Result<ByteStream, RemoteError>;

    async fn fetch_file(
        &self,
        mod_name: &str,
        rel_path: &RelativePath,
    ) -> Result<ByteStream, RemoteError>;
}
