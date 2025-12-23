use std::collections::HashMap;

/// App-level sync events.
///
/// This type is the UI/API boundary: it should remain stable even if `fleet_sync`
/// changes internal enums or payload types.
#[derive(Clone, Debug)]
pub enum SyncEvent {
    CheckStarted {
        repo: String,
    },
    CheckFinished {
        ok: bool,
    },
    RepairStarted {
        repo: String,
    },
    RepairSkipEvaluated {
        skippable: bool,
        reason: Option<String>,
    },
    RepairFinished {
        ok: bool,
        skipped: bool,
    },
    SyncFreshStarted {
        repo: String,
    },
    SyncFreshFinished {
        ok: bool,
    },
    RemoteCapabilities {
        supports_ranges: bool,
    },

    ModStarted {
        mod_id: String,
    },
    ModFinished {
        mod_id: String,
    },

    FileUpToDate {
        mod_id: String,
        path: String,
    },
    FileNeedsRepair {
        mod_id: String,
        path: String,
        strategy: String,
    },
    FileStarted {
        mod_id: String,
        path: String,
        bytes_total: u64,
    },
    FileProgress {
        mod_id: String,
        path: String,
        bytes_done: u64,
        bytes_total: u64,
    },
    FileVerified {
        mod_id: String,
        path: String,
    },

    UnexpectedPathsFound {
        mod_id: String,
        files: u64,
        dirs: u64,
        bytes: u64,
        sample: Vec<String>,
    },
    UnexpectedPathDeleted {
        mod_id: String,
        path: String,
        bytes: u64,
        is_dir: bool,
    },
    UnexpectedPathsActionRequired {
        mod_id: String,
        message: String,
    },
    UnexpectedPathsCapReached {
        mod_id: String,
        message: String,
    },
    EmptyDirDeleted {
        path: String,
    },

    Warning {
        message: String,
    },
    Error {
        message: String,
    },
}

impl SyncEvent {
    /// Telemetry is high-volume and must never block the producer.
    /// Critical events are state transitions / warnings / errors that should be delivered best-effort.
    pub fn class(&self) -> SyncEventClass {
        match self {
            SyncEvent::FileProgress { .. }
            | SyncEvent::FileUpToDate { .. }
            | SyncEvent::FileVerified { .. }
            | SyncEvent::FileStarted { .. } => SyncEventClass::Telemetry,
            _ => SyncEventClass::Critical,
        }
    }

    pub fn is_telemetry(&self) -> bool {
        self.class() == SyncEventClass::Telemetry
    }

