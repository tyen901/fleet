use super::{Files, Mode, Result};
use crate::{input::FLEET_DIRECTORY, inventory::Inventory, profile::read_recipe};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use flux::{Goal, LocalFile, Stamp};
use relative_path::RelativePath;
use std::{
    collections::{BTreeSet, HashSet},
    io::{self, Read},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio_util::sync::CancellationToken;

// Bound both payload re-reading after interruption and tiny-file transaction size.
const CHECKPOINT_BYTES: u64 = 64 * 1024 * 1024;
const CHECKPOINT_FILES: usize = 128;
pub(super) const CHECKPOINT_EVENT: &str = "fleet.inventory.checkpoint";

struct Inspection<'a> {
    inventory: &'a Inventory,
    pending_bytes: u64,
    goal: &'a Goal,
    mode: Mode,
    cancel: &'a CancellationToken,
    bytes: &'a AtomicU64,
    cached: redb::ReadOnlyTable<&'static str, &'static [u8]>,
    files: Files,
    changed: Files,
    extras: Vec<Arc<RelativePath>>,
    directories: BTreeSet<Arc<RelativePath>>,
    exact: bool,
}
struct Meter<'a> {
    file: &'a mut std::fs::File,
    cancel: &'a CancellationToken,
    bytes: &'a AtomicU64,
}
impl Read for Meter<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("scan cancelled"));
        }
        let n = self.file.read(buf)?;
        self.bytes.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}
