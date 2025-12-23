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
    pub fn apply_to(self, model: &mut crate::sync::model::SyncModel) {
        match self {
            SyncEvent::CheckStarted { repo } => {
                model.phase = format!("Checking {repo}");
            }
            SyncEvent::RepairStarted { repo } => {
                model.phase = format!("Repairing {repo}");
            }
            SyncEvent::SyncFreshStarted { repo } => {
                model.phase = format!("Syncing {repo}");
            }
            SyncEvent::RemoteCapabilities { supports_ranges } => {
                model.remote_supports_ranges = Some(supports_ranges);
            }
            SyncEvent::CheckFinished { .. }
            | SyncEvent::RepairFinished { .. }
            | SyncEvent::SyncFreshFinished { .. } => {
                model.finished = true;
            }

            SyncEvent::FileStarted { bytes_total, .. } => {
                model.files_started += 1;
                model.bytes_total = bytes_total;
                model.bytes_done = 0;
                model.percent = 0;
            }
            SyncEvent::FileProgress {
                bytes_done,
                bytes_total,
                ..
            } => {
                model.bytes_done = bytes_done;
                model.bytes_total = bytes_total;
                model.percent = if model.bytes_total == 0 {
                    0
                } else {
                    ((model.bytes_done * 100) / model.bytes_total).min(100) as u8
                };
            }
            SyncEvent::FileVerified { .. } => {
                model.files_verified += 1;
            }
            SyncEvent::FileUpToDate { .. } => {
                model.files_up_to_date += 1;
            }
            SyncEvent::FileNeedsRepair { strategy, .. } => {
                model.last_strategy = Some(strategy);
            }
            SyncEvent::Warning { message } => {
                model.push_warning(message);
            }
            SyncEvent::Error { message } => {
                model.error = Some(message);
            }

            _ => {}
        }
    }
}

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
