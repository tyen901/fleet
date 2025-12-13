use camino::Utf8Path;
use fleet_core::path_utils::FleetPath;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct FileCacheEntry {
    pub mtime: u64,
    pub size: u64,
    pub checksum: String,
}

#[derive(Debug, Default, Clone)]
pub struct ScanCache {
    /// Map relative_path (Unix style) -> Entry
    pub entries: HashMap<String, FileCacheEntry>,
    dirty: bool,
}

impl ScanCache {
    /// Insert or update an entry. Returns true if something actually changed.
    pub fn update(&mut self, rel_path: &str, mtime: u64, size: u64, checksum: String) {
        let path_key = FleetPath::normalize(rel_path);
        let entry = FileCacheEntry {
            mtime,
            size,
            checksum,
        };
        self.entries.insert(path_key, entry);
        self.dirty = true;
    }

    pub fn get(&self, rel_path: &str) -> Option<&FileCacheEntry> {
        self.entries.get(&FleetPath::normalize(rel_path))
    }

    /// Remove an entry (e.g., after file deletion)
    pub fn remove(&mut self, rel_path: &str) {
        if self
            .entries
            .remove(&FleetPath::normalize(rel_path))
            .is_some()
        {
            self.dirty = true;
        }
    }

    /// Remove entries that point to non-existent files.
    /// Should be run rarely (e.g., end of a full sync or on app start).
    pub fn prune_ghosts(&mut self, base_path: &Utf8Path) {
        let before = self.entries.len();
        self.entries
            .retain(|rel_path, _| base_path.join(rel_path).exists());
        if self.entries.len() != before {
            self.dirty = true;
        }
    }
}
