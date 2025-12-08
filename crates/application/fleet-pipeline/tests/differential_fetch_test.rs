use async_trait::async_trait;
use camino::Utf8PathBuf;
use fleet_core::{Manifest, Mod};
use fleet_pipeline::sync::remote::RemoteStateProvider;
use fleet_pipeline::sync::storage::{FileManifestStore, ManifestStore};
use fleet_pipeline::sync::{DefaultSyncEngine, SyncError, SyncMode, SyncOptions, SyncRequest};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

// Mock Remote
struct MockRemote {
    repo_ext: fleet_core::formats::RepositoryExternal,
    srf_calls: Arc<Mutex<Vec<String>>>, // Track which mods were fetched
}

#[async_trait]
impl RemoteStateProvider for MockRemote {
    async fn head_repo_json_mtime(&self, _: &str) -> Result<Option<String>, SyncError> {
        Ok(None)
    }

    async fn fetch_repo_json(
        &self,
        _: &str,
    ) -> Result<fleet_core::formats::RepositoryExternal, SyncError> {
        Ok(self.repo_ext.clone())
    }

    async fn fetch_mod_srf(&self, _: &reqwest::Url, mod_name: &str) -> Result<Mod, SyncError> {
        self.srf_calls.lock().unwrap().push(mod_name.to_string());
        // Return dummy mod
        Ok(Mod {
            name: mod_name.into(),
            checksum: "hash".into(),
            files: vec![],
        })
    }

    async fn fetch_remote(
        &self,
        _: &str,
    ) -> Result<fleet_pipeline::sync::remote::RemoteState, SyncError> {
        unimplemented!("Not needed for plan()")
    }
}

#[tokio::test]
async fn test_differential_fetch_skips_unchanged_mods() {
    let root = tempdir().unwrap();
    let root_path = Utf8PathBuf::from_path_buf(root.path().to_path_buf()).unwrap();

    // 1. Setup Local State (Simulate previous sync)
    let manifest_store = Arc::new(FileManifestStore::new());
    let local_manifest = Manifest {
        version: "1.0".into(),
        mods: vec![
            Mod {
                name: "@mod_unchanged".into(),
                checksum: "hash_A".into(),
                files: vec![],
            },
            Mod {
                name: "@mod_changed".into(),
                checksum: "hash_OLD".into(),
                files: vec![],
            },
        ],
    };
    manifest_store.save(&root_path, &local_manifest).unwrap();

    // 2. Setup Remote State
    let srf_calls = Arc::new(Mutex::new(Vec::new()));
    let repo_ext = fleet_core::formats::RepositoryExternal {
        repo_name: "test".into(),
        checksum: "abc".into(),
        required_mods: vec![
            fleet_core::formats::repo::RepoModExternal {
                mod_name: "@mod_unchanged".into(),
                checksum: "hash_A".into(),
                enabled: true,
            },
            fleet_core::formats::repo::RepoModExternal {
                mod_name: "@mod_changed".into(),
                checksum: "hash_NEW".into(),
                enabled: true,
            },
            fleet_core::formats::repo::RepoModExternal {
                mod_name: "@mod_new".into(),
                checksum: "hash_B".into(),
                enabled: true,
            },
        ],
        optional_mods: vec![],
    };

    let remote = MockRemote {
        repo_ext,
        srf_calls: srf_calls.clone(),
    };

    // 3. Init Engine
    let engine = DefaultSyncEngine::with_components(
        Box::new(remote),
        Box::new(fleet_pipeline::sync::local::DefaultLocalStateProvider::new(
            None,
            manifest_store.clone(),
        )),
        Box::new(fleet_pipeline::sync::execute::DefaultPlanExecutor::new(
            reqwest::Client::new(),
        )),
        manifest_store,
        Arc::new(fleet_pipeline::sync::storage::FileRepoSummaryStore::new()),
    );

    let req = SyncRequest {
        repo_url: "http://fake".into(),
        local_root: root_path,
        mode: SyncMode::FastCheck,
        options: SyncOptions::default(),
        profile_id: None,
    };

    // 4. Run
    let _ = engine.fetch_remote_state(&req).await.unwrap();

    // 5. Verify
    let calls = srf_calls.lock().unwrap();
    assert!(
        calls.contains(&"@mod_changed".to_string()),
        "Must fetch changed mod"
    );
    assert!(
        calls.contains(&"@mod_new".to_string()),
        "Must fetch new mod"
    );
    assert!(
        !calls.contains(&"@mod_unchanged".to_string()),
        "Must NOT fetch unchanged mod"
    );
}
