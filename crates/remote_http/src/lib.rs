use bytes::Bytes;
use futures_util::StreamExt;
use manifest_types::{ModManifest, RepoBasicAuth, RepoSpec};
use relative_path::RelativePath;
use remote_core::{ByteStream, RemoteError, RemoteRepo, RemoteSession};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT_ENCODING, RANGE};
use std::sync::Arc;
use url::Url;

const REPO_JSON: &str = "repo.json";
const MOD_MANIFEST_JSON: &str = "manifest.json";
const MOD_SRF: &str = "mod.srf";

#[derive(Clone)]
pub struct HttpRemoteRepo {
    base: Url,
    client: reqwest::Client,
}

impl HttpRemoteRepo {
    pub fn new(base_url: &str) -> Result<Self, RemoteError> {
        let base = Url::parse(base_url)
            .map_err(|e| RemoteError::Protocol(format!("invalid base url: {e}")))?;

        let mut default_headers = HeaderMap::new();
        default_headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));

        let client = reqwest::Client::builder()
            .default_headers(default_headers)
            .no_gzip()
            .no_brotli()
            .no_zstd()
            .pool_max_idle_per_host(32)
            .tcp_keepalive(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| RemoteError::Http(format!("failed to build client: {e}")))?;

        Ok(Self { base, client })
    }
}

pub struct HttpRemoteSession {
    base: Url,
    client: reqwest::Client,
    repo: RepoSpec,
    auth: Option<Arc<RepoBasicAuth>>,
}

impl HttpRemoteSession {
    fn url_mod_manifest(&self, mod_name: &str, manifest_name: &str) -> Result<Url, RemoteError> {
        self.base
            .join(&format!("{mod_name}/{manifest_name}"))
            .map_err(|e| RemoteError::Protocol(format!("bad mod manifest url join: {e}")))
    }

    fn url_file(&self, mod_name: &str, rel_path: &RelativePath) -> Result<Url, RemoteError> {
        let rel = rel_path.as_str();
        self.base
            .join(&format!("{mod_name}/{rel}"))
            .map_err(|e| RemoteError::Protocol(format!("bad file url join: {e}")))
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(auth) = &self.auth {
            req.basic_auth(auth.username.clone(), Some(auth.password.clone()))
        } else {
            req
        }
    }

