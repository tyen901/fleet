use crate::{Error, NonAsciiPolicy, ScanPolicy};
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Clone)]
pub struct WalkItem {
    pub fs_path: PathBuf,
    pub rel_path: String, // forward slash; ASCII per policy
    pub len: u64,
}

pub struct WalkStream {
    iter: Box<dyn Iterator<Item = Result<DirEntry, walkdir::Error>>>,
    root: PathBuf,
    policy: ScanPolicy,
}

impl WalkStream {
    pub fn new(root: &Path, policy: &ScanPolicy) -> Result<Self, Error> {
        let root = root.to_path_buf();
        let policy = policy.clone();

        let root_clone = root.clone();
        let policy_clone = policy.clone();

        let iter = WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(move |e| filter_entry(&root_clone, &policy_clone, e));

        Ok(Self {
            iter: Box::new(iter),
            root,
            policy,
        })
    }
}

impl Iterator for WalkStream {
    type Item = Result<WalkItem, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let entry = self.iter.next()?;
            let entry = match entry {
                Ok(e) => e,
                Err(e) => return Some(Err(Error::Walkdir(e))),
            };

            if entry.file_type().is_symlink()
                || entry.file_type().is_dir()
                || !entry.file_type().is_file()
            {
                continue;
            }

            let fs_path = entry.path().to_path_buf();
            let rel = match fs_path.strip_prefix(&self.root) {
                Ok(r) => r,
                Err(_) => {
                    return Some(Err(Error::InvalidInput(format!(
                        "path not under root: {}",
                        fs_path.display()
                    ))))
                }
            };

            let rel_s = rel.to_string_lossy().replace('\\', "/");

            if !rel_s.is_ascii() {
                match self.policy.non_ascii {
                    NonAsciiPolicy::Skip => continue,
                    NonAsciiPolicy::Error => return Some(Err(Error::NonAsciiPath(rel_s))),
                }
            }

            if !self.policy.should_include_rel_path(&rel_s, false) {
                continue;
            }

            let md = match std::fs::symlink_metadata(entry.path()) {
                Ok(m) => m,
                Err(e) => return Some(Err(Error::Io(e))),
            };

            return Some(Ok(WalkItem {
                fs_path,
                rel_path: rel_s,
                len: md.len(),
            }));
        }
    }
}

fn filter_entry(root: &Path, policy: &ScanPolicy, e: &DirEntry) -> bool {
    if e.depth() == 0 {
        return true;
    }

    if !policy.include_hidden {
        if let Some(name) = e.file_name().to_str() {
            if name.starts_with('.') && name != "." && name != ".." {
                return false;
            }
        }
    }

    if e.file_type().is_file() {
        if let Ok(rel) = e.path().strip_prefix(root) {
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            if rel_s.is_ascii() && !policy.should_include_rel_path(&rel_s, false) {
                return false;
            }
        }
    }
    true
}
