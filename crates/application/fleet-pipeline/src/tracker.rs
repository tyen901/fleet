use fleet_core::SyncPlan;
use fleet_infra::net::DownloadEvent;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct ActiveDownload {
    pub id: u64,
    pub file_name: String,
    pub mod_name: String,
    pub rel_path: String,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct TransferSnapshot {
    pub total_files: u64,
    pub downloaded_files: u64,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub speed_bps: u64,
    pub failed_count: u64,
    pub in_flight: Vec<ActiveDownload>,
}

pub struct ProgressTracker {
    id_map: HashMap<u64, (String, String)>, // ID -> (ModName, RelPath)
    in_flight: HashMap<u64, ActiveDownload>,
    downloaded_files: u64,
    failed_count: u64,
    current_downloaded_bytes: u64,
    total_files: u64,
    total_bytes: u64,
    last_tick: Instant,
    bytes_since_last_tick: u64,
    speed_bps: u64,
    history: VecDeque<u64>,
}

impl ProgressTracker {
    pub fn new(plan: &SyncPlan) -> Self {
        let mut id_map = HashMap::new();
        let mut total_bytes = 0;

        for (idx, action) in plan.downloads.iter().enumerate() {
            let id = idx as u64;
            id_map.insert(id, (action.mod_name.clone(), action.rel_path.clone()));
            total_bytes += action.size;
        }

        Self {
            id_map,
            in_flight: HashMap::new(),
            downloaded_files: 0,
            failed_count: 0,
            current_downloaded_bytes: 0,
            total_files: plan.downloads.len() as u64,
            total_bytes,
            last_tick: Instant::now(),
            bytes_since_last_tick: 0,
            speed_bps: 0,
            history: VecDeque::new(),
        }
    }

    pub fn update(&mut self, event: DownloadEvent) {
        match event {
            DownloadEvent::Started { id, total_bytes } => {
                if let Some((mod_name, rel_path)) = self.id_map.get(&id) {
                    let file_name = std::path::Path::new(rel_path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    self.in_flight.insert(
                        id,
                        ActiveDownload {
                            id,
                            mod_name: mod_name.clone(),
                            file_name,
                            rel_path: rel_path.clone(),
                            bytes_downloaded: 0,
                            total_bytes,
                        },
                    );
                }
            }
            DownloadEvent::Progress { id, bytes_delta } => {
                self.bytes_since_last_tick += bytes_delta;
                self.current_downloaded_bytes += bytes_delta;
                if let Some(entry) = self.in_flight.get_mut(&id) {
                    entry.bytes_downloaded += bytes_delta;
                }
            }
            DownloadEvent::Completed { id, success } => {
                self.in_flight.remove(&id);
                if success {
                    self.downloaded_files += 1;
                } else {
                    self.failed_count += 1;
                }
            }
        }
    }

    pub fn get_snapshot(&mut self) -> TransferSnapshot {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_tick).as_secs_f64();

        if elapsed >= 0.5 {
            let current_bps = (self.bytes_since_last_tick as f64 / elapsed) as u64;
            self.history.push_back(current_bps);
            if self.history.len() > 5 {
                self.history.pop_front();
            }
            self.speed_bps =
                (self.history.iter().sum::<u64>() as f64 / self.history.len() as f64) as u64;
            self.last_tick = now;
            self.bytes_since_last_tick = 0;
        }

        TransferSnapshot {
            total_files: self.total_files,
            downloaded_files: self.downloaded_files,
            total_bytes: self.total_bytes,
            downloaded_bytes: self.current_downloaded_bytes,
            speed_bps: self.speed_bps,
            failed_count: self.failed_count,
            in_flight: self.in_flight.values().cloned().collect(),
        }
    }
}
