use crate::sync::SyncError;
use fleet_core::formats::RepositoryExternal;
use fleet_core::path_utils::FleetPath;
use fleet_core::repo::Repository;
use fleet_core::Manifest;
use futures::StreamExt;
use reqwest::Client;

#[derive(Debug, Clone)]
pub struct RemoteState {
    pub manifest: Manifest,
}

#[async_trait::async_trait]
pub trait RemoteStateProvider: Send + Sync {
    async fn head_repo_json_mtime(&self, repo_url: &str) -> Result<Option<String>, SyncError>;
    async fn fetch_repo_json(&self, repo_url: &str) -> Result<RepositoryExternal, SyncError>;
    async fn fetch_mod_srf(
        &self,
        base: &reqwest::Url,
        mod_name: &str,
    ) -> Result<fleet_core::Mod, SyncError>;
    async fn fetch_remote(&self, repo_url: &str) -> Result<RemoteState, SyncError>;
}

/// HTTP-based remote provider that fetches repo.json and per-mod SRFs.
pub struct HttpRemoteStateProvider {
    client: Client,
}

/// Normalize a repository URL so it can be used as a base for repo.json and mod files.
/// Supports inputs ending with or without `repo.json`.
pub(crate) fn normalize_repo_base(repo_url: &str) -> Result<reqwest::Url, SyncError> {
    let mut url = reqwest::Url::parse(repo_url)
        .map_err(|e| SyncError::Remote(format!("invalid repo url {repo_url}: {e}")))?;

    if let Some(last) = url
        .path_segments()
        .and_then(|mut s| s.next_back().map(|p| p.to_string()))
    {
        if last == "repo.json" {
            url.path_segments_mut()
                .map_err(|_| SyncError::Remote("invalid repo url".into()))?
                .pop();
        }
    }

    // Treat the input as a *directory base* even when the caller provided
    // something like `https://host/path` without a trailing slash.
    //
    // Without this, `Url::join("repo.json")` would resolve to
    // `https://host/repo.json` (replacing `path`) rather than
    // `https://host/path/repo.json`.
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }

    Ok(url)
}

impl HttpRemoteStateProvider {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    async fn manifest_url(&self, repo_url: &str) -> Result<reqwest::Url, SyncError> {
        // If caller already provided repo.json, honor it. Otherwise append it.
        let parsed = reqwest::Url::parse(repo_url)
            .map_err(|e| SyncError::Remote(format!("invalid repo url {repo_url}: {e}")))?;

        if parsed
            .path_segments()
            .and_then(|mut s| s.next_back())
            .is_some_and(|last| last == "repo.json")
        {
            return Ok(parsed);
        }

        let base = normalize_repo_base(repo_url)?;
        base.join("repo.json")
            .map_err(|e| SyncError::Remote(format!("bad repo.json url from {base}: {e}")))
    }

    async fn fetch_repo_json_internal(
        &self,
        repo_url: &str,
    ) -> Result<RepositoryExternal, SyncError> {
        let manifest_url = self.manifest_url(repo_url).await?;

        let bytes = self
            .client
            .get(manifest_url)
            .send()
            .await
            .map_err(|e| SyncError::Remote(format!("repo.json request failed: {e}")))?
            .bytes()
            .await
            .map_err(|e| SyncError::Remote(format!("repo.json bytes failed: {e}")))?;

        serde_json::from_slice(&bytes)
            .map_err(|e| SyncError::Remote(format!("repo.json parse failed: {e}")))
    }

    async fn fetch_mod_srf_internal(
        &self,
        base: &reqwest::Url,
        mod_name: &str,
    ) -> Result<fleet_core::Mod, SyncError> {
        let mut url = base.clone();
        url.path_segments_mut()
            .map_err(|_| SyncError::Remote("invalid base url".into()))?
            .pop_if_empty();
        {
            let mut segs = url
                .path_segments_mut()
                .map_err(|_| SyncError::Remote("cannot mutate url segments".into()))?;
            segs.push(mod_name);
            segs.push("mod.srf");
        }

        let bytes = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| SyncError::Remote(format!("srf request for {mod_name} failed: {e}")))?
            .bytes()
            .await
            .map_err(|e| SyncError::Remote(format!("srf bytes for {mod_name} failed: {e}")))?;

        let mut mod_data = fleet_core::formats::parse_srf(&bytes)
            .map_err(|e| SyncError::Remote(format!("srf parse for {mod_name} failed: {e}")))?;

        // SECURITY & CONSISTENCY: Normalize paths at the boundary.
        // This ensures downstream logic (Diff, Execute) never sees backslashes
        // or inconsistent separators, preventing redownload loops.
        for file in &mut mod_data.files {
            file.path = FleetPath::normalize(&file.path);
            for part in &mut file.parts {
                part.path = FleetPath::normalize(&part.path);
            }
        }

        Ok(mod_data)
    }
}

#[async_trait::async_trait]
impl RemoteStateProvider for HttpRemoteStateProvider {
    async fn head_repo_json_mtime(&self, repo_url: &str) -> Result<Option<String>, SyncError> {
        let manifest_url = self.manifest_url(repo_url).await?;
        let resp = self
            .client
            .head(manifest_url)
            .send()
            .await
            .map_err(|e| SyncError::Remote(format!("repo.json HEAD failed: {e}")))?;
        let last_modified = resp
            .headers()
            .get("Last-Modified")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        Ok(last_modified)
    }

    async fn fetch_repo_json(&self, repo_url: &str) -> Result<RepositoryExternal, SyncError> {
        self.fetch_repo_json_internal(repo_url).await
    }

    async fn fetch_mod_srf(
        &self,
        base: &reqwest::Url,
        mod_name: &str,
    ) -> Result<fleet_core::Mod, SyncError> {
        self.fetch_mod_srf_internal(base, mod_name).await
    }

    async fn fetch_remote(&self, repo_url: &str) -> Result<RemoteState, SyncError> {
        let repo_external = self.fetch_repo_json_internal(repo_url).await?;
        let repository: Repository = repo_external.clone().into();

        let base = normalize_repo_base(repo_url)?;

        let required_mods = repository.required_mods;
        let fetch_stream = futures::stream::iter(required_mods)
            .map(|rmod| {
                let base = base.clone();
                let this = &*self;
                async move { this.fetch_mod_srf_internal(&base, &rmod.mod_name).await }
            })
            .buffer_unordered(20);

        let results: Vec<Result<fleet_core::Mod, SyncError>> = fetch_stream.collect().await;

        let mut mods = Vec::new();
        for res in results {
            mods.push(res?);
        }

        let manifest = Manifest {
            version: "1.0".to_string(),
            mods,
        };

        Ok(RemoteState { manifest })
    }
}
