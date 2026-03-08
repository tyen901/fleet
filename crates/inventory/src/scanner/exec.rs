use crate::scanner::config::{ScanObserver, ScanRuntimeConfig};
use crate::scanner::plan::{PlannedWalkItem, ScanPlan};
use crate::scanner::{ScanProgress, ScanStage};
use crate::{Error, FileEntry, SegmentEntry};
use crossbeam_channel as chan;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct ScanExecResult {
    pub scanned_files: u64,
    pub scanned_bytes: u64,
    pub outputs: Vec<ScanExecOutput>,
}

#[derive(Debug)]
pub(crate) struct ScanExecOutput {
    pub result: Result<(FileEntry, Vec<SegmentEntry>), swifty_artifacts::SwiftyError>,
}

pub(crate) fn execute_scan_plan(
    plan: ScanPlan,
    runtime: &ScanRuntimeConfig,
    observer: &ScanObserver,
) -> Result<ScanExecResult, Error> {
    let mut progress = ScanProgress {
        stage: ScanStage::Scanning,
        files_total: plan.hash_files_total,
        bytes_total: plan.hash_bytes_total,
        files_seen: plan.files_seen,
        ..Default::default()
    };
    emit_progress(observer, &progress);

    if is_cancelled(observer) {
        progress.stage = ScanStage::Cancelled;
        emit_progress(observer, &progress);
        return Err(Error::Cancelled);
    }

    let cap = runtime.queue_capacity.max(1);
    let workers = runtime.workers.max(1);
    let mut pool = WorkerPool::new(workers, cap, observer.cancel.clone());
    let mut throttle = ProgressReporter::new(runtime.progress_interval);

    let output_len = plan.scan_items.len();
    let mut outputs = (0..output_len).map(|_| None).collect::<Vec<_>>();
    let mut files_scanned = 0u64;
    let mut bytes_scanned = 0u64;
    let mut pending_scans = 0usize;

    for (idx, item) in plan.scan_items.iter().cloned().enumerate() {
        if is_cancelled(observer) {
            pool.join();
            progress.stage = ScanStage::Cancelled;
            progress.files_scanned = files_scanned;
            progress.bytes_scanned = bytes_scanned;
            emit_progress(observer, &progress);
            return Err(Error::Cancelled);
        }

        loop {
            match pool.sender().try_send(IndexedScanItem {
                idx,
                item: item.clone(),
            }) {
                Ok(()) => {
                    pending_scans += 1;
                    break;
                }
                Err(chan::TrySendError::Full(_)) => {
                    let msg = pool.receiver().recv().map_err(|_| Error::ChannelClosed)?;
                    record_exec_msg(
                        msg,
                        &mut outputs,
                        &mut files_scanned,
                        &mut bytes_scanned,
                        &mut progress,
                    );
                    pending_scans = pending_scans.saturating_sub(1);
                    if throttle.should_report() {
                        emit_progress(observer, &progress);
                    }
                }
                Err(chan::TrySendError::Disconnected(_)) => {
                    pool.join();
                    return Err(Error::ChannelClosed);
                }
            }
        }
    }
    pool.close_sender();

    while pending_scans > 0 {
        if is_cancelled(observer) {
            pool.join();
            progress.stage = ScanStage::Cancelled;
            progress.files_scanned = files_scanned;
            progress.bytes_scanned = bytes_scanned;
            emit_progress(observer, &progress);
            return Err(Error::Cancelled);
        }

        let msg = pool.receiver().recv().map_err(|_| Error::ChannelClosed)?;
        record_exec_msg(
            msg,
            &mut outputs,
            &mut files_scanned,
            &mut bytes_scanned,
            &mut progress,
        );
        pending_scans = pending_scans.saturating_sub(1);
        if throttle.should_report() {
            emit_progress(observer, &progress);
        }
    }
    pool.join();

    progress.files_scanned = files_scanned;
    progress.bytes_scanned = bytes_scanned;
    emit_progress(observer, &progress);

    Ok(ScanExecResult {
        scanned_files: files_scanned,
        scanned_bytes: bytes_scanned,
        outputs: outputs
            .into_iter()
            .map(|output| output.expect("scan result missing"))
            .collect(),
    })
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

fn record_exec_msg(
    msg: ScanMsg,
    outputs: &mut [Option<ScanExecOutput>],
    files_scanned: &mut u64,
    bytes_scanned: &mut u64,
    progress: &mut ScanProgress,
) {
    if msg.result.is_ok() {
        *files_scanned = files_scanned.saturating_add(1);
        *bytes_scanned = bytes_scanned.saturating_add(msg.item.item.len);
    }

    outputs[msg.idx] = Some(ScanExecOutput { result: msg.result });
    progress.files_scanned = *files_scanned;
    progress.bytes_scanned = *bytes_scanned;
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

#[derive(Clone, Debug)]
struct IndexedScanItem {
    idx: usize,
    item: PlannedWalkItem,
}

#[derive(Debug)]
struct ScanMsg {
    idx: usize,
    item: PlannedWalkItem,
    result: Result<(FileEntry, Vec<SegmentEntry>), swifty_artifacts::SwiftyError>,
}

struct WorkerPool {
    scan_tx: Option<chan::Sender<IndexedScanItem>>,
    res_rx: chan::Receiver<ScanMsg>,
    handles: Vec<std::thread::JoinHandle<()>>,
}

impl WorkerPool {
    fn new(workers: usize, cap: usize, cancel: Option<crate::scanner::CancelFn>) -> Self {
        let (scan_tx, scan_rx) = chan::bounded::<IndexedScanItem>(cap);
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

    fn sender(&self) -> &chan::Sender<IndexedScanItem> {
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
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.close_sender();
    }
}

fn worker_loop(
    rx: chan::Receiver<IndexedScanItem>,
    tx: chan::Sender<ScanMsg>,
    cancel: Option<crate::scanner::CancelFn>,
) {
    loop {
        if cancel.as_ref().map(|f| f()).unwrap_or(false) {
            break;
        }

        let item = match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(item) => item,
            Err(chan::RecvTimeoutError::Timeout) => continue,
            Err(chan::RecvTimeoutError::Disconnected) => break,
        };

        if cancel.as_ref().map(|f| f()).unwrap_or(false) {
            break;
        }

        let result = crate::scanner::swifty_map::scan_one(&item.item.item);
        if tx
            .send(ScanMsg {
                idx: item.idx,
                item: item.item,
                result,
            })
            .is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::execute_scan_plan;
    use crate::scanner::config::{ScanObserver, ScanRuntimeConfig};
    use crate::scanner::plan::{PlannedWalkItem, ScanPlan};
    use crate::FolderStamp;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn make_plan(root: &std::path::Path, files: &[&str]) -> ScanPlan {
        let scan_items = files
            .iter()
            .map(|name| PlannedWalkItem {
                item: crate::scanner::walk::WalkItem {
                    fs_path: root.join(name),
                    rel_path: (*name).to_string(),
                    len: std::fs::metadata(root.join(name)).expect("metadata").len(),
                },
            })
            .collect::<Vec<_>>();

        ScanPlan {
            seen_paths: files.iter().map(|name| (*name).to_string()).collect(),
            scan_items,
            current_stamp: FolderStamp {
                algo: "quick-v1".to_string(),
                hash64: 0,
                file_count: files.len() as u64,
                total_bytes: files.len() as u64,
            },
            files_seen: files.len() as u64,
            files_needing_scan: files.len() as u64,
            hash_files_total: files.len() as u64,
            hash_bytes_total: files.len() as u64,
            db_files_count: 0,
            saw_removed: false,
            saw_new_or_modified: !files.is_empty(),
            last_stamp: Some(FolderStamp {
                algo: "quick-v1".to_string(),
                hash64: 1,
                file_count: 0,
                total_bytes: 0,
            }),
        }
    }

    #[test]
    fn execution_honors_cancellation_before_scan() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(root.path().join("a.txt"), "a").expect("write file");

        let observer = ScanObserver {
            progress: None,
            cancel: Some(Arc::new(|| true)),
        };

        let result = execute_scan_plan(
            make_plan(root.path(), &["a.txt"]),
            &ScanRuntimeConfig {
                workers: 1,
                queue_capacity: 1,
                progress_interval: Duration::from_millis(0),
            },
            &observer,
        );

        assert!(matches!(result, Err(crate::Error::Cancelled)));
    }

    #[test]
    fn execution_honors_cancellation_during_scan() {
        let root = tempfile::tempdir().expect("tempdir");
        let files = (0..32)
            .map(|idx| {
                let name = format!("file_{idx}.txt");
                std::fs::write(root.path().join(&name), format!("content-{idx}")).expect("write");
                name
            })
            .collect::<Vec<_>>();

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_progress = Arc::clone(&cancel);
        let cancel_for_check = Arc::clone(&cancel);
        let observer = ScanObserver {
            progress: Some(Arc::new(move |progress| {
                if progress.files_scanned > 0 {
                    cancel_for_progress.store(true, Ordering::SeqCst);
                }
            })),
            cancel: Some(Arc::new(move || cancel_for_check.load(Ordering::SeqCst))),
        };

        let file_refs = files.iter().map(String::as_str).collect::<Vec<_>>();
        let result = execute_scan_plan(
            make_plan(root.path(), &file_refs),
            &ScanRuntimeConfig {
                workers: 1,
                queue_capacity: 1,
                progress_interval: Duration::from_millis(0),
            },
            &observer,
        );

        assert!(matches!(result, Err(crate::Error::Cancelled)));
    }
}
