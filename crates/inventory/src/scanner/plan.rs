use crate::hash::{hash_file_record, mix64};
use crate::scanner::walk::{WalkItem, WalkStream};
use crate::{Error, FolderStamp, LocalStateMetrics, ScanPolicy};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(crate) struct PlannedWalkItem {
    pub item: WalkItem,
}

#[derive(Debug, Clone)]
pub(crate) struct ScanPlan {
    pub seen_paths: Vec<String>,
    pub scan_items: Vec<PlannedWalkItem>,
    pub current_stamp: FolderStamp,
    pub files_seen: u64,
    pub files_needing_scan: u64,
    pub hash_files_total: u64,
    pub hash_bytes_total: u64,
    pub db_files_count: u64,
    pub saw_removed: bool,
    pub saw_new_or_modified: bool,
    pub last_stamp: Option<FolderStamp>,
}

impl ScanPlan {
    pub(crate) fn is_no_changes(&self) -> bool {
        let stamp_matches_last = self.last_stamp.as_ref().is_some_and(|prev| {
            prev.algo == self.current_stamp.algo
                && prev.hash64 == self.current_stamp.hash64
                && prev.file_count == self.current_stamp.file_count
                && prev.total_bytes == self.current_stamp.total_bytes
        });

        !self.saw_new_or_modified
            && !self.saw_removed
            && self.files_needing_scan == 0
            && self.files_seen == self.db_files_count
            && self.last_stamp.is_some()
            && stamp_matches_last
    }

