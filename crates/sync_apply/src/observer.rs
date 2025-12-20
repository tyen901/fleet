use manifest_types::Md5Digest;
use relative_path::RelativePathBuf;

#[derive(Debug, Clone)]
pub enum ApplyEvent {
    TransferPlanned {
        total_bytes: u64,
    },
    TransferProgress {
        transferred_bytes: u64,
        total_bytes: u64,
    },
    FileStarted {
        mod_name: String,
        rel_path: RelativePathBuf,
        total_bytes: u64,
        resume_from: u64,
    },
    FileProgress {
        mod_name: String,
        rel_path: RelativePathBuf,
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    FileVerified {
        mod_name: String,
        rel_path: RelativePathBuf,
        checksum: Md5Digest,
    },
    FileDeleted {
        mod_name: String,
        rel_path: RelativePathBuf,
    },
}

pub trait ApplyObserver: Send + Sync {
    fn on_event(&self, ev: ApplyEvent);
}

pub struct NoopObserver;

impl ApplyObserver for NoopObserver {
    fn on_event(&self, _ev: ApplyEvent) {}
}
