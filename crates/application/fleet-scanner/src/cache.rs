use camino::{Utf8Path, Utf8PathBuf};
use fleet_core::path_utils::FleetPath;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileCacheEntry {
    pub mtime: u64,
    pub len: u64,
    pub checksum: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ScanCache {
    /// Map relative_path (Unix style) -> Entry
    pub entries: HashMap<String, FileCacheEntry>,
    #[serde(skip)]
    dirty: bool,
}

impl ScanCache {
    pub fn load(path: &Utf8Path) -> Self {
        match fs::read_to_string(path) {
            Ok(s) => serde_json::from_str::<ScanCache>(&s).map_or_else(
                |e| {
                    tracing::warn!("Cache corrupted at {}, resetting: {}", path, e);
                    Self::default()
                },
                |mut c| {
                    c.dirty = false;
                    c
                },
            ),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Utf8Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let s = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // Atomic write
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, s)?;
        fs::rename(&tmp_path, path)?;
        Ok(())
    }

    /// Insert or update an entry. Returns true if something actually changed.
    pub fn update(&mut self, rel_path: &str, mtime: u64, len: u64, checksum: String) {
        let path_key = FleetPath::normalize(rel_path);
        let entry = FileCacheEntry {
            mtime,
            len,
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

    pub fn get_path(cache_root: &Utf8Path, mod_name: &str) -> Utf8PathBuf {
        let safe = mod_name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        cache_root.join(format!("{}.json", safe))
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
