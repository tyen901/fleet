use crate::{
    compute_stamp, sqlite::SqliteUpdateSession, Error, FileEntry, FolderStamp, InventoryId,
    InventoryMetrics, InventorySnapshot, Result, RootId, ScanPolicy, SegmentEntry, SqliteStore,
};
use std::path::Path;

/// Low-level DB facade for inventory storage operations.
#[derive(Clone)]
pub struct InventoryDb {
    store: SqliteStore,
}

impl InventoryDb {
    pub fn new(store: SqliteStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    pub fn init(&self) -> Result<()> {
        self.store.init()
    }

    pub fn get_or_create_inventory(&self, name: &str) -> Result<InventoryId> {
        require_non_empty("inventory name", name)?;
        self.store.get_or_create_inventory(name)
    }

    pub fn get_or_create_root(&self, inventory_id: InventoryId, root_path: &str) -> Result<RootId> {
        require_non_empty("root path", root_path)?;
        self.store.get_or_create_root(inventory_id, root_path)
    }

    pub fn compute_stamp(&self, root_path: &Path, policy: &ScanPolicy) -> Result<FolderStamp> {
        compute_stamp(root_path, policy)
    }

    pub fn get_last_stamp(&self, root_id: RootId) -> Result<Option<FolderStamp>> {
        self.store.get_last_stamp(root_id)
    }

    pub fn is_dirty(&self, root_id: RootId, current: &FolderStamp) -> Result<bool> {
        let last = self.store.get_last_stamp(root_id)?;
        Ok(match last {
            None => true,
            Some(prev) => {
                prev.algo != current.algo
                    || prev.hash64 != current.hash64
                    || prev.file_count != current.file_count
                    || prev.total_bytes != current.total_bytes
            }
        })
    }

    pub fn export_file_index(&self, root_id: RootId) -> Result<Vec<FileEntry>> {
        self.store.export_file_index(root_id)
    }

    pub fn stream_file_index(
        &self,
        root_id: RootId,
        mut cb: impl FnMut(FileEntry) -> Result<()>,
    ) -> Result<()> {
        self.store.stream_file_index(root_id, &mut cb)
    }

    pub fn begin_update(&self, root_id: RootId) -> Result<UpdateSession> {
        Ok(UpdateSession {
            inner: self.store.begin_update(root_id)?,
        })
    }

    pub fn export_snapshot(&self, root_id: RootId) -> Result<InventorySnapshot> {
        self.store.export_snapshot(root_id)
    }

    pub fn metrics(&self, root_id: RootId) -> Result<InventoryMetrics> {
        self.store.metrics(root_id)
    }
}

pub struct UpdateSession {
    inner: SqliteUpdateSession,
}

impl UpdateSession {
    pub fn set_stamp(&mut self, stamp: FolderStamp) -> Result<()> {
        if stamp.algo.trim().is_empty() {
            return Err(Error::InvalidInput(
                "stamp algo must not be empty".to_string(),
            ));
        }
        self.inner.set_stamp(stamp)
    }

    pub fn upsert_file(&mut self, file: &FileEntry) -> Result<()> {
        require_non_empty("file rel_path", &file.rel_path)?;
        self.inner.upsert_file(file)
    }

    pub fn upsert_files_batch(&mut self, files: &[FileEntry]) -> Result<()> {
        for f in files {
            require_non_empty("file rel_path", &f.rel_path)?;
        }
        self.inner.upsert_files_batch(files)
    }

    pub fn replace_segments(&mut self, rel_path: &str, segments: &[SegmentEntry]) -> Result<()> {
        require_non_empty("file rel_path", rel_path)?;
        self.inner.replace_segments(rel_path, segments)
    }

    pub fn begin_seen_set(&mut self) -> Result<()> {
        self.inner.begin_seen_set()
    }

    pub fn mark_seen(&mut self, rel_path: &str) -> Result<()> {
        require_non_empty("file rel_path", rel_path)?;
        self.inner.mark_seen(rel_path)
    }

    pub fn prune_unseen(&mut self) -> Result<()> {
        self.inner.prune_unseen()
    }

    pub fn delete_file(&mut self, rel_path: &str) -> Result<()> {
        require_non_empty("file rel_path", rel_path)?;
        self.inner.delete_file(rel_path)
    }

    pub fn commit(self) -> Result<()> {
        self.inner.commit()
    }

    pub fn rollback(self) -> Result<()> {
        self.inner.rollback()
    }
}

fn require_non_empty(label: &str, s: &str) -> Result<()> {
    if s.trim().is_empty() {
        return Err(Error::InvalidInput(format!("{label} must not be empty")));
    }
    Ok(())
}