    async fn send_bytes_stream(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<ByteStream, RemoteError> {
        let resp = req
            .send()
            .await
            .map_err(|e| RemoteError::Http(format!("{e}")))?;

        if !resp.status().is_success() {
            return Err(RemoteError::Http(format!("http status {}", resp.status())));
        }

        let s = resp.bytes_stream().map(|r| {
            r.map_err(|e| RemoteError::Http(format!("stream error: {e}")))
                .map(|b: Bytes| b)
        });

        Ok(Box::pin(s))
    }
}

fn summarize_body_prefix(bytes: &[u8]) -> String {
    let n = bytes.len().min(64);
    let prefix = &bytes[..n];
    let hex = hex::encode(prefix);
    let text = String::from_utf8_lossy(prefix)
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("hex:{hex} text:{text}")
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    const BOM: &[u8] = b"\xEF\xBB\xBF";
    bytes.strip_prefix(BOM).unwrap_or(bytes)
}

async fn send_json_repo_spec(req: reqwest::RequestBuilder) -> Result<RepoSpec, RemoteError> {
    let resp = req
        .send()
        .await
        .map_err(|e| RemoteError::Http(format!("{e}")))?;

    if !resp.status().is_success() {
        return Err(RemoteError::Http(format!("http status {}", resp.status())));
    }

    let headers = resp.headers().clone();
    let body = resp
        .bytes()
        .await
        .map_err(|e| RemoteError::Http(format!("read error: {e}")))?;

    let body = strip_utf8_bom(&body);
    serde_json::from_slice::<RepoSpec>(body).map_err(|e| {
        let encoding = headers
            .get(reqwest::header::CONTENT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("<none>");
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("<none>");
        RemoteError::Deserialize(format!(
            "{e} (content-type={content_type}, content-encoding={encoding}, body_prefix={})",
            summarize_body_prefix(body)
        ))
    })
}

#[async_trait::async_trait]
impl RemoteRepo for HttpRemoteRepo {
    type Session = HttpRemoteSession;

    async fn open_session(&self) -> Result<Self::Session, RemoteError> {
        let repo_url = self
            .base
            .join(REPO_JSON)
            .map_err(|e| RemoteError::Protocol(format!("bad repo.json url join: {e}")))?;

        let repo = send_json_repo_spec(self.client.get(repo_url)).await?;

        let auth = repo.repo_basic_authentication.clone().map(Arc::new);

        Ok(HttpRemoteSession {
            base: self.base.clone(),
            client: self.client.clone(),
            repo,
            auth,
        })
    }
}

#[async_trait::async_trait]
impl RemoteSession for HttpRemoteSession {
    fn repo_spec(&self) -> &RepoSpec {
        &self.repo
    }

    async fn fetch_mod_manifest(&self, mod_name: &str) -> Result<ModManifest, RemoteError> {
        async fn try_fetch(
            session: &HttpRemoteSession,
            url: Url,
        ) -> Result<Option<ModManifest>, RemoteError> {
            let req = session.apply_auth(session.client.get(url));
            let resp = req
                .send()
                .await
                .map_err(|e| RemoteError::Http(format!("{e}")))?;

            if resp.status().as_u16() == 404 {
                return Ok(None);
            }
            if !resp.status().is_success() {
                return Err(RemoteError::Http(format!("http status {}", resp.status())));
            }

            let headers = resp.headers().clone();
            let body = resp
                .bytes()
                .await
                .map_err(|e| RemoteError::Http(format!("read error: {e}")))?;

            let body = strip_utf8_bom(&body);
            serde_json::from_slice::<ModManifest>(body)
                .map(Some)
                .map_err(|e| {
                    let encoding = headers
                        .get(reqwest::header::CONTENT_ENCODING)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("<none>");
                    let content_type = headers
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("<none>");
                    RemoteError::Deserialize(format!(
                        "{e} (content-type={content_type}, content-encoding={encoding}, body_prefix={})",
                        summarize_body_prefix(body)
                    ))
                })
        }

        let primary_url = self.url_mod_manifest(mod_name, MOD_MANIFEST_JSON)?;
        if let Some(m) = try_fetch(self, primary_url).await? {
            return Ok(m);
        }

        let fallback_url = self.url_mod_manifest(mod_name, MOD_SRF)?;
        if let Some(m) = try_fetch(self, fallback_url).await? {
            return Ok(m);
        }

        Err(RemoteError::Http(format!(
            "http status 404 Not Found (no {MOD_MANIFEST_JSON} or {MOD_SRF})"
        )))
    }

    async fn fetch_range(
        &self,
        mod_name: &str,
        rel_path: &RelativePath,
        start: u64,
        length: u64,
    ) -> Result<ByteStream, RemoteError> {
        if length == 0 {
            return Err(RemoteError::Protocol("range length must be > 0".into()));
        }

        let url = self.url_file(mod_name, rel_path)?;
        let end_inclusive = start
            .checked_add(length - 1)
            .ok_or_else(|| RemoteError::Protocol("range overflow".into()))?;

        let mut headers = HeaderMap::new();
        headers.insert(
            RANGE,
            HeaderValue::from_str(&format!("bytes={start}-{end_inclusive}"))
                .map_err(|e| RemoteError::Protocol(format!("invalid range header: {e}")))?,
        );

        let req = self.apply_auth(self.client.get(url).headers(headers));

        let resp = req
            .send()
            .await
            .map_err(|e| RemoteError::Http(format!("{e}")))?;

        if resp.status().as_u16() != 206 {
            return Err(RemoteError::Protocol(format!(
                "range not supported or not honored (status {})",
                resp.status()
            )));
        }

        let s = resp
            .bytes_stream()
            .map(|r| r.map_err(|e| RemoteError::Http(format!("stream error: {e}"))));

        Ok(Box::pin(s))
    }

    async fn fetch_file(
        &self,
        mod_name: &str,
        rel_path: &RelativePath,
    ) -> Result<ByteStream, RemoteError> {
        let url = self.url_file(mod_name, rel_path)?;
        let req = self.apply_auth(self.client.get(url));
        self.send_bytes_stream(req).await
    }
}
