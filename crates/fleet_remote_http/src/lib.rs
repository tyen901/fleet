use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use fleet_formats::compat;
use reqwest::header::{HeaderValue, RANGE};
use std::sync::Mutex;
use sync_engine::fetch::{FileEntry, FilePart, ModManifest};
use sync_engine::remote::{RemoteCapabilities, RemoteRepo, RemoteStream, RemoteStreamImpl};
use tokio::sync::Mutex as AsyncMutex;
use url::Url;

fn apply_basic_auth(
    mut req: reqwest::RequestBuilder,
    auth: &Option<(String, String)>,
) -> reqwest::RequestBuilder {
    if let Some((user, pass)) = auth {
        req = req.basic_auth(user, Some(pass));
    }
    req
}

pub struct HttpRemote {
    base: Url,
    client: reqwest::Client,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    caps: Option<RemoteCapabilities>,
    basic_auth: Option<(String, String)>,
    repo_spec: Option<manifest_types::RepoSpec>,
}

impl HttpRemote {
    pub fn new(base_url: &str) -> Result<Self> {
        let mut base = Url::parse(base_url)?;
        // Normalize to ensure path ends with '/'
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path().trim_end_matches('/')));
        }

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;

        Ok(Self {
            base,
            client,
            state: Mutex::new(State::default()),
        })
    }

    fn url_join(&self, p: &str) -> Result<Url> {
        // Ensure no leading slash breaks join semantics.
        let p = p.trim_start_matches('/');
        Ok(self.base.join(p)?)
    }

    async fn ensure_repo_loaded(&self) -> Result<()> {
        // Fast-path: if we've already loaded auth and caps, stop.
        {
            let st = self.state.lock().unwrap();
            if st.caps.is_some() && st.repo_spec.is_some() {
                return Ok(());
            }
        }

        // 1) Probe range support with GET repo.json Range: bytes=0-0.
        let repo_url = self.url_join("repo.json")?;
        let probe = self
            .client
            .get(repo_url.clone())
            .header(RANGE, "bytes=0-0")
            .send()
            .await
            .context("range probe repo.json")?;

        let supports_ranges = probe.status().as_u16() == 206;

        // 2) Fetch full repo.json to get optional basic auth credentials.
        let resp = self
            .client
            .get(repo_url)
            .send()
            .await
            .context("fetch repo.json")?
            .error_for_status()
            .context("repo.json status")?;

        let bytes = resp.bytes().await.context("read repo.json body")?;
        let repo = compat::parse_repo_spec(&bytes)?;

        let basic_auth = repo
            .repo_basic_authentication
            .as_ref()
            .map(|a| (a.username.clone(), a.password.clone()));

        let mut st = self.state.lock().unwrap();
        st.caps = Some(RemoteCapabilities { supports_ranges });
        st.basic_auth = basic_auth;
        st.repo_spec = Some(repo);
        Ok(())
    }

    pub async fn fetch_repo_spec(&self) -> Result<manifest_types::RepoSpec> {
        self.ensure_repo_loaded().await?;
        Ok(self
            .state
            .lock()
            .unwrap()
            .repo_spec
            .clone()
            .expect("repo_spec is loaded"))
    }
}

struct ResponseStream {
    resp: AsyncMutex<reqwest::Response>,
}

#[async_trait]
impl RemoteStreamImpl for ResponseStream {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>> {
        let mut resp = self.resp.lock().await;
        let chunk = resp.chunk().await?;
        Ok(chunk)
    }
}

#[async_trait]
impl RemoteRepo for HttpRemote {
    async fn capabilities(&self) -> Result<RemoteCapabilities> {
        self.ensure_repo_loaded().await?;
        Ok(self.state.lock().unwrap().caps.clone().unwrap_or_default())
    }

    async fn fetch_mod_manifest(&self, mod_id: &str) -> Result<ModManifest> {
        self.ensure_repo_loaded().await?;
        let auth = self.state.lock().unwrap().basic_auth.clone();

        // Prefer manifest.json, fall back to mod.srf. Both may contain BOM / legacy SRF.
        let manifest_url = self.url_join(&format!("{}/manifest.json", mod_id))?;
        let req = apply_basic_auth(self.client.get(manifest_url), &auth);
        let res = req.send().await?;
        if res.status().as_u16() != 404 {
            let res = res.error_for_status()?;
            let bytes = res.bytes().await?;
            let parsed = compat::parse_mod_manifest_any(&bytes)?;
            return Ok(ModManifest {
                mod_id: parsed.name,
                files: parsed
                    .files
                    .into_iter()
                    .map(|f| FileEntry {
                        rel_path: f.path.as_str().replace('\\', "/"),
                        size: f.length,
                        file_checksum: f.checksum.as_bytes().to_vec(),
                        parts: f
                            .parts
                            .into_iter()
                            .map(|p| FilePart {
                                offset: p.start,
                                len: p.length,
                                checksum: p.checksum.as_bytes().to_vec(),
                            })
                            .collect(),
                    })
                    .collect(),
            });
        }

        let srf_url = self.url_join(&format!("{}/mod.srf", mod_id))?;
        let req = apply_basic_auth(self.client.get(srf_url), &auth);
        let res = req.send().await?.error_for_status()?;
        let bytes = res.bytes().await?;
        let parsed = compat::parse_mod_manifest_any(&bytes)?;

        Ok(ModManifest {
            mod_id: parsed.name,
            files: parsed
                .files
                .into_iter()
                .map(|f| FileEntry {
                    rel_path: f.path.as_str().replace('\\', "/"),
                    size: f.length,
                    file_checksum: f.checksum.as_bytes().to_vec(),
                    parts: f
                        .parts
                        .into_iter()
                        .map(|p| FilePart {
                            offset: p.start,
                            len: p.length,
                            checksum: p.checksum.as_bytes().to_vec(),
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    async fn fetch_file(&self, mod_id: &str, rel_path: &str) -> Result<RemoteStream> {
        self.ensure_repo_loaded().await?;
        let auth = self.state.lock().unwrap().basic_auth.clone();
        let url = self.url_join(&format!("{}/{}", mod_id, rel_path))?;

        let req = apply_basic_auth(self.client.get(url), &auth);
        let resp = req.send().await?.error_for_status()?;
        Ok(RemoteStream::new(Box::new(ResponseStream {
            resp: AsyncMutex::new(resp),
        })))
    }

    async fn fetch_range(
        &self,
        mod_id: &str,
        rel_path: &str,
        offset: u64,
        len: u64,
    ) -> Result<RemoteStream> {
        self.ensure_repo_loaded().await?;
        let auth = self.state.lock().unwrap().basic_auth.clone();
        let caps = self.state.lock().unwrap().caps.clone().unwrap_or_default();
        if !caps.supports_ranges {
            anyhow::bail!("remote does not support range requests");
        }
        let url = self.url_join(&format!("{}/{}", mod_id, rel_path))?;

        let end = offset.saturating_add(len).saturating_sub(1);
        let req = self.client.get(url).header(
            RANGE,
            HeaderValue::from_str(&format!("bytes={}-{}", offset, end))?,
        );
        let req = apply_basic_auth(req, &auth);
        let resp = req.send().await?;
        if resp.status().as_u16() != 206 {
            anyhow::bail!("range not supported or ignored (status {})", resp.status());
        }
        let resp = resp.error_for_status()?;
        Ok(RemoteStream::new(Box::new(ResponseStream {
            resp: AsyncMutex::new(resp),
        })))
    }
}
