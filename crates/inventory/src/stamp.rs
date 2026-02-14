use crate::{Error, FolderStamp, Result, ScanPolicy};
use std::path::Path;
use walkdir::WalkDir;

/// Fast metadata-only stamp for dirty detection.
/// Regular files only; policy-filtered; order-independent aggregation.
pub fn compute_stamp(root: &Path, policy: &ScanPolicy) -> Result<FolderStamp> {
    if !root.exists() {
        return Err(Error::InvalidInput(format!(
            "root does not exist: {}",
            root.display()
        )));
    }
    if !root.is_dir() {
        return Err(Error::InvalidInput(format!(
            "root is not a directory: {}",
            root.display()
        )));
    }

    let mut agg_hash: u64 = 0;
    let mut file_count: u64 = 0;
    let mut total_bytes: u64 = 0;

    let policy = policy.clone();
    let iter = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| stamp_filter_entry(&policy, e));

    for entry in iter {
        let entry = entry?;
        if entry.file_type().is_symlink() || !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let rel = match policy.rel_path_forward_slash(root, path) {
            Ok(s) => s,
            Err(Error::InvalidInput(s)) if s == "SKIP" => continue,
            Err(e) => return Err(e),
        };

        if !policy.should_include_rel_path(&rel, false) {
            continue;
        }

        let len = std::fs::symlink_metadata(entry.path())?.len();
        let per = hash_file_record(&rel, len);
        agg_hash ^= mix64(per);

        file_count += 1;
        total_bytes = total_bytes.saturating_add(len);
    }

    Ok(FolderStamp {
        algo: "quick-v1".to_string(),
        hash64: agg_hash,
        file_count,
        total_bytes,
    })
}

fn stamp_filter_entry(policy: &ScanPolicy, e: &walkdir::DirEntry) -> bool {
    if e.depth() == 0 {
        return true;
    }

    if !policy.include_hidden {
        if let Some(name) = e.file_name().to_str() {
            if name.starts_with('.') && name != "." && name != ".." {
                return false;
            }
        }
    }

    true
}

fn hash_file_record(rel: &str, len: u64) -> u64 {
    const OFFSET: u64 = 14695981039346656037;
    const PRIME: u64 = 1099511628211;

    let mut h = OFFSET;
    for &b in rel.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h ^= 0xFF;
    h = h.wrapping_mul(PRIME);

    for &b in len.to_le_bytes().as_slice() {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}
