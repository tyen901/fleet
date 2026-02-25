use crate::scanner::{
    CancelFn, ScanProgress, ScanStage, ScannerConfig, SyncMode, SyncRequest, SyncResult,
};
use crate::{
    hash::{hash_file_record, mix64},
    scanner::walk::{WalkItem, WalkStream},
    Error, FileEntry, FolderStamp, InventoryDb, SegmentEntry, UpdateSession,
};
use crossbeam_channel as chan;
use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

fn emit_progress(cfg: &ScannerConfig, p: &ScanProgress) {
    if let Some(cb) = &cfg.progress {
        cb(p.clone());
    }
}

fn is_cancelled(cfg: &ScannerConfig) -> bool {
    cfg.cancel.as_ref().map(|c| c()).unwrap_or(false)
}

struct ProgressReporter {
    last: Instant,
    interval: Duration,
}

impl ProgressReporter {
    fn new(interval: Duration) -> Self {
        Self {
            last: Instant::now() - interval,
            interval,
        }
    }
    fn should_report(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last) >= self.interval {
            self.last = now;
            true
        } else {
            false
        }
    }
}

pub struct Scanner {
    db: InventoryDb,
    cfg: ScannerConfig,
}

struct WorkerPool {
    scan_tx: Option<chan::Sender<WalkItem>>,
    res_rx: chan::Receiver<ScanMsg>,
    handles: Vec<std::thread::JoinHandle<()>>,
}

impl WorkerPool {
    fn new(workers: usize, cap: usize, cancel: Option<CancelFn>) -> Self {
        let (scan_tx, scan_rx) = chan::bounded::<WalkItem>(cap);
        let (res_tx, res_rx) = chan::bounded::<ScanMsg>(cap);

        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let rx = scan_rx.clone();
            let tx = res_tx.clone();
            let cancel = cancel.clone();
            handles.push(thread::spawn(move || worker_loop(rx, tx, cancel)));
        }
        drop(res_tx);