impl Inspection<'_> {
    fn checkpoint(&mut self) -> Result<()> {
        if self.changed.is_empty() {
            return Ok(());
        }
        self.inventory.update(&self.changed, None)?;
        self.cached = self.inventory.reader()?;
        tracing::debug!(name: CHECKPOINT_EVENT, files=self.changed.len(),bytes=self.pending_bytes,"committed scan checkpoint");
        self.changed.clear();
        self.pending_bytes = 0;
        Ok(())
    }

    fn observe(
        &mut self,
        dir: &Dir,
        name: &str,
        path: &Arc<RelativePath>,
    ) -> Result<Option<LocalFile>> {
        let mut opts = OpenOptions::new();
        opts.read(true).follow(FollowSymlinks::No);
        let mut file = dir.open_with(name, &opts)?.into_std();
        let stamp = Stamp::from_file(&file)?;
        let cached = if self.mode == Mode::Verify {
            None
        } else {
            Inventory::lookup(&self.cached, path, &stamp)?
        };
        let observed = if let Some(recipe) = cached {
            LocalFile { stamp, recipe }
        } else {
            if self.mode == Mode::Check {
                self.exact = false;
                return Ok(None);
            }
            let mut reader = Meter {
                file: &mut file,
                cancel: self.cancel,
                bytes: self.bytes,
            };
            let recipe = read_recipe(path, stamp.length(), &mut reader);
            if self.cancel.is_cancelled() {
                return Err(flux::Error::Cancelled.into());
            }
            let recipe = recipe?;
            if recipe.length() != stamp.length()
                || Stamp::from_file(&file)? != stamp
                || Stamp::from_file(&dir.open_with(name, &opts)?.into_std())? != stamp
            {
                return Err(flux::Error::Conflict(format!("changed during scan: {path}")).into());
            }
            let observed = LocalFile { stamp, recipe };
            self.pending_bytes = self.pending_bytes.saturating_add(observed.stamp.length());
            self.changed.insert(path.clone(), observed.clone());
            if self.pending_bytes >= CHECKPOINT_BYTES || self.changed.len() >= CHECKPOINT_FILES {
                self.checkpoint()?;
            }
            observed
        };
        Ok(Some(observed))
    }

    fn find_extra_donors(&mut self, root: &Dir) -> Result<()> {
        if self.extras.is_empty() {
            return Ok(());
        }
        let mut missing: HashSet<_> = self
            .goal
            .files()
            .iter()
            .filter(|(p, r)| self.files.get(*p).is_none_or(|f| &f.recipe != *r))
            .flat_map(|(_, r)| r.keys())
            .cloned()
            .collect();
        for file in self.files.values() {
            for key in file.recipe.keys() {
                missing.remove(key);
            }
        }
        for path in std::mem::take(&mut self.extras) {
            if missing.is_empty() {
                break;
            }
            if self.cancel.is_cancelled() {
                return Err(flux::Error::Cancelled.into());
            }
            let mut dir = root.try_clone()?;
            for parent in path.parent().into_iter().flat_map(|p| p.iter()) {
                dir = dir.open_dir_nofollow(parent)?;
            }
            if let Some(file) =
                self.observe(&dir, path.file_name().expect("enumerated file name"), &path)?
            {
                for key in file.recipe.keys() {
                    missing.remove(key);
                }
                self.files.insert(path, file);
            }
        }
        Ok(())
    }
    fn walk(&mut self, dir: &Dir, prefix: &RelativePath) -> Result<bool> {
        let mut private = false;
        let mut visible = false;
        for entry in dir.entries()? {
            if self.cancel.is_cancelled() {
                return Err(flux::Error::Cancelled.into());
            }
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                self.exact = false;
                continue;
            };
            if name.eq_ignore_ascii_case(FLEET_DIRECTORY)
                || name.eq_ignore_ascii_case(flux::PRIVATE_DIRECTORY)
            {
                private = true;
                continue;
            }
            let meta = dir.symlink_metadata(name)?;
            #[cfg(windows)]
            {
                use cap_std::fs::MetadataExt;
                if meta.file_attributes()
                    & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                    != 0
                {
                    return Err(flux::Error::Conflict(format!(
                        "target contains a reparse point: {name}"
                    ))
                    .into());
                }
            }
            let path: Arc<RelativePath> = prefix.join(name).into();
            if meta.is_dir() {
                let only_private = self.walk(&dir.open_dir_nofollow(name)?, &path)?;
                if self.goal.directories().contains(&path) {
                    self.directories.insert(path);
                } else if !only_private {
                    self.exact = false;
                }
                private |= only_private;
                visible |= !only_private;
            } else if meta.is_file() {
                visible = true;
                if !self.goal.files().contains_key(&path) {
                    self.exact = false;
                    if self.mode == Mode::Prepare {
                        self.extras.push(path);
                    }
                    continue;
                }
                if let Some(file) = self.observe(dir, name, &path)? {
                    self.exact &= self.goal.files().get(&path) == Some(&file.recipe);
                    self.files.insert(path, file);
                }
            } else {
                return Err(
                    flux::Error::Conflict(format!("unsupported target entry: {path}")).into(),
                );
            }
        }
        Ok(private && !visible)
    }
}

pub(super) fn root(path: &Path, create: bool) -> Result<Dir> {
    if create {
        std::fs::create_dir_all(path)?;
    }
    let meta = std::fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err(flux::Error::Conflict("target is a reparse point".into()).into());
        }
    }
    if !meta.is_dir() || meta.is_symlink() {
        return Err(flux::Error::Conflict("invalid target directory".into()).into());
    }
    Ok(Dir::open_ambient_dir(path, ambient_authority())?)
}
pub(super) fn observe(
    root: &Dir,
    inventory: &Inventory,
    goal: &Goal,
    mode: Mode,
    cancel: &CancellationToken,
    bytes: &AtomicU64,
) -> Result<(bool, Files)> {
    let mut scan = Inspection {
        pending_bytes: 0,
        inventory,
        goal,
        mode,
        cancel,
        bytes,
        cached: inventory.reader()?,
        files: Files::new(),
        changed: Files::new(),
        extras: Vec::new(),
        directories: Default::default(),
        exact: true,
    };
    let scanned = scan
        .walk(root, RelativePath::new(""))
        .and_then(|_| scan.find_extra_donors(root));
    scan.checkpoint()?;
    scanned?;
    let exact = scan.exact
        && scan.files.len() == goal.files().len()
        && scan.directories.len() == goal.directories().len();
    Ok((exact, scan.files))
}
