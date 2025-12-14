use redb::TableDefinition;

pub const DB_FILENAME: &str = "fleet_state.redb";
pub const META_FORMAT: &str = "fleet-state-redb";
pub const SCHEMA_VERSION: u32 = 2;

pub const META: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("meta");
pub const PROFILES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("profiles");
pub const SETTINGS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("settings");
pub const UI_STATE: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("ui_state");
pub const REMOTE_REPO: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("remote_repo");
pub const SERVER_CHOICE: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("server_choice");
pub const PLAN: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("plan");
pub const STATUS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("status");
pub const LOCAL_BASELINE_MANIFEST: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("local_baseline_manifest");
pub const LOCAL_BASELINE_SUMMARY: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("local_baseline_summary");
pub const SCAN_CACHE: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("scan_cache");
