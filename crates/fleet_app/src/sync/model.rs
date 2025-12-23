use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct SyncModel {
    pub phase: String,
    pub remote_supports_ranges: Option<bool>,
    pub percent: u8,
    pub bytes_done: u64,
    pub bytes_total: u64,

    pub files_verified: u64,
    pub files_up_to_date: u64,
    pub files_started: u64,

    pub last_strategy: Option<String>,

    pub warnings: VecDeque<String>,
    pub error: Option<String>,
    pub finished: bool,
}

impl SyncModel {
    pub fn new() -> Self {
        Self {
            phase: "Idle".into(),
            remote_supports_ranges: None,
            percent: 0,
            bytes_done: 0,
            bytes_total: 0,
            files_verified: 0,
            files_up_to_date: 0,
            files_started: 0,
            last_strategy: None,
            warnings: VecDeque::with_capacity(128),
            error: None,
            finished: false,
        }
    }

    pub(crate) fn push_warning(&mut self, msg: String) {
        if self.warnings.len() >= 128 {
            self.warnings.pop_front();
        }
        self.warnings.push_back(msg);
    }
}

impl Default for SyncModel {
    fn default() -> Self {
        Self::new()
    }
}
