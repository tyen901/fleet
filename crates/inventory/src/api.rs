use crate::{
    scanner::walk::WalkStream,
    scanner::{Scanner, ScannerConfig, SyncRequest, SyncResult},
    DirtyFile, DirtyKind, Error, FolderStamp, InventoryDb, InventoryMetrics, InventorySnapshot,
    Result, RootId, ScanPolicy, SqliteStore,
};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// High-level facade for consumers (pipeline/sync/UI) to inspect state, scan, and repair.
///
/// This wraps:
/// - SQLite store + DB operations (InventoryDb)
/// - Scanner (Swifty scan + delta + DB update)
#[derive(Clone)]
pub struct Inventory {
    db: InventoryDb,
}

impl Inventory {
    pub fn open_sqlite(db_path: impl AsRef<Path>) -> Result<Self> {
        let store = SqliteStore::open(db_path)?;
        let db = InventoryDb::new(store);
        db.init()?;
        Ok(Self { db })
    }

    pub fn from_store(store: SqliteStore) -> Result<Self> {
        let db = InventoryDb::new(store);
        db.init()?;
        Ok(Self { db })
    }

    pub fn db(&self) -> &InventoryDb {
        &self.db
    }

    /// Bind to a logical inventory name + on-disk root path.
    pub fn open_root(
        &self,
        inventory_name: impl AsRef<str>,
        root_path: impl AsRef<Path>,
    ) -> Result<RootInventory> {
        let inventory_name = inventory_name.as_ref().to_string();
        if inventory_name.trim().is_empty() {
            return Err(Error::InvalidInput(
                "inventory_name must not be empty".into(),
            ));
        }

        let root_path = root_path.as_ref().to_path_buf();
        let inv_id = self.db.get_or_create_inventory(&inventory_name)?;
        let root_id = self
            .db
            .get_or_create_root(inv_id, &root_path.to_string_lossy())?;

        Ok(RootInventory {
            db: self.db.clone(),
            inventory_name,
            root_id,
            root_path,
        })
    }
}

/// A handle bound to one root.
///
/// This is what most callers should use.
#[derive(Clone)]
pub struct RootInventory {
    db: InventoryDb,
    inventory_name: String,
    root_id: RootId,
    root_path: PathBuf,
}

impl RootInventory {
    pub fn inventory_name(&self) -> &str {
        &self.inventory_name
    }

    pub fn root_id(&self) -> RootId {
        self.root_id
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub fn metrics(&self) -> Result<InventoryMetrics> {
        self.db.metrics(self.root_id)
    }

    pub fn snapshot(&self) -> Result<InventorySnapshot> {
        self.db.export_snapshot(self.root_id)
    }

    pub fn compute_stamp(&self, policy: &ScanPolicy) -> Result<FolderStamp> {
        self.db.compute_stamp(&self.root_path, policy)
    }

    /// Compute file-level drift between inventory DB and current disk view.
    ///
    /// - Added: path exists on disk but not in DB.
    /// - Removed: path exists in DB but not on disk.
    /// - Modified: path exists in both but byte length differs.
    pub fn dirty_files(&self, policy: &ScanPolicy) -> Result<Vec<DirtyFile>> {
        if !self.root_path.exists() {
            return Ok(Vec::new());
        }

        let mut db_files: HashMap<String, u64> = HashMap::new();
        for file in self.db.export_file_index(self.root_id)? {
            db_files.insert(file.rel_path, file.length);
        }

        let mut disk_files: HashMap<String, u64> = HashMap::new();
        let walk = WalkStream::new(&self.root_path, policy)?;
        for item in walk {
            let item = item?;
            disk_files.insert(item.rel_path, item.len);
        }

        let mut paths = BTreeSet::<String>::new();
        for path in db_files.keys() {
            paths.insert(path.clone());
        }
        for path in disk_files.keys() {
            paths.insert(path.clone());
        }

        let mut out = Vec::new();
        for rel_path in paths {
            let db_len = db_files.get(&rel_path).copied();
            let disk_len = disk_files.get(&rel_path).copied();
            let kind = match (db_len, disk_len) {
                (None, Some(_)) => Some(DirtyKind::Added),
                (Some(_), None) => Some(DirtyKind::Removed),
                (Some(a), Some(b)) if a != b => Some(DirtyKind::Modified),
                _ => None,
            };

            if let Some(kind) = kind {
                out.push(DirtyFile {
                    rel_path,
                    kind,
                    disk_len,
                    db_len,
                });
            }
        }

        Ok(out)
    }

    pub fn state(&self, policy: &ScanPolicy) -> Result<InventoryState> {
        // If root is missing, report it as state (not an error).
        if !self.root_path.exists() {
            return Ok(InventoryState::MissingRoot {
                root_path: self.root_path.clone(),
            });
        }

        let current = self.compute_stamp(policy)?;
        let dirty = self.db.is_dirty(self.root_id, &current)?;
        if dirty {
            let last = self.db.get_last_stamp(self.root_id)?;
            return Ok(InventoryState::Dirty {
                root_id: self.root_id,
                root_path: self.root_path.clone(),
                current,
                last,
            });
        }

        // Integrity guard: if stamp says clean but index totals mismatch, treat as dirty.
        let m = self.metrics()?;
        if m.files_count != current.file_count || m.files_bytes != current.total_bytes {
            return Ok(InventoryState::Dirty {
                root_id: self.root_id,
                root_path: self.root_path.clone(),
                current,
                last: m.last_stamp,
            });
        }

        Ok(InventoryState::Clean {
            root_id: self.root_id,
            root_path: self.root_path.clone(),
            current,
        })
    }

    /// Run a scan/update pass (delta-aware) and persist results.
    pub fn scan(&self, cfg: ScannerConfig) -> Result<SyncResult> {
        let scanner = Scanner::new(self.db.clone(), cfg);
        scanner.sync_root(SyncRequest {
            inventory_name: self.inventory_name.clone(),
            root_path: self.root_path.clone(),
        })
    }
}

#[derive(Clone, Debug)]
pub enum InventoryState {
    MissingRoot {
        root_path: PathBuf,
    },
    Dirty {
        root_id: RootId,
        root_path: PathBuf,
        current: FolderStamp,
        last: Option<FolderStamp>,
    },
    Clean {
        root_id: RootId,
        root_path: PathBuf,
        current: FolderStamp,
    },
}