    pub(crate) fn needs_stamp_refresh_only(&self) -> bool {
        !self.saw_new_or_modified
            && !self.saw_removed
            && self.files_needing_scan == 0
            && self.files_seen == self.db_files_count
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PlanRequest {
    pub root_path: std::path::PathBuf,
    pub policy: ScanPolicy,
    pub persisted_index: Option<HashMap<String, u64>>,
    pub persisted_metrics: LocalStateMetrics,
    pub last_stamp: Option<FolderStamp>,
    pub delta: bool,
    pub delta_index_cache: bool,
}

pub(crate) fn build_scan_plan(
    request: PlanRequest,
    mut on_item: impl FnMut(u64, u64, u64) -> Result<(), Error>,
) -> Result<ScanPlan, Error> {
    let mut index = request.persisted_index.unwrap_or_default();
    let mut walk = WalkStream::new(&request.root_path, &request.policy)?;
    let mut stamp = StampAccumulator::new();

    let mut files_seen = 0u64;
    let mut files_needing_scan = 0u64;
    let mut hash_files_total = 0u64;
    let mut hash_bytes_total = 0u64;
    let mut saw_new_or_modified = false;
    let mut seen_paths = Vec::new();
    let mut scan_items = Vec::new();

    loop {
        match walk.next() {
            Some(Ok(item)) => {
                files_seen += 1;
                stamp.update(&item);
                seen_paths.push(item.rel_path.clone());

                let needs_scan = if request.delta {
                    if request.delta_index_cache {
                        match index.remove(&item.rel_path) {
                            None => {
                                saw_new_or_modified = true;
                                true
                            }
                            Some(prev_len) => {
                                let changed = prev_len != item.len;
                                if changed {
                                    saw_new_or_modified = true;
                                }
                                changed
                            }
                        }
                    } else {
                        true
                    }
                } else {
                    true
                };

                if needs_scan {
                    files_needing_scan = files_needing_scan.saturating_add(1);
                    hash_files_total = hash_files_total.saturating_add(1);
                    hash_bytes_total = hash_bytes_total.saturating_add(item.len);
                    scan_items.push(PlannedWalkItem { item });
                }

                on_item(files_seen, 0, 0)?;
            }
            Some(Err(err)) => return Err(err),
            None => break,
        }
    }

    let saw_removed = request.delta && request.delta_index_cache && !index.is_empty();

    Ok(ScanPlan {
        seen_paths,
        scan_items,
        current_stamp: stamp.finish(),
        files_seen,
        files_needing_scan,
        hash_files_total,
        hash_bytes_total,
        db_files_count: request.persisted_metrics.files_count,
        saw_removed,
        saw_new_or_modified,
        last_stamp: request.last_stamp,
    })
}

#[derive(Debug, Clone, Copy)]
struct StampAccumulator {
    hash64: u64,
    file_count: u64,
    total_bytes: u64,
}

impl StampAccumulator {
    fn new() -> Self {
        Self {
            hash64: 0,
            file_count: 0,
            total_bytes: 0,
        }
    }

    fn update(&mut self, item: &WalkItem) {
        let per = hash_file_record(&item.rel_path, item.len);
        self.hash64 ^= mix64(per);
        self.file_count = self.file_count.saturating_add(1);
        self.total_bytes = self.total_bytes.saturating_add(item.len);
    }

    fn finish(self) -> FolderStamp {
        FolderStamp {
            algo: "quick-v1".to_string(),
            hash64: self.hash64,
            file_count: self.file_count,
            total_bytes: self.total_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{build_scan_plan, PlanRequest};
    use crate::{LocalStateMetrics, RootId, ScanPolicy};

    fn metrics(files_count: u64) -> LocalStateMetrics {
        LocalStateMetrics {
            root_id: RootId(1),
            root_path: "/tmp/root".to_string(),
            files_count,
            files_bytes: 0,
            last_stamp: None,
        }
    }

    #[test]
    fn planning_detects_no_op_clean_case_without_scan_work() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(root.path().join("a.txt"), "alpha").expect("write file");

        let first = build_scan_plan(
            PlanRequest {
                root_path: root.path().to_path_buf(),
                policy: ScanPolicy::default(),
                persisted_index: Some(std::collections::HashMap::new()),
                persisted_metrics: metrics(0),
                last_stamp: None,
                delta: true,
                delta_index_cache: true,
            },
            |_, _, _| Ok(()),
        )
        .expect("initial plan");

        let cached_index = std::collections::HashMap::from([("a.txt".to_string(), 5)]);
        let second = build_scan_plan(
            PlanRequest {
                root_path: root.path().to_path_buf(),
                policy: ScanPolicy::default(),
                persisted_index: Some(cached_index),
                persisted_metrics: metrics(1),
                last_stamp: Some(first.current_stamp.clone()),
                delta: true,
                delta_index_cache: true,
            },
            |_, _, _| Ok(()),
        )
        .expect("steady-state plan");

        assert!(second.is_no_changes());
        assert_eq!(second.files_needing_scan, 0);
    }

    #[test]
    fn planning_detects_stamp_refresh_only_case() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(root.path().join("a.txt"), "alpha").expect("write file");

        let current = build_scan_plan(
            PlanRequest {
                root_path: root.path().to_path_buf(),
                policy: ScanPolicy::default(),
                persisted_index: Some(std::collections::HashMap::new()),
                persisted_metrics: metrics(0),
                last_stamp: None,
                delta: true,
                delta_index_cache: true,
            },
            |_, _, _| Ok(()),
        )
        .expect("initial plan")
        .current_stamp;

        let mut stale = current.clone();
        stale.hash64 ^= 1;

        let cached_index = std::collections::HashMap::from([("a.txt".to_string(), 5)]);
        let plan = build_scan_plan(
            PlanRequest {
                root_path: root.path().to_path_buf(),
                policy: ScanPolicy::default(),
                persisted_index: Some(cached_index),
                persisted_metrics: metrics(1),
                last_stamp: Some(stale),
                delta: true,
                delta_index_cache: true,
            },
            |_, _, _| Ok(()),
        )
        .expect("refresh plan");

        assert!(!plan.is_no_changes());
        assert!(plan.needs_stamp_refresh_only());
        assert_eq!(plan.files_needing_scan, 0);
    }
}
