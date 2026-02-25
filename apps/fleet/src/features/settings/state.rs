#[derive(PartialEq, Clone)]
pub(crate) enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    UpdateAvailable { version: String },
    Downloading,
    Error(String),
}
