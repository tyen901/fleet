use manifest_types::Md5Digest;
use relative_path::RelativePathBuf;

#[derive(Debug, Clone)]
pub enum Event {
    Started,
    RepoFetched {
        repo_name: String,
        version: String,
    },

    TransferPlanned {
        total_bytes: u64,
    },
    TransferProgress {
        transferred_bytes: u64,
        total_bytes: u64,
    },

    ModSkippedClean {
        mod_name: String,
    },
    ModChecking {
        mod_name: String,
    },
    ModAlreadyInSync {
        mod_name: String,
    },
    ModPlanned {
        mod_name: String,
        downloads: usize,
        deletes: usize,
    },
    ModApplied {
        mod_name: String,
    },
    ModFinished {
        mod_name: String,
        checksum: Md5Digest,
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
    },
    FileDeleted {
        mod_name: String,
        rel_path: RelativePathBuf,
    },

    Finished,
}
