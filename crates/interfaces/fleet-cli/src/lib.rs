pub mod commands;
pub mod profiles;

use clap::ValueEnum;
use fleet_pipeline::sync::SyncMode;

#[derive(ValueEnum, Clone, Debug, Copy)]
pub enum CliScanStrategy {
    Smart,
    Force,
}

#[derive(ValueEnum, Clone, Debug, Copy)]
pub enum CliSyncMode {
    CacheOnly,
    Metadata,
    Smart,
    Fast,
    Full,
}

impl From<CliSyncMode> for SyncMode {
    fn from(m: CliSyncMode) -> Self {
        match m {
            CliSyncMode::CacheOnly => SyncMode::CacheOnly,
            CliSyncMode::Metadata => SyncMode::MetadataOnly,
            CliSyncMode::Smart => SyncMode::SmartVerify,
            CliSyncMode::Fast => SyncMode::FastCheck,
            CliSyncMode::Full => SyncMode::FullRehash,
        }
    }
}
