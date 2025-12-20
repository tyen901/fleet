#![forbid(unsafe_code)]

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use relative_path::RelativePath;
use remote_core::{RemoteRepo as _, RemoteSession as _};
use tokio::sync::OnceCell;

pub struct HttpRemoteAdapter {
    inner: remote_http::HttpRemoteRepo,
    session: OnceCell<remote_http::HttpRemoteSession>,
}

impl HttpRemoteAdapter {
    pub fn new(base_url: &str) -> Result<Self> {
        let inner = remote_http::HttpRemoteRepo::new(base_url)
            .map_err(|e| anyhow::anyhow!("remote_http: {e}"))?;
        Ok(Self {
            inner,
            session: OnceCell::new(),
        })
    }

    pub async fn fetch_raw_repo_spec(&self) -> Result<manifest_types::RepoSpec> {
        let session = self.session().await?;
        Ok(session.repo_spec().clone())
    }

    async fn session(&self) -> Result<&remote_http::HttpRemoteSession> {
        self.session
            .get_or_try_init(|| async {
                self.inner
                    .open_session()
                    .await
                    .map_err(|e| anyhow::anyhow!("open_session: {e}"))
            })
            .await
    }
}

#[async_trait]
impl sync_engine::remote::RemoteRepo for HttpRemoteAdapter {
    async fn capabilities(&self) -> Result<sync_engine::remote::RemoteCapabilities> {
        Ok(sync_engine::remote::RemoteCapabilities {
            supports_ranges: true,
        })
    }

    async fn fetch_repo_spec(&self) -> Result<sync_engine::types::RepoSpec> {
        let session = self.session().await?;
        let spec = session.repo_spec();

        let mods = spec
            .required_mods
            .iter()
            .chain(spec.optional_mods.iter())
            .map(|m| sync_engine::types::ModSpec {
                mod_id: m.mod_name.clone(),
                version: m.checksum.to_hex_upper(),
            })
            .collect();

        Ok(sync_engine::types::RepoSpec { mods })
    }

    async fn fetch_mod_manifest(&self, mod_id: &str) -> Result<sync_engine::types::ModManifest> {
        let session = self.session().await?;
        let manifest = session
            .fetch_mod_manifest(mod_id)
            .await
            .map_err(|e| anyhow::anyhow!("fetch_mod_manifest({mod_id}): {e}"))?;

        let files = manifest
            .files
            .into_iter()
            .map(|f| sync_engine::types::FileEntry {
                rel_path: f.path.as_str().replace('\\', "/"),
                size: f.length,
                file_checksum: sync_engine::types::Checksum {
                    bytes: f.checksum.as_bytes().to_vec(),
                },
                parts: f
                    .parts
                    .into_iter()
                    .map(|p| sync_engine::types::FilePart {
                        offset: p.start,
                        len: p.length,
                        checksum: sync_engine::types::Checksum {
                            bytes: p.checksum.as_bytes().to_vec(),
                        },
                    })
                    .collect(),
            })
            .collect();

        Ok(sync_engine::types::ModManifest {
            mod_id: manifest.name,
            version: manifest.checksum.to_hex_upper(),
            files,
        })
    }

    async fn fetch_file(
        &self,
        mod_id: &str,
        rel_path: &str,
    ) -> Result<sync_engine::remote::RemoteStream> {
        let session = self.session().await?;
        let rel = RelativePath::new(rel_path);
        let stream = session
            .fetch_file(mod_id, rel)
            .await
            .map_err(|e| anyhow::anyhow!("fetch_file({mod_id},{rel_path}): {e}"))?;
        Ok(sync_engine::remote::RemoteStream::new(Box::new(
            ByteStreamImpl { inner: stream },
        )))
    }

    async fn fetch_range(
        &self,
        mod_id: &str,
        rel_path: &str,
        offset: u64,
        len: u64,
    ) -> Result<sync_engine::remote::RemoteStream> {
        let session = self.session().await?;
        let rel = RelativePath::new(rel_path);
        let stream = session
            .fetch_range(mod_id, rel, offset, len)
            .await
            .map_err(|e| anyhow::anyhow!("fetch_range({mod_id},{rel_path},{offset},{len}): {e}"))?;
        Ok(sync_engine::remote::RemoteStream::new(Box::new(
            ByteStreamImpl { inner: stream },
        )))
    }
}

struct ByteStreamImpl {
    inner: remote_core::ByteStream,
}

#[async_trait]
impl sync_engine::remote::RemoteStreamImpl for ByteStreamImpl {
    async fn next_chunk(&mut self) -> Result<Option<bytes::Bytes>> {
        match self.inner.next().await {
            None => Ok(None),
            Some(Ok(b)) => Ok(Some(b)),
            Some(Err(e)) => Err(anyhow::anyhow!("remote stream: {e}")),
        }
    }
}

#[derive(Clone, Default)]
pub struct Md5Checksummer;

impl sync_engine::types::Checksummer for Md5Checksummer {
    fn algorithm_name(&self) -> &str {
        "md5"
    }

    fn hash_bytes(&self, data: &[u8]) -> Result<Vec<u8>> {
        use md5::Digest;
        let mut ctx = md5::Md5::new();
        ctx.update(data);
        Ok(ctx.finalize().to_vec())
    }

    fn hash_file(&self, path: &std::path::Path) -> Result<Vec<u8>> {
        use md5::Digest;
        use std::io::Read;

        let mut f =
            std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mut buf = vec![0u8; 1024 * 1024];
        let mut ctx = md5::Md5::new();
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            ctx.update(&buf[..n]);
        }
        Ok(ctx.finalize().to_vec())
    }

    fn hash_range(&self, path: &std::path::Path, offset: u64, len: u64) -> Result<Vec<u8>> {
        use md5::Digest;
        use std::io::{Read, Seek};

        let mut f =
            std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
        f.seek(std::io::SeekFrom::Start(offset))
            .with_context(|| format!("seek {} to {offset}", path.display()))?;

        let mut ctx = md5::Md5::new();
        let mut buf = vec![0u8; 1024 * 1024];
        let mut remaining = len;
        while remaining > 0 {
            let want = (remaining as usize).min(buf.len());
            let n = f
                .read(&mut buf[..want])
                .with_context(|| format!("read {} @{}+{}", path.display(), offset, len))?;
            if n == 0 {
                anyhow::bail!("short read {} @{}+{}", path.display(), offset, len);
            }
            ctx.update(&buf[..n]);
            remaining -= n as u64;
        }
        Ok(ctx.finalize().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use sync_engine::types::Checksummer;

    #[test]
    fn md5_range_matches_whole_for_single_part() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "hello world").unwrap();

        let checksummer = Md5Checksummer;
        let whole = checksummer.hash_file(tmp.path()).unwrap();
        let part = checksummer
            .hash_range(tmp.path(), 0, "hello world".len() as u64)
            .unwrap();

        assert_eq!(whole, part);
    }
}
