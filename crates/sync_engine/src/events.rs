#[derive(Debug, Clone)]
pub enum SyncEvent {
    RepoStarted {
        repo: String,
    },
    RemoteCapabilities {
        supports_ranges: bool,
    },
    RepoReady {
        mods_available: usize,
        mods_enabled: usize,
    },

    TransferPlanned {
        total_bytes: u64,
    },
    TransferProgress {
        transferred_bytes: u64,
        total_bytes: u64,
    },

    ModStarted {
        mod_id: String,
    },
    ModFinished {
        mod_id: String,
    },

    DirEnsured {
        path: String,
    },
    PathDeleted {
        path: String,
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

    Warning {
        message: String,
    },
    Error {
        message: String,
    },
}

pub trait EventSink: Send + Sync {
    fn push(&self, ev: SyncEvent);
}

pub struct NoopSink;
impl EventSink for NoopSink {
    fn push(&self, _ev: SyncEvent) {}
}
