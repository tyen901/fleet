use bytes::Bytes;
use camino::Utf8Path;
use futures_util::stream;
use manifest_types::{FileManifest, Md5Digest, ModManifest, PartManifest, RepoSpec};
use md5::{Digest, Md5};
use relative_path::RelativePath;
use remote_core::{ByteStream, RemoteError, RemoteSession};
use std::collections::HashMap;
use std::sync::Arc;
use sync_apply::{apply_plan, ApplyError, ApplyOptions};
use sync_plan::{Op, SyncPlan};
use tokio::sync::Mutex;

fn md5_of(bytes: &[u8]) -> Md5Digest {
    let mut ctx = Md5::new();
    ctx.update(bytes);
    Md5Digest::from_bytes(ctx.finalize().into())
}

fn chunk_bytes(mut input: &[u8], pattern: &[usize]) -> Vec<Bytes> {
    if input.is_empty() {
        return vec![];
    }

    let mut out = Vec::new();
    let mut i = 0usize;
    while !input.is_empty() {
        let step = pattern[i % pattern.len()].max(1);
        let take = step.min(input.len());
        out.push(Bytes::copy_from_slice(&input[..take]));
        input = &input[take..];
        i += 1;
    }
    out
}

#[derive(Default, Debug)]
struct CallLog {
    fetch_file: usize,
    fetch_range: Vec<(String, String, u64, u64)>,
}

type RangeOverrides = Arc<Mutex<HashMap<(u64, u64), Vec<u8>>>>;

#[derive(Clone)]
struct FakeSession {
    repo: RepoSpec,
    remote: Arc<Vec<u8>>,
    range_overrides: RangeOverrides,
    file_override: Arc<Mutex<Option<Vec<u8>>>>,
    calls: Arc<Mutex<CallLog>>,
    chunk_pattern: Arc<Vec<usize>>,
}

impl FakeSession {
    fn new(remote: Vec<u8>) -> Self {
        Self {
            repo: RepoSpec {
                repo_name: "test".to_string(),
                checksum: "0000000000000000000000000000000000000000".to_string(),
                required_mods: vec![],
                optional_mods: vec![],
                client_parameters: "".to_string(),
                repo_basic_authentication: None,
                version: "0".to_string(),
                servers: vec![],
            },
            remote: Arc::new(remote),
            range_overrides: Arc::new(Mutex::new(HashMap::new())),
            file_override: Arc::new(Mutex::new(None)),
            calls: Arc::new(Mutex::new(CallLog::default())),
            chunk_pattern: Arc::new(vec![7, 13, 5, 29, 3, 11]),
        }
    }

    async fn set_range_override(&self, start: u64, length: u64, data: Vec<u8>) {
        self.range_overrides
            .lock()
            .await
            .insert((start, length), data);
    }

    #[allow(dead_code)]
    async fn set_file_override(&self, data: Vec<u8>) {
        *self.file_override.lock().await = Some(data);
    }
}

#[async_trait::async_trait]
impl RemoteSession for FakeSession {
    fn repo_spec(&self) -> &RepoSpec {
        &self.repo
    }

    async fn fetch_mod_manifest(&self, _mod_name: &str) -> Result<ModManifest, RemoteError> {
        Err(RemoteError::Protocol("not used in tests".into()))
    }

    async fn fetch_range(
        &self,
        mod_name: &str,
        rel_path: &RelativePath,
        start: u64,
        length: u64,
    ) -> Result<ByteStream, RemoteError> {
        self.calls.lock().await.fetch_range.push((
            mod_name.to_string(),
            rel_path.as_str().to_string(),
            start,
            length,
        ));

        let override_bytes = self
            .range_overrides
            .lock()
            .await
            .get(&(start, length))
            .cloned();

        let bytes = if let Some(b) = override_bytes {
            b
        } else {
            let end = (start + length) as usize;
            let start = start as usize;
            self.remote[start..end].to_vec()
        };

        let chunks = chunk_bytes(&bytes, &self.chunk_pattern);
        let s = stream::iter(chunks.into_iter().map(Ok::<Bytes, RemoteError>));

        Ok(Box::pin(s))
    }

