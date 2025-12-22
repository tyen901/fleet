#[cfg(test)]
mod tests {
    use crate::apply::{apply_ops, ApplyOptions};
    use crate::ports::EventSink;
    use crate::ports::FilePart;
    use crate::manifest::{ValidatedFileEntry, ValidatedModManifest};
    use crate::plan::RepairStrategy;
    use crate::test_support::{MockRemoteRepo, TestSink};
    use crate::model::{Checksummer, Durability, RepairRequest, RepairTuning};
    use bytes::Bytes;
    use std::collections::HashMap;
    use std::fs;
    use std::io::{Read, Seek, SeekFrom};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    fn fnv1a64(data: &[u8]) -> [u8; 8] {
        let mut hash: u64 = 0xcbf29ce484222325;
        for &b in data {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash.to_le_bytes()
    }

    #[derive(Clone)]
    struct TestChecksummer;

    impl Checksummer for TestChecksummer {
        fn algorithm_name(&self) -> &str {
            "fnv1a64"
        }

        fn hash_file(&self, path: &Path) -> anyhow::Result<Vec<u8>> {
            let mut f = fs::File::open(path)?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            Ok(fnv1a64(&buf).to_vec())
        }

        fn hash_range(&self, path: &Path, offset: u64, len: u64) -> anyhow::Result<Vec<u8>> {
            let mut f = fs::File::open(path)?;
            f.seek(SeekFrom::Start(offset))?;
            let mut buf = vec![0u8; len as usize];
            f.read_exact(&mut buf)?;
            Ok(fnv1a64(&buf).to_vec())
        }
    }

    fn make_parts(bytes: &[u8], part_size: usize) -> Vec<FilePart> {
        let mut out = Vec::new();
        let mut offset: u64 = 0;
        while (offset as usize) < bytes.len() {
            let start = offset as usize;
            let end = (start + part_size).min(bytes.len());
            out.push(FilePart {
                offset,
                len: (end - start) as u64,
                checksum: fnv1a64(&bytes[start..end]).to_vec(),
            });
            offset += (end - start) as u64;
        }
        out
    }

    fn make_manifest(
        mod_id: &str,
        rel_path: &str,
        bytes: &[u8],
        part_size: usize,
    ) -> ValidatedModManifest {
        ValidatedModManifest {
            mod_id: mod_id.to_string(),
            files: vec![ValidatedFileEntry {
                rel_path: rel_path.to_string(),
                size: bytes.len() as u64,
                file_checksum: fnv1a64(bytes).to_vec(),
                parts: make_parts(bytes, part_size),
            }],
        }
    }

    fn write_local_file(root: &Path, mod_id: &str, rel_path: &str, bytes: &[u8]) -> PathBuf {
        let mod_root = root.join(mod_id);
        fs::create_dir_all(&mod_root).unwrap();
        let abs = mod_root.join(rel_path);
        fs::write(&abs, bytes).unwrap();
        abs
    }

    #[tokio::test]
    async fn patch_coalesces_across_small_gap_into_single_range_request() {
        let file_size = 8 * 1024;
        let part_size = 512;

        let remote_bytes: Vec<u8> = (0..file_size).map(|i| (i % 251) as u8).collect();
        let manifest = make_manifest("m", "a.bin", &remote_bytes, part_size);

        let mut local_bytes = remote_bytes.clone();
        local_bytes[10] ^= 0xFF;
        local_bytes[1030] ^= 0xFF;

        let tmp = tempfile::tempdir().unwrap();
        write_local_file(tmp.path(), "m", "a.bin", &local_bytes);

        let tuning = RepairTuning {
            patch_merge_gap_bytes: 512,
            patch_min_range_bytes: 0,
            patch_max_bad_ratio: 1.0,
            patch_max_fetch_ratio: 1.0,
            patch_max_range_requests: Some(64),
            durability: Durability::BestEffort,
            ..Default::default()
        };

        let checksummer: Arc<dyn Checksummer> = Arc::new(TestChecksummer);
        let (plan, _hints) = crate::plan::plan_mod(
            tmp.path(),
            &manifest,
            &HashMap::new(),
            true,
            &tuning,
            checksummer.as_ref(),
        )
        .unwrap();

        let op = plan.ops.into_iter().next().unwrap();
        assert!(matches!(op.target.strategy, RepairStrategy::Patch));
        assert_eq!(op.target.parts_to_fetch.len(), 1);

        let remote = Arc::new(MockRemoteRepo::new(1024).with_file(
            "m",
            "a.bin",
            Bytes::from(remote_bytes.clone()),
        ));

        let req = RepairRequest {
            repo_name: "r".to_string(),
            checkout_root: tmp.path().to_path_buf(),
            enabled_mods: vec!["m".to_string()],
            tuning,
        };

        let sink: Arc<dyn EventSink> = Arc::new(TestSink::new());
        apply_ops(
            vec![op],
            &req.checkout_root,
            remote.clone(),
            checksummer.clone(),
            &req.tuning,
            sink.as_ref(),
            &tokio_util::sync::CancellationToken::new(),
            ApplyOptions {
                supports_ranges: true,
            },
        )
        .await
        .unwrap();

        let calls = remote.range_calls();
        assert_eq!(calls.len(), 1);
        let (_m, _p, off, len) = &calls[0];
        assert_eq!(*off, 0);
        assert_eq!(*len, 1536);

        let final_bytes = fs::read(tmp.path().join("m").join("a.bin")).unwrap();
        assert_eq!(final_bytes, remote_bytes);
    }

    #[tokio::test]
    async fn patch_enforces_min_range_size_by_expanding_request() {
        let file_size = 8 * 1024;
        let part_size = 512;

        let remote_bytes: Vec<u8> = (0..file_size).map(|i| (i % 239) as u8).collect();
        let manifest = make_manifest("m", "b.bin", &remote_bytes, part_size);

        let mut local_bytes = remote_bytes.clone();
        local_bytes[4096 + 3] ^= 0xAA;

        let tmp = tempfile::tempdir().unwrap();
        write_local_file(tmp.path(), "m", "b.bin", &local_bytes);

        let tuning = RepairTuning {
            patch_merge_gap_bytes: 0,
            patch_min_range_bytes: 2048,
            patch_max_bad_ratio: 1.0,
            patch_max_fetch_ratio: 1.0,
            patch_max_range_requests: Some(64),
            ..Default::default()
        };

        let checksummer: Arc<dyn Checksummer> = Arc::new(TestChecksummer);
        let (plan, _hints) = crate::plan::plan_mod(
            tmp.path(),
            &manifest,
            &HashMap::new(),
            true,
            &tuning,
            checksummer.as_ref(),
        )
        .unwrap();
        let op = plan.ops.into_iter().next().unwrap();
        assert!(matches!(op.target.strategy, RepairStrategy::Patch));
        assert_eq!(op.target.parts_to_fetch.len(), 1);

        assert_eq!(op.target.parts_to_fetch[0].offset, 3072);
        assert_eq!(op.target.parts_to_fetch[0].len, 2048);

        let remote = Arc::new(MockRemoteRepo::new(1024).with_file(
            "m",
            "b.bin",
            Bytes::from(remote_bytes.clone()),
        ));
        let req = RepairRequest {
            repo_name: "r".to_string(),
            checkout_root: tmp.path().to_path_buf(),
            enabled_mods: vec!["m".to_string()],
            tuning,
        };

        let sink: Arc<dyn EventSink> = Arc::new(TestSink::new());
        apply_ops(
            vec![op],
            &req.checkout_root,
            remote.clone(),
            checksummer.clone(),
            &req.tuning,
            sink.as_ref(),
            &tokio_util::sync::CancellationToken::new(),
            ApplyOptions {
                supports_ranges: true,
            },
        )
        .await
        .unwrap();

        let calls = remote.range_calls();
        assert_eq!(calls.len(), 1);
        let (_m, _p, off, len) = &calls[0];
        assert_eq!(*off, 3072);
        assert_eq!(*len, 2048);

        let final_bytes = fs::read(tmp.path().join("m").join("b.bin")).unwrap();
        assert_eq!(final_bytes, remote_bytes);
    }

    #[test]
    fn planner_falls_back_to_full_if_min_range_forces_near_full_download() {
        let file_size = 4096;
        let part_size = 512;

        let remote_bytes: Vec<u8> = (0..file_size).map(|i| (i % 199) as u8).collect();
        let manifest = make_manifest("m", "c.bin", &remote_bytes, part_size);

        let mut local_bytes = remote_bytes.clone();
        local_bytes[7] ^= 0x11;

        let tmp = tempfile::tempdir().unwrap();
        write_local_file(tmp.path(), "m", "c.bin", &local_bytes);

        let tuning = RepairTuning {
            patch_merge_gap_bytes: 0,
            patch_min_range_bytes: 4096,
            patch_max_bad_ratio: 1.0,
            patch_max_fetch_ratio: 0.75,
            patch_max_range_requests: Some(64),
            ..Default::default()
        };

        let checksummer: Arc<dyn Checksummer> = Arc::new(TestChecksummer);
        let (plan, _hints) = crate::plan::plan_mod(
            tmp.path(),
            &manifest,
            &HashMap::new(),
            true,
            &tuning,
            checksummer.as_ref(),
        )
        .unwrap();
        let op = plan.ops.into_iter().next().unwrap();
        assert!(matches!(op.target.strategy, RepairStrategy::Full));
    }
}
