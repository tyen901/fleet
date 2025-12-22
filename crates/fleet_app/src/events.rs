/// App-level sync events.
///
/// This type is the UI/API boundary: it should remain stable even if `sync_engine`
/// changes internal enums or payload types.
#[derive(Clone, Debug)]
pub enum SyncEvent {
    VerifyStarted {
        repo: String,
    },
    VerifyFinished {
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
    pub fn is_high_frequency(&self) -> bool {
        matches!(self, SyncEvent::FileProgress { .. })
    }
}

impl From<sync_engine::events::SyncEvent> for SyncEvent {
    fn from(ev: sync_engine::events::SyncEvent) -> Self {
        use sync_engine::events::SyncEvent as E;
        match ev {
            E::VerifyStarted { repo } => SyncEvent::VerifyStarted { repo },
            E::VerifyFinished { ok } => SyncEvent::VerifyFinished { ok },
            E::RepairStarted { repo } => SyncEvent::RepairStarted { repo },
            E::RepairSkipEvaluated { skippable, reason } => {
                SyncEvent::RepairSkipEvaluated { skippable, reason }
            }
            E::RepairFinished { ok, skipped } => SyncEvent::RepairFinished { ok, skipped },
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
