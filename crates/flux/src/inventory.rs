use crate::inspect::InspectionError;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use flux::{Key, LocalFile, Recipe, Stamp};
use redb::{Database, ReadableDatabase, TableDefinition};
use relative_path::RelativePath;
use std::{collections::BTreeMap, io, sync::Arc};

const DATABASE_FILE: &str = "swifty-inventory-v1.redb";
const FILES: TableDefinition<&str, &[u8]> = TableDefinition::new("recipes");
type Result<T> = std::result::Result<T, InspectionError>;
fn failure(error: impl std::fmt::Display) -> InspectionError {
    InspectionError::Inventory(error.to_string())
}

pub(super) struct Inventory(Database);
impl Inventory {
    pub fn open(root: &Dir, create: bool) -> Result<Option<Self>> {
        let name = super::input::FLEET_DIRECTORY;
        if create {
            match root.create_dir(name) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(e.into()),
            }
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(create)
            .follow(FollowSymlinks::No);
        let file = match root
            .open_dir_nofollow(name)
            .and_then(|area| area.open_with(DATABASE_FILE, &options))
        {
            Err(e) if !create && e.kind() == io::ErrorKind::NotFound => return Ok(None),
            other => other?,
        }
        .into_std();
        if !file.metadata()?.is_file() {
            return Err(failure("invalid inventory file"));
        }
        let backend = redb::backends::FileBackend::new(file).map_err(failure)?;
        let db = Database::builder()
            .create_with_backend(backend)
            .map_err(failure)?;
        if !db
            .begin_read()
            .map_err(failure)?
            .list_tables()
            .map_err(failure)?
            .any(|t| redb::TableHandle::name(&t) == redb::TableHandle::name(&FILES))
        {
            let tx = db.begin_write().map_err(failure)?;
            tx.open_table(FILES).map_err(failure)?;
            tx.commit().map_err(failure)?;
        }
        Ok(Some(Self(db)))
    }
    pub fn reader(&self) -> Result<redb::ReadOnlyTable<&'static str, &'static [u8]>> {
        self.0
            .begin_read()
            .map_err(failure)?
            .open_table(FILES)
            .map_err(failure)
    }
    pub fn lookup(
        table: &redb::ReadOnlyTable<&str, &[u8]>,
        path: &RelativePath,
        live: &Stamp,
    ) -> Result<Option<Recipe>> {
        let Some(row) = table.get(path.as_str()).map_err(failure)? else {
            return Ok(None);
        };
        let ((stamp, keys), consumed): ((Stamp, Vec<Key>), usize) =
            bincode::serde::decode_from_slice(row.value(), bincode::config::standard())
                .map_err(failure)?;
        if consumed != row.value().len() {
            return Err(failure("trailing inventory bytes"));
        }
        if &stamp != live {
            return Ok(None);
        }
        let recipe = Recipe::new(keys).map_err(failure)?;
        if recipe.length() != stamp.length() {
            return Err(failure("inventory recipe length mismatch"));
        }
        Ok(Some(recipe))
    }
    pub fn update(
        &self,
        rows: &BTreeMap<Arc<RelativePath>, LocalFile>,
        retain: Option<&flux::Goal>,
    ) -> Result<()> {
        if rows.is_empty() && retain.is_none() {
            return Ok(());
        }
        let tx = self.0.begin_write().map_err(failure)?;
        let mut changed = !rows.is_empty();
        {
            let mut table = tx.open_table(FILES).map_err(failure)?;
            if let Some(goal) = retain {
                table
                    .retain(|p, _| {
                        let keep = goal
                            .files()
                            .get_key_value(RelativePath::new(p))
                            .is_some_and(|(k, _)| k.as_str() == p);
                        changed |= !keep;
                        keep
                    })
                    .map_err(failure)?;
            }
            for (path, file) in rows {
                let data = bincode::serde::encode_to_vec(
                    (&file.stamp, file.recipe.keys()),
                    bincode::config::standard(),
                )
                .map_err(failure)?;
                table
                    .insert(path.as_str(), data.as_slice())
                    .map_err(failure)?;
            }
        }
        if changed {
            tx.commit().map_err(failure)?;
        }
        Ok(())
    }
}
