use crate::scanner::apply::{apply_scan_results, refresh_scan_stamp};
use crate::scanner::config::ScanObserver;
use crate::scanner::exec::execute_scan_plan;
use crate::scanner::plan::{build_scan_plan, PlanRequest};
use crate::scanner::{ScanProgress, ScanStage, ScannerConfig, SyncMode, SyncRequest, SyncResult};
use crate::{Error, InventoryDb};
use std::collections::HashMap;

pub struct Scanner {
    db: InventoryDb,
    cfg: ScannerConfig,
}

impl Scanner {
    pub fn new(db: InventoryDb, cfg: ScannerConfig) -> Self {
        Self { db, cfg }
    }

    pub fn db(&self) -> &InventoryDb {
        &self.db
    }

    pub fn sync_root(&self, req: SyncRequest) -> Result<SyncResult, Error> {
        self.db.init()?; // safe repeatedly

        let observer = self.cfg.observer();
        let behavior = self.cfg.behavior();
        let runtime = self.cfg.runtime();
        let mut prog = ScanProgress {
            stage: ScanStage::Planning,
            ..Default::default()
        };
        emit_progress(&observer, &prog);

        if is_cancelled(&observer) {
            prog.stage = ScanStage::Cancelled;
            emit_progress(&observer, &prog);
            return Err(Error::Cancelled);
        }

        let root_path = req.root_path;
        let root_path_ref = root_path.as_path();

        let inv_id = self.db.get_or_create_inventory(&req.inventory_name)?;
        let root_id = self
            .db
            .get_or_create_root(inv_id, root_path.to_string_lossy().as_ref())?;

        let last_stamp = self.db.get_last_stamp(root_id)?;
        let metrics = self.db.metrics(root_id)?;

        let mut index: HashMap<String, u64> = HashMap::new();
        if behavior.delta && behavior.delta_index_cache {
            for f in self.db.export_file_index(root_id)? {
                index.insert(f.rel_path, f.length);
            }
        }
        prog.stage = ScanStage::Walking;
        emit_progress(&observer, &prog);

        let plan = build_scan_plan(
            PlanRequest {
                root_path: root_path_ref.to_path_buf(),
                policy: behavior.policy.clone(),
                persisted_index: if behavior.delta && behavior.delta_index_cache {
                    Some(index)
                } else {
                    None
                },
                persisted_metrics: metrics.clone(),
                last_stamp,
                delta: behavior.delta,
                delta_index_cache: behavior.delta_index_cache,
            },
            |files_seen, files_scanned, bytes_scanned| {
                if is_cancelled(&observer) {
                    prog.stage = ScanStage::Cancelled;
                    prog.files_seen = files_seen;
                    prog.files_scanned = files_scanned;
                    prog.bytes_scanned = bytes_scanned;
                    emit_progress(&observer, &prog);
                    return Err(Error::Cancelled);
                }

                prog.files_seen = files_seen;
                prog.files_scanned = files_scanned;
                prog.bytes_scanned = bytes_scanned;
                emit_progress(&observer, &prog);
                Ok(())
            },
        )?;

        prog.files_total = plan.hash_files_total;
        prog.bytes_total = plan.hash_bytes_total;
        prog.files_seen = plan.files_seen;
        prog.files_scanned = 0;
        prog.bytes_scanned = 0;
        emit_progress(&observer, &prog);

        if plan.is_no_changes() {
            prog.stage = ScanStage::Finished;
            emit_progress(&observer, &prog);
            return Ok(SyncResult {
                root_id,
                mode: SyncMode::SkippedClean,
                files_seen: plan.files_seen,
                files_scanned: 0,
                bytes_scanned: 0,
            });
        }

        if plan.needs_stamp_refresh_only() {
            prog.stage = ScanStage::UpdatingDb;
            emit_progress(&observer, &prog);

            let result = refresh_scan_stamp(
                &self.db,
                root_id,
                plan.current_stamp.clone(),
                plan.files_seen,
            )?;

            prog.stage = ScanStage::Finished;
            emit_progress(&observer, &prog);
            return Ok(result);
        }

        let exec_result = execute_scan_plan(plan.clone(), &runtime, &observer)?;
        prog.stage = ScanStage::UpdatingDb;
        prog.files_seen = plan.files_seen;
        prog.files_scanned = exec_result.scanned_files;
        prog.bytes_scanned = exec_result.scanned_bytes;
        emit_progress(&observer, &prog);

        let result = apply_scan_results(
            &self.db,
            root_id,
            &plan,
            exec_result,
            plan.current_stamp.clone(),
        )?;

        prog.stage = ScanStage::Finished;
        emit_progress(&observer, &prog);

        Ok(result)
    }
}

fn emit_progress(observer: &ScanObserver, progress: &ScanProgress) {
    if let Some(cb) = &observer.progress {
        cb(progress.clone());
    }
}

fn is_cancelled(observer: &ScanObserver) -> bool {
    observer
        .cancel
        .as_ref()
        .map(|cancel| cancel())
        .unwrap_or(false)
}