    pub fn is_critical(&self) -> bool {
        self.class() == SyncEventClass::Critical
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncEventClass {
    Telemetry,
    Critical,
}

/// Strongly-typed “critical only” channel payload.
#[derive(Clone, Debug)]
pub struct CriticalEvent(pub SyncEvent);

impl CriticalEvent {
    pub fn new(ev: SyncEvent) -> Option<Self> {
        ev.is_critical().then_some(Self(ev))
    }

    pub fn as_inner(&self) -> &SyncEvent {
        &self.0
    }

    pub fn into_inner(self) -> SyncEvent {
        self.0
    }
}

#[derive(Clone, Debug, Default)]
pub struct TelemetryCounts {
    pub files_started: u64,
    pub files_verified: u64,
    pub files_up_to_date: u64,
}

#[derive(Clone, Debug)]
pub struct FileProgressSnapshot {
    pub mod_id: String,
    pub path: String,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

#[derive(Clone, Debug)]
pub struct TelemetryLogEntry {
    pub seq: u64,
    pub text: String,
}

/// UI-facing progress snapshot: latest-per-file + rollups.
#[derive(Clone, Debug)]
pub struct ProgressSnapshot {
    pub phase: String,
    pub percent: u8,
    pub bytes_done: Option<u64>,
    pub bytes_total: Option<u64>,
    pub active_files: Vec<FileProgressSnapshot>,
    pub counts: TelemetryCounts,
    pub dropped_critical_count: u64,
    pub remote_supports_ranges: Option<bool>,
    pub last_strategy: Option<String>,
    /// Debug-only: bounded tail of per-file telemetry lines (empty unless enabled).
    pub telemetry_log_tail: Vec<TelemetryLogEntry>,
}

impl Default for ProgressSnapshot {
    fn default() -> Self {
        Self {
            phase: "Idle".to_string(),
            percent: 0,
            bytes_done: None,
            bytes_total: None,
            active_files: Vec::new(),
            counts: TelemetryCounts::default(),
            dropped_critical_count: 0,
            remote_supports_ranges: None,
            last_strategy: None,
            telemetry_log_tail: Vec::new(),
        }
    }
}

/// Optional helper for consumers that want a quick lookup.
pub type ActiveFileMap = HashMap<(String, String), FileProgressSnapshot>;

impl From<fleet_sync::SyncEvent> for SyncEvent {
    fn from(ev: fleet_sync::SyncEvent) -> Self {
        use fleet_sync::SyncEvent as E;
        match ev {
            E::CheckStarted { repo } => SyncEvent::CheckStarted { repo },
            E::CheckFinished { ok } => SyncEvent::CheckFinished { ok },
            E::RepairStarted { repo } => SyncEvent::RepairStarted { repo },
            E::RepairSkipEvaluated { skippable, reason } => {
                SyncEvent::RepairSkipEvaluated { skippable, reason }
            }
            E::RepairFinished { ok, skipped } => SyncEvent::RepairFinished { ok, skipped },
            E::SyncFreshStarted { repo } => SyncEvent::SyncFreshStarted { repo },
            E::SyncFreshFinished { ok } => SyncEvent::SyncFreshFinished { ok },
            E::RemoteCapabilities { supports_ranges } => {
                SyncEvent::RemoteCapabilities { supports_ranges }
            }

            E::ModStarted { mod_id } => SyncEvent::ModStarted { mod_id },
            E::ModFinished { mod_id } => SyncEvent::ModFinished { mod_id },

            E::FileUpToDate { mod_id, path } => SyncEvent::FileUpToDate { mod_id, path },
            E::FileNeedsRepair {
                mod_id,
                path,
                strategy,
            } => SyncEvent::FileNeedsRepair {
                mod_id,
                path,
                strategy,
            },
            E::FileStarted {
                mod_id,
                path,
                bytes_total,
            } => SyncEvent::FileStarted {
                mod_id,
                path,
                bytes_total,
            },
            E::FileProgress {
                mod_id,
                path,
                bytes_done,
                bytes_total,
            } => SyncEvent::FileProgress {
                mod_id,
                path,
                bytes_done,
                bytes_total,
            },
            E::FileVerified { mod_id, path } => SyncEvent::FileVerified { mod_id, path },

            E::UnexpectedPathsFound {
                mod_id,
                files,
                dirs,
                bytes,
                sample,
            } => SyncEvent::UnexpectedPathsFound {
                mod_id,
                files,
                dirs,
                bytes,
                sample,
            },
            E::UnexpectedPathDeleted {
                mod_id,
                path,
                bytes,
                is_dir,
            } => SyncEvent::UnexpectedPathDeleted {
                mod_id,
                path,
                bytes,
                is_dir,
            },
            E::UnexpectedPathsActionRequired { mod_id, message } => {
                SyncEvent::UnexpectedPathsActionRequired { mod_id, message }
            }
            E::UnexpectedPathsCapReached { mod_id, message } => {
                SyncEvent::UnexpectedPathsCapReached { mod_id, message }
            }
            E::EmptyDirDeleted { path } => SyncEvent::EmptyDirDeleted { path },

            E::Warning { message } => SyncEvent::Warning { message },
            E::Error { message } => SyncEvent::Error { message },
        }
    }
}