        Self {
            scan_tx: Some(scan_tx),
            res_rx,
            handles,
        }
    }

    fn sender(&self) -> &chan::Sender<WalkItem> {
        self.scan_tx.as_ref().expect("sender closed")
    }

    fn receiver(&self) -> &chan::Receiver<ScanMsg> {
        &self.res_rx
    }

    fn close_sender(&mut self) {
        self.scan_tx.take();
    }

    fn join(mut self) {
        self.close_sender();
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.close_sender();
    }
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

        let mut prog = ScanProgress {
            stage: ScanStage::Planning,
            ..Default::default()
        };
        emit_progress(&self.cfg, &prog);

        if is_cancelled(&self.cfg) {
            prog.stage = ScanStage::Cancelled;
            emit_progress(&self.cfg, &prog);
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
        let db_files_count = metrics.files_count;

        let mut index: HashMap<String, u64> = HashMap::new();
        if self.cfg.delta && self.cfg.delta_index_cache {
            for f in self.db.export_file_index(root_id)? {
                index.insert(f.rel_path, f.length);
            }
        }

        let mut walk = WalkStream::new(root_path_ref, &self.cfg.policy)?;
        let mut files_seen = 0u64;
        let mut files_scanned = 0u64;
        let mut bytes_scanned = 0u64;
        let mut files_needing_scan = 0u64;
        let mut hash_files_total = 0u64;
        let mut hash_bytes_total = 0u64;
        let mut saw_removed = false;
        let mut saw_new_or_modified = false;

        let mut stamp = StampAccumulator::new();
        let mut seen_paths: Vec<String> = Vec::new();
        let mut scan_items: Vec<WalkItem> = Vec::new();

        let mut throttle = ProgressReporter::new(self.cfg.progress_interval);
        prog.stage = ScanStage::Walking;
        emit_progress(&self.cfg, &prog);

        let mut cancelled = false;

        loop {
            if !cancelled && is_cancelled(&self.cfg) {
                cancelled = true;

                prog.stage = ScanStage::Cancelled;
                prog.files_seen = files_seen;
                prog.files_scanned = files_scanned;
                prog.bytes_scanned = bytes_scanned;
                emit_progress(&self.cfg, &prog);
            }

            match walk.next() {
                Some(Ok(item)) => {
                    files_seen += 1;
                    stamp.update(&item);
                    seen_paths.push(item.rel_path.clone());

                    let needs_scan = if self.cfg.delta {
                        if self.cfg.delta_index_cache {
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

                    if needs_scan && !cancelled {
                        files_needing_scan += 1;
                        hash_files_total = hash_files_total.saturating_add(1);
                        hash_bytes_total = hash_bytes_total.saturating_add(item.len);
                        scan_items.push(item);
                    }

                    prog.files_seen = files_seen;
                    prog.files_scanned = files_scanned;
                    prog.bytes_scanned = bytes_scanned;
                    if throttle.should_report() {
                        emit_progress(&self.cfg, &prog);
                    }
                }
                Some(Err(e)) => return Err(e),
                None => break,
            }
        }

        if cancelled {
            emit_progress(&self.cfg, &prog);
            return Err(Error::Cancelled);
        }

        if self.cfg.delta && self.cfg.delta_index_cache && !index.is_empty() {
            saw_removed = true;
        }

        prog.files_total = hash_files_total;
        prog.bytes_total = hash_bytes_total;
        prog.files_seen = files_seen;
        prog.files_scanned = files_scanned;
        prog.bytes_scanned = bytes_scanned;
        emit_progress(&self.cfg, &prog);

        let current_stamp = stamp.finish();
        let stamp_matches_last = last_stamp.as_ref().is_some_and(|prev| {
            prev.algo == current_stamp.algo
                && prev.hash64 == current_stamp.hash64
                && prev.file_count == current_stamp.file_count
                && prev.total_bytes == current_stamp.total_bytes
        });

        let no_changes = !saw_new_or_modified
            && !saw_removed
            && files_needing_scan == 0
            && files_seen == db_files_count
            && last_stamp.is_some()
            && stamp_matches_last;
        if no_changes {
            prog.stage = ScanStage::Finished;
            emit_progress(&self.cfg, &prog);
            return Ok(SyncResult {
                root_id,
                mode: SyncMode::SkippedClean,
                files_seen,
                files_scanned: 0,
                bytes_scanned: 0,
            });
        }

        // Index may already match disk while persisted folder_stamp lags behind. Refresh stamp
        // to keep state coherent and avoid false LocalDrift on follow-up assessments.
        if !saw_new_or_modified
            && !saw_removed
            && files_needing_scan == 0
            && files_seen == db_files_count
        {
            prog.stage = ScanStage::UpdatingDb;
            emit_progress(&self.cfg, &prog);

            let mut session = self.db.begin_update(root_id)?;
            session.set_stamp(current_stamp)?;
            session.commit()?;

            prog.stage = ScanStage::Finished;
            emit_progress(&self.cfg, &prog);
            return Ok(SyncResult {
                root_id,
                mode: SyncMode::DeltaSync,
                files_seen,
                files_scanned: 0,
                bytes_scanned: 0,
            });
        }

        let cap = self.cfg.queue_capacity.max(1);
        let workers = self.cfg.workers.max(1);
        let mut pool = WorkerPool::new(workers, cap, self.cfg.cancel.clone());

        let mut session = self.db.begin_update(root_id)?;
        session.begin_seen_set()?;

        for rel_path in &seen_paths {
            session.mark_seen(rel_path)?;
        }

        prog.stage = ScanStage::Scanning;
        emit_progress(&self.cfg, &prog);

        let mut pending_scans = 0u64;
        for item in &scan_items {
            if !cancelled && is_cancelled(&self.cfg) {
                cancelled = true;
                prog.stage = ScanStage::Cancelled;
                prog.files_seen = files_seen;
                prog.files_scanned = files_scanned;
                prog.bytes_scanned = bytes_scanned;
                emit_progress(&self.cfg, &prog);
            }

            if cancelled {
                pool.join();
                session.rollback()?;
                emit_progress(&self.cfg, &prog);
                return Err(Error::Cancelled);
            }

            loop {
                if !cancelled && is_cancelled(&self.cfg) {
                    cancelled = true;
                    prog.stage = ScanStage::Cancelled;
                    prog.files_seen = files_seen;
                    prog.files_scanned = files_scanned;
                    prog.bytes_scanned = bytes_scanned;
                    emit_progress(&self.cfg, &prog);
                }
                if cancelled {
                    pool.join();
                    session.rollback()?;
                    emit_progress(&self.cfg, &prog);
                    return Err(Error::Cancelled);
                }

                match pool.sender().try_send(item.clone()) {
                    Ok(()) => {
                        pending_scans = pending_scans.saturating_add(1);
                        break;
                    }
                    Err(chan::TrySendError::Full(_)) => {
                        let msg = pool.receiver().recv().map_err(|_| Error::ChannelClosed)?;
                        if let Err(e) = apply_scan_msg(
                            &mut session,
                            msg,
                            &mut files_scanned,
                            &mut bytes_scanned,
                        ) {
                            let _ = session.rollback();
                            pool.join();
                            return Err(e);
                        }
                        pending_scans = pending_scans.saturating_sub(1);
                        prog.files_seen = files_seen;
                        prog.files_scanned = files_scanned;
                        prog.bytes_scanned = bytes_scanned;
                        if throttle.should_report() {
                            emit_progress(&self.cfg, &prog);
                        }
                    }
                    Err(chan::TrySendError::Disconnected(_)) => {
                        let _ = session.rollback();
                        pool.join();
                        return Err(Error::ChannelClosed);
                    }
                }
            }
        }
        pool.close_sender();

        while pending_scans > 0 {
            if !cancelled && is_cancelled(&self.cfg) {
                cancelled = true;
                prog.stage = ScanStage::Cancelled;
                prog.files_seen = files_seen;
                prog.files_scanned = files_scanned;
                prog.bytes_scanned = bytes_scanned;
                emit_progress(&self.cfg, &prog);
            }
            if cancelled {
                pool.join();
                session.rollback()?;
                emit_progress(&self.cfg, &prog);
                return Err(Error::Cancelled);
            }

            let msg = pool.receiver().recv().map_err(|_| Error::ChannelClosed)?;
            if let Err(e) =
                apply_scan_msg(&mut session, msg, &mut files_scanned, &mut bytes_scanned)
            {
                let _ = session.rollback();
                pool.join();
                return Err(e);
            }
            pending_scans = pending_scans.saturating_sub(1);

            prog.files_seen = files_seen;
            prog.files_scanned = files_scanned;
            prog.bytes_scanned = bytes_scanned;
            if throttle.should_report() {
                emit_progress(&self.cfg, &prog);
            }
        }
        pool.join();

        prog.stage = ScanStage::UpdatingDb;
        prog.files_seen = files_seen;
        prog.files_scanned = files_scanned;
        prog.bytes_scanned = bytes_scanned;
        emit_progress(&self.cfg, &prog);

        session.prune_unseen()?;
        session.set_stamp(current_stamp)?;
        session.commit()?;

        prog.stage = ScanStage::Finished;
        emit_progress(&self.cfg, &prog);

        Ok(SyncResult {
            root_id,
            mode: SyncMode::DeltaSync,
            files_seen,
            files_scanned,
            bytes_scanned,
        })
    }
}

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

fn worker_loop(rx: chan::Receiver<WalkItem>, tx: chan::Sender<ScanMsg>, cancel: Option<CancelFn>) {
    loop {
        if cancel.as_ref().map(|c| c()).unwrap_or(false) {
            break;
        }

        let item = match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(item) => item,
            Err(chan::RecvTimeoutError::Timeout) => continue,
            Err(chan::RecvTimeoutError::Disconnected) => break,
        };

        if cancel.as_ref().map(|c| c()).unwrap_or(false) {
            break;
        }

        let res = crate::scanner::swifty_map::scan_one(&item);
        if tx.send(ScanMsg { item, res }).is_err() {
            break;
        }
    }
}

fn apply_scan_msg(
    session: &mut UpdateSession,
    msg: ScanMsg,
    files_scanned: &mut u64,
    bytes_scanned: &mut u64,
) -> Result<(), Error> {
    match msg.res {
        Ok((file, segs)) => {
            session.upsert_file(&file)?;
            session.replace_segments(&file.rel_path, &segs)?;
            *files_scanned += 1;
            *bytes_scanned = bytes_scanned.saturating_add(msg.item.len);
            Ok(())
        }
        Err(e) => Err(Error::Swifty(e)),
    }
}

struct ScanMsg {
    item: WalkItem,
    res: Result<(FileEntry, Vec<SegmentEntry>), swifty_artifacts::SwiftyError>,
}
