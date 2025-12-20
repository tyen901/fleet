use anyhow::Result;
use async_trait::async_trait;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use sync_engine::events::NoopSink;
use sync_engine::planner::PlanBuilder;
use sync_engine::remote::{RemoteCapabilities, RemoteRepo, RemoteStream};
use sync_engine::types::{Checksum, Checksummer, FileEntry, FilePart, ModManifest, RepoSpec, SyncTuning};

struct TestChecksummer;

impl Checksummer for TestChecksummer {
    fn algorithm_name(&self) -> &str {
        "blake3"
    }

    fn hash_bytes(&self, data: &[u8]) -> Result<Vec<u8>> {
        Ok(blake3::hash(data).as_bytes().to_vec())
    }

    fn hash_file(&self, path: &Path) -> Result<Vec<u8>> {
        let mut file = fs::File::open(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        self.hash_bytes(&buf)
    }

    fn hash_range(&self, path: &Path, offset: u64, len: u64) -> Result<Vec<u8>> {
        let mut file = fs::File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len as usize];
        file.read_exact(&mut buf)?;
        self.hash_bytes(&buf)
    }
}

#[derive(Clone)]
struct TestRemote {
    manifest: ModManifest,
    supports_ranges: bool,
}

#[async_trait]
impl RemoteRepo for TestRemote {
    async fn capabilities(&self) -> Result<RemoteCapabilities> {
        Ok(RemoteCapabilities {
            supports_ranges: self.supports_ranges,
        })
    }

    async fn fetch_repo_spec(&self) -> Result<RepoSpec> {
        Ok(RepoSpec {
            mods: vec![sync_engine::types::ModSpec {
                mod_id: self.manifest.mod_id.clone(),
                version: self.manifest.version.clone(),
            }],
        })
    }

    async fn fetch_mod_manifest(&self, _mod_id: &str) -> Result<ModManifest> {
        Ok(self.manifest.clone())
    }

    async fn fetch_file(&self, _mod_id: &str, _rel_path: &str) -> Result<RemoteStream> {
        anyhow::bail!("fetch_file not expected in this test")
    }

    async fn fetch_range(
        &self,
        _mod_id: &str,
        _rel_path: &str,
        _offset: u64,
        _len: u64,
    ) -> Result<RemoteStream> {
        anyhow::bail!("fetch_range not expected in this test")
    }
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let base = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    let p = base.join(format!("{prefix}_{pid}_{nanos}"));
    fs::create_dir_all(&p).unwrap();
    p
}

#[tokio::test]
async fn planner_skips_up_to_date_files() -> Result<()> {
    let checkout_root = unique_temp_dir("sync_engine_planner_skip_test");
    tokio::fs::create_dir_all(checkout_root.join(".fleet")).await?;

    let mod_id = "mod1";
    let rel_path = "file.bin";
    let abs_path = checkout_root.join(mod_id).join(rel_path);
    tokio::fs::create_dir_all(abs_path.parent().unwrap()).await?;

    let content: Vec<u8> = (0u8..=255u8).collect();
    tokio::fs::write(&abs_path, &content).await?;

    let checksummer = TestChecksummer;
    let parts: Vec<(u64, u64)> = vec![(0, 64), (64, 64), (128, 64), (192, 64)];

    let mut file_parts = Vec::new();
    for (off, len) in parts {
        let bytes = checksummer.hash_range(&abs_path, off, len)?;
        file_parts.push(FilePart {
            offset: off,
            len,
            checksum: Checksum { bytes },
        });
    }
    let file_checksum = checksummer.hash_file(&abs_path)?;

    let fe = FileEntry {
        rel_path: rel_path.to_string(),
        size: content.len() as u64,
        file_checksum: Checksum {
            bytes: file_checksum,
        },
        parts: file_parts,
    };

    let remote = Arc::new(TestRemote {
        manifest: ModManifest {
            mod_id: mod_id.to_string(),
            version: "1".to_string(),
            files: vec![fe],
        },
        supports_ranges: true,
    });

    let mut idx =
        sync_engine::index::LocalIndex::open_or_recover(&checkout_root, Arc::new(NoopSink))?;

    let plan = PlanBuilder::new(
        remote,
        checkout_root.clone(),
        vec![mod_id.to_string()],
        SyncTuning::default(),
        Arc::new(checksummer),
        Arc::new(NoopSink),
    )
    .build(
        sync_engine::types::RepoSpec {
            mods: vec![sync_engine::types::ModSpec {
                mod_id: mod_id.to_string(),
                version: "1".to_string(),
            }],
        },
        &mut idx,
        true,
    )
    .await?;

    assert_eq!(plan.total_bytes, 0);
    assert!(plan.ops.is_empty(), "expected no transfer ops for up-to-date file");
    Ok(())
}