    async fn fetch_file(
        &self,
        mod_name: &str,
        rel_path: &RelativePath,
    ) -> Result<ByteStream, RemoteError> {
        let mut calls = self.calls.lock().await;
        calls.fetch_file += 1;
        drop(calls);

        let override_bytes = self.file_override.lock().await.clone();
        let bytes = override_bytes.unwrap_or_else(|| self.remote.as_ref().clone());

        let _ = (mod_name, rel_path);

        let chunks = chunk_bytes(&bytes, &self.chunk_pattern);
        let s = stream::iter(chunks.into_iter().map(Ok::<Bytes, RemoteError>));
        Ok(Box::pin(s))
    }
}

fn build_manifest_parts(remote: &[u8], layout: &[(u64, u64)]) -> Vec<PartManifest> {
    layout
        .iter()
        .map(|(start, length)| {
            let s = *start as usize;
            let e = (*start + *length) as usize;
            PartManifest {
                start: *start,
                length: *length,
                checksum: md5_of(&remote[s..e]),
            }
        })
        .collect()
}

fn build_plan(mod_name: &str, rel_path: &str, remote: &[u8], parts: Vec<PartManifest>) -> SyncPlan {
    let file = FileManifest {
        path: relative_path::RelativePathBuf::from(rel_path),
        length: remote.len() as u64,
        checksum: manifest_types::file_checksum_from_parts(&parts),
        parts,
    };

    SyncPlan {
        ops: vec![Op::EnsureFileFromParts {
            mod_name: mod_name.to_string(),
            file,
        }],
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn happy_full_download_when_missing() {
    let temp = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp.path()).unwrap();

    let remote: Vec<u8> = (0..2048).map(|i| (i % 251) as u8).collect();
    let parts = build_manifest_parts(&remote, &[(0, 512), (512, 1024), (1536, 512)]);
    let plan = build_plan("@mod", "addons/data.bin", &remote, parts);

    let session = FakeSession::new(remote.clone());

    let opts = ApplyOptions {
        max_concurrent_files: 8,
        max_concurrent_range_requests: 32,
        full_download_part_threshold: 1,
        full_download_byte_ratio_threshold: 0.0,
        ..ApplyOptions::default()
    };

    apply_plan(&session, checkout, &plan, opts).await.unwrap();

    let out_path = checkout.join("@mod").join("addons/data.bin");
    let got = tokio::fs::read(out_path.as_std_path()).await.unwrap();
    assert_eq!(got, remote);

    let calls = session.calls.lock().await;
    assert_eq!(calls.fetch_file, 1);
    assert_eq!(calls.fetch_range.len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deletes_files_from_plan() {
    let temp = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp.path()).unwrap();

    let mod_root = checkout.join("@mod");
    tokio::fs::create_dir_all(mod_root.as_std_path())
        .await
        .unwrap();
    let target = mod_root.join("old.bin");
    tokio::fs::write(target.as_std_path(), b"stale")
        .await
        .unwrap();

    let plan = SyncPlan {
        ops: vec![Op::DeleteFile {
            mod_name: "@mod".to_string(),
            rel_path: relative_path::RelativePathBuf::from("old.bin"),
        }],
    };

    let session = FakeSession::new(vec![]);
    apply_plan(&session, checkout, &plan, ApplyOptions::default())
        .await
        .unwrap();

    assert!(!target.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_index_skips_hashing_when_clean() {
    let temp = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp.path()).unwrap();

    let remote: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
    let parts = build_manifest_parts(&remote, &[(0, 1024), (1024, 1024), (2048, 2048)]);
    let plan = build_plan("@mod", "addons/data.bin", &remote, parts);

    let session = FakeSession::new(remote);
    let index = local_index::LocalIndex::open(checkout).await.unwrap();
    let opts = ApplyOptions {
        index: Some(index),
        ..ApplyOptions::default()
    };

    apply_plan(&session, checkout, &plan, opts.clone())
        .await
        .unwrap();

    {
        let mut calls = session.calls.lock().await;
        calls.fetch_file = 0;
        calls.fetch_range.clear();
    }

    apply_plan(&session, checkout, &plan, opts).await.unwrap();

    let calls = session.calls.lock().await;
    assert_eq!(calls.fetch_file, 0);
    assert!(calls.fetch_range.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumes_full_download_from_partial_tmp() {
    let temp = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp.path()).unwrap();

    let remote: Vec<u8> = (0..8192).map(|i| (i % 251) as u8).collect();
    let parts = build_manifest_parts(&remote, &[(0, 2048), (2048, 2048), (4096, 4096)]);
    let expected_md5 = manifest_types::file_checksum_from_parts(&parts);
    let plan = build_plan("@mod", "addons/data.bin", &remote, parts);

    let session = FakeSession::new(remote.clone());
    let opts = ApplyOptions::default();

    let final_path = checkout.join("@mod").join("addons").join("data.bin");
    tokio::fs::create_dir_all(final_path.parent().unwrap().as_std_path())
        .await
        .unwrap();

    let tmp_name = format!(
        ".fleet_tmp_{}_{}.part",
        expected_md5.to_hex_upper(),
        "data.bin"
    );
    let tmp_path = final_path.parent().unwrap().join(tmp_name);
    let partial_len = 2048usize;
    tokio::fs::write(tmp_path.as_std_path(), &remote[..partial_len])
        .await
        .unwrap();

    apply_plan(&session, checkout, &plan, opts).await.unwrap();

    let calls = session.calls.lock().await;
    let expected = [(partial_len as u64, 2048u64), (4096u64, 4096u64)];
    for (start, len) in expected {
        assert!(
            calls
                .fetch_range
                .iter()
                .any(|(_, _, s, l)| *s == start && *l == len),
            "expected resume range request ({start},{len})"
        );
    }

    let on_disk = tokio::fs::read(final_path.as_std_path()).await.unwrap();
    assert_eq!(on_disk, remote);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_download_fails_final_checksum_validation() {
    let temp = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp.path()).unwrap();

    let remote: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
    let parts = build_manifest_parts(&remote, &[(0, 2048), (2048, 2048)]);
    let plan = build_plan("@mod", "addons/data.bin", &remote, parts);

    let session = FakeSession::new(remote.clone());
    session.set_file_override(vec![0u8; remote.len()]).await;

    let err = apply_plan(&session, checkout, &plan, ApplyOptions::default())
        .await
        .unwrap_err();
    match err {
        ApplyError::ChecksumMismatch { .. } => {}
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }

    let final_path = checkout.join("@mod").join("addons").join("data.bin");
    assert!(!final_path.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn happy_patch_only_mismatched_parts() {
    let temp = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp.path()).unwrap();

    let remote: Vec<u8> = (0..4096).map(|i| ((i * 7) % 251) as u8).collect();

    let layout = &[
        (0u64, 333u64),
        (333u64, 1500u64),
        (1833u64, 777u64),
        (2610u64, 1486u64),
    ];
    let parts = build_manifest_parts(&remote, layout);
    let plan = build_plan("@mod", "addons/pbo_like.bin", &remote, parts.clone());

    let mut local = remote.clone();
    for item in local.iter_mut().skip(333usize).take(1500usize) {
        *item ^= 0xAA;
    }
    for item in local.iter_mut().skip(2610usize).take(1486usize) {
        *item = item.wrapping_add(3);
    }

    let out_path = checkout.join("@mod").join("addons/pbo_like.bin");
    tokio::fs::create_dir_all(out_path.parent().unwrap().as_std_path())
        .await
        .unwrap();
    tokio::fs::write(out_path.as_std_path(), &local)
        .await
        .unwrap();

    let session = FakeSession::new(remote.clone());

    let opts = ApplyOptions {
        max_concurrent_files: 8,
        max_concurrent_range_requests: 64,
        full_download_part_threshold: 9999,
        full_download_byte_ratio_threshold: 1.0,
        ..ApplyOptions::default()
    };

    apply_plan(&session, checkout, &plan, opts).await.unwrap();

    let got = tokio::fs::read(out_path.as_std_path()).await.unwrap();
    assert_eq!(got, remote);

    let calls = session.calls.lock().await;
    assert_eq!(calls.fetch_file, 0);
    assert_eq!(calls.fetch_range.len(), 2);

    let mut requested: Vec<(u64, u64)> = calls
        .fetch_range
        .iter()
        .map(|(_, _, s, l)| (*s, *l))
        .collect();
    requested.sort_unstable();

    let mut expected = vec![(333u64, 1500u64), (2610u64, 1486u64)];
    expected.sort_unstable();

    assert_eq!(requested, expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nonhappy_checksum_mismatch_does_not_corrupt_final_file() {
    let temp = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp.path()).unwrap();

    let remote: Vec<u8> = (0..2048).map(|i| ((i * 3) % 251) as u8).collect();
    let layout = &[(0u64, 600u64), (600u64, 800u64), (1400u64, 648u64)];
    let parts = build_manifest_parts(&remote, layout);
    let plan = build_plan("@mod", "addons/file.bin", &remote, parts.clone());

    let mut local = remote.clone();
    for item in local.iter_mut().skip(600usize).take(800usize) {
        *item ^= 0x55;
    }

    let out_path = checkout.join("@mod").join("addons/file.bin");
    tokio::fs::create_dir_all(out_path.parent().unwrap().as_std_path())
        .await
        .unwrap();
    tokio::fs::write(out_path.as_std_path(), &local)
        .await
        .unwrap();

    let session = FakeSession::new(remote.clone());

    let bad = vec![0x11u8; 800];
    session.set_range_override(600, 800, bad).await;

    let opts = ApplyOptions {
        max_concurrent_files: 4,
        max_concurrent_range_requests: 16,
        full_download_part_threshold: 9999,
        full_download_byte_ratio_threshold: 1.0,
        ..ApplyOptions::default()
    };

    let err = apply_plan(&session, checkout, &plan, opts)
        .await
        .unwrap_err();
    match err {
        ApplyError::ChecksumMismatch { .. } => {}
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }

    let got = tokio::fs::read(out_path.as_std_path()).await.unwrap();
    assert_eq!(got, local);

    let dir = out_path.parent().unwrap().as_std_path();
    let leftovers: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(".fleet_tmp_"))
        .collect();

    assert!(
        leftovers.is_empty(),
        "expected no temp leftovers, found: {leftovers:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nonhappy_short_download_is_error_and_final_is_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let checkout = Utf8Path::from_path(temp.path()).unwrap();

    let remote: Vec<u8> = (0..1500).map(|i| (i % 251) as u8).collect();
    let layout = &[(0u64, 500u64), (500u64, 500u64), (1000u64, 500u64)];
    let parts = build_manifest_parts(&remote, layout);
    let plan = build_plan("@mod", "addons/file.bin", &remote, parts.clone());

    let mut local = remote.clone();
    for item in local.iter_mut().skip(1000usize).take(500usize) {
        *item ^= 0xF0;
    }

    let out_path = checkout.join("@mod").join("addons/file.bin");
    tokio::fs::create_dir_all(out_path.parent().unwrap().as_std_path())
        .await
        .unwrap();
    tokio::fs::write(out_path.as_std_path(), &local)
        .await
        .unwrap();

    let session = FakeSession::new(remote.clone());

    session
        .set_range_override(1000, 500, vec![0xABu8; 100])
        .await;

    let opts = ApplyOptions {
        max_concurrent_files: 4,
        max_concurrent_range_requests: 16,
        full_download_part_threshold: 9999,
        full_download_byte_ratio_threshold: 1.0,
        ..ApplyOptions::default()
    };

    let err = apply_plan(&session, checkout, &plan, opts)
        .await
        .unwrap_err();
    match err {
        ApplyError::ShortDownload { .. } => {}
        other => panic!("expected ShortDownload, got {other:?}"),
    }

    let got = tokio::fs::read(out_path.as_std_path()).await.unwrap();
    assert_eq!(got, local);
}
