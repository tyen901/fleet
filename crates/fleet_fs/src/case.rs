use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

type HashFileFn = dyn Fn(&Path) -> io::Result<Vec<u8>>;

#[derive(Clone, Debug, Default)]
pub struct CaseIndex {
    pub files: HashMap<String, Vec<String>>,
    pub dirs: HashMap<String, Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct CaseFixTuning {
    pub hard_delete_losers: bool,
    pub trash_rel: PathBuf,
}

impl Default for CaseFixTuning {
    fn default() -> Self {
        Self {
            hard_delete_losers: false,
            trash_rel: PathBuf::from(".fleet/trash/casefix"),
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct CaseFixStats {
    pub mod_roots_merged: u64,
    pub files_renamed_or_moved: u64,
    pub collision_losers_removed: u64,
    pub io_errors: u64,
}

#[derive(Clone, Debug)]
pub struct ModDirResolution {
    pub expected: String,
    pub matches: Vec<String>,
    pub chosen: Option<String>,
}

impl ModDirResolution {
    pub fn has_collision(&self) -> bool {
        self.matches.len() > 1
    }

    pub fn has_case_mismatch(&self) -> bool {
        self.chosen.as_deref().is_some_and(|c| c != self.expected)
    }
}

pub fn case_key(s: &str) -> String {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}

pub fn norm_rel(s: &str) -> String {
    s.replace('\\', "/")
}

pub fn build_case_index(mod_root: &Path) -> io::Result<CaseIndex> {
    let mut idx = CaseIndex::default();

    for entry in walkdir::WalkDir::new(mod_root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path == mod_root {
            continue;
        }

        let md = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if md.file_type().is_symlink() {
            continue;
        }

        let rel = match path.strip_prefix(mod_root) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let rel_norm = rel.to_string_lossy().replace('\\', "/");
        let key = case_key(&rel_norm);

        if md.is_dir() {
            idx.dirs.entry(key).or_default().push(rel_norm);
        } else if md.is_file() {
            idx.files.entry(key).or_default().push(rel_norm);
        }
    }

    for v in idx.dirs.values_mut() {
        v.sort();
    }
    for v in idx.files.values_mut() {
        v.sort();
    }

    Ok(idx)
}

pub fn resolve_mod_dir(checkout_root: &Path, mod_id: &str) -> io::Result<ModDirResolution> {
    let expected = mod_id.to_string();
    let want = case_key(mod_id);

    let mut matches = Vec::new();
    // read_dir can fail if checkout_root doesn't exist, handle gracefully or prop
    if let Ok(rd) = std::fs::read_dir(checkout_root) {
        for entry in rd {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if !ft.is_dir() || ft.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if case_key(&name) == want {
                matches.push(name);
            }
        }
    }
    matches.sort();

    let chosen = if matches.is_empty() {
        None
    } else if matches.iter().any(|m| m == mod_id) {
        Some(mod_id.to_string())
    } else {
        Some(matches[0].clone())
    };

    Ok(ModDirResolution {
        expected,
        matches,
        chosen,
    })
}

pub fn join_under(mod_root: &Path, rel: &str) -> PathBuf {
    let rel = norm_rel(rel);
    mod_root.join(rel)
}

fn now_unix_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn rand_suffix() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{n}")
}

fn unique_temp_path_in_dir(dir: &Path) -> PathBuf {
    dir.join(format!(".fleet_case_tmp_{}", rand_suffix()))
}

fn rename_via_temp(from: &Path, to: &Path) -> io::Result<()> {
    if from == to {
        return Ok(());
    }
    let parent = from
        .parent()
        .ok_or_else(|| io::Error::other("rename_via_temp: no parent"))?;
    let tmp = unique_temp_path_in_dir(parent);
    std::fs::rename(from, &tmp)?;
    std::fs::rename(&tmp, to)?;
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    Ok(())
}

fn dir_entries_case_insensitive(parent: &Path, name: &str) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(parent) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for entry in rd {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let md = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if !md.is_dir() {
            continue;
        }
        let n = entry.file_name().to_string_lossy().to_string();
        if n.eq_ignore_ascii_case(name) {
            out.push(entry.path());
        }
    }
    out.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    Ok(out)
}

fn file_entries_case_insensitive(parent: &Path, name: &str) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(parent) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for entry in rd {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let md = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if !md.is_file() {
            continue;
        }
        let n = entry.file_name().to_string_lossy().to_string();
        if n.eq_ignore_ascii_case(name) {
            out.push(entry.path());
        }
    }
    out.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    Ok(out)
}

fn ensure_dir_named(
    checkout_root: &Path,
    parent: &Path,
    desired: &str,
    tuning: &CaseFixTuning,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(parent)?;
    let desired_path = parent.join(desired);
    let mut matches = dir_entries_case_insensitive(parent, desired)?;
    if matches.iter().any(|p| {
        p.file_name()
            .is_some_and(|n| n == std::ffi::OsStr::new(desired))
    }) {
        return Ok(desired_path);
    }

    if matches.is_empty() {
        std::fs::create_dir_all(&desired_path)?;
        return Ok(desired_path);
    }

    // Keep the first match; remove the rest silently.
    let keep = matches.remove(0);
    for loser in matches {
        let _ = remove_or_trash(checkout_root, &loser, tuning, "dirloser");
    }

    // Rename to desired casing (even if it already resolves via case-insensitive lookup).
    rename_via_temp(&keep, &desired_path)?;
    Ok(desired_path)
}

fn ensure_path_dirs_cased(
    checkout_root: &Path,
    canonical_root: &Path,
    rel_norm: &str,
    tuning: &CaseFixTuning,
) -> io::Result<PathBuf> {
    let mut cur = canonical_root.to_path_buf();
    let parts: Vec<&str> = rel_norm.split('/').collect();
    if parts.len() <= 1 {
        return Ok(cur);
    }
    for dir in &parts[..parts.len() - 1] {
        if dir.is_empty() {
            continue;
        }
        cur = ensure_dir_named(checkout_root, &cur, dir, tuning)?;
    }
    Ok(cur)
}

fn copy_then_delete(src: &Path, dst: &Path) -> io::Result<()> {
    ensure_parent_dir(dst)?;
    std::fs::copy(src, dst)?;
    std::fs::remove_file(src)?;
    Ok(())
}

fn move_file(src: &Path, dst: &Path) -> io::Result<()> {
    ensure_parent_dir(dst)?;
    match rename_via_temp(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => copy_then_delete(src, dst),
    }
}

fn remove_any(path: &Path) -> io::Result<()> {
    let md = std::fs::symlink_metadata(path)?;
    if md.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn trash_path(checkout_root: &Path, tuning: &CaseFixTuning, label: &str) -> io::Result<PathBuf> {
    let root = checkout_root
        .join(&tuning.trash_rel)
        .join(format!("{}", now_unix_s()));
    std::fs::create_dir_all(&root)?;
    Ok(root.join(format!("{label}_{}", rand_suffix())))
}

fn remove_or_trash(
    checkout_root: &Path,
    path: &Path,
    tuning: &CaseFixTuning,
    label: &str,
) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if tuning.hard_delete_losers {
        let _ = remove_any(path);
        return Ok(());
    }
    let dst = trash_path(checkout_root, tuning, label)?;
    match std::fs::rename(path, &dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            let _ = remove_any(path);
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
struct Candidate {
    full: PathBuf,
    size: u64,
    mtime_ns: i64,
    is_canonical_path: bool,
}

fn file_mtime_ns(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
        .unwrap_or(0)
}

pub fn case_sweep_and_fix(
    checkout_root: &Path,
    mod_id: &str,
    expected: &[(String, u64, Option<Vec<u8>>)],
    tuning: &CaseFixTuning,
    hash_file: Option<&HashFileFn>,
) -> io::Result<CaseFixStats> {
    let mut stats = CaseFixStats::default();

    let canonical_root = ensure_dir_named(checkout_root, checkout_root, mod_id, tuning)?;
    let canonical_real = std::fs::canonicalize(&canonical_root).unwrap_or(canonical_root.clone());

    let roots_raw: Vec<PathBuf> = resolve_mod_dir(checkout_root, mod_id)?
        .matches
        .into_iter()
        .map(|m| checkout_root.join(m))
        .chain(std::iter::once(canonical_root.clone()))
        .collect();

    let mut roots_by_real: HashMap<PathBuf, PathBuf> = HashMap::new();
    for r in roots_raw {
        if std::fs::metadata(&r).is_err() {
            continue;
        }
        let real = std::fs::canonicalize(&r).unwrap_or_else(|_| r.clone());
        roots_by_real.entry(real).or_insert(r);
    }

    let mut indices: Vec<(PathBuf, PathBuf, CaseIndex)> = Vec::new();
    for (real, display) in &roots_by_real {
        indices.push((display.clone(), real.clone(), build_case_index(display)?));
    }

    for (rel, expected_size, expected_hash) in expected {
        let rel_norm = norm_rel(rel);
        let key = case_key(&rel_norm);
        let mut cands: Vec<Candidate> = Vec::new();

        for (root_display, root_real, idx) in &indices {
            let Some(list) = idx.files.get(&key) else {
                continue;
            };
            for on_disk_rel in list {
                let abs = join_under(root_display, on_disk_rel);
                let md = match std::fs::symlink_metadata(&abs) {
                    Ok(m) => m,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e),
                };
                if !md.is_file() || md.file_type().is_symlink() {
                    continue;
                }
                let is_canonical_root = *root_real == canonical_real;
                let is_canonical_path = is_canonical_root && *on_disk_rel == rel_norm;
                cands.push(Candidate {
                    full: abs,
                    size: md.len(),
                    mtime_ns: file_mtime_ns(&md),
                    is_canonical_path,
                });
            }
        }

        if cands.is_empty() {
            continue;
        }

        let dst_parent = ensure_path_dirs_cased(checkout_root, &canonical_root, &rel_norm, tuning)?;
        let file_name = rel_norm.split('/').next_back().unwrap_or(&rel_norm);
        let dst = dst_parent.join(file_name);

        let mut best_idx = 0usize;
        for i in 1..cands.len() {
            if better_candidate(
                &cands[i],
                &cands[best_idx],
                expected_size,
                expected_hash.as_deref(),
                hash_file,
            )? {
                best_idx = i;
            }
        }

        if dst.exists()
            && cands[best_idx].full != dst
            && !same_fs_entry(&cands[best_idx].full, &dst)
        {
            stats.collision_losers_removed += 1;
            remove_or_trash(checkout_root, &dst, tuning, "dst")?;
        }

        if cands[best_idx].full != dst && !same_fs_entry(&cands[best_idx].full, &dst) {
            if let Err(_e) = move_file(&cands[best_idx].full, &dst) {
                stats.io_errors += 1;
            } else {
                stats.files_renamed_or_moved += 1;
            }
        } else if cands[best_idx].full != dst && same_fs_entry(&cands[best_idx].full, &dst) {
            if let Ok(mut matches) = file_entries_case_insensitive(&dst_parent, file_name) {
                if let Some(from) = matches.pop() {
                    if from != dst {
                        let _ = rename_via_temp(&from, &dst);
                    }
                }
            }
        }

        for (i, c) in cands.iter().enumerate() {
            if i == best_idx {
                continue;
            }
            stats.collision_losers_removed += 1;
            let _ = remove_or_trash(checkout_root, &c.full, tuning, "loser");
        }
    }

    for (real, display) in roots_by_real {
        if real == canonical_real {
            continue;
        }
        if display.exists() {
            stats.mod_roots_merged += 1;
            let _ = remove_or_trash(checkout_root, &display, tuning, "modroot");
        }
    }

    Ok(stats)
}

fn better_candidate(
    a: &Candidate,
    b: &Candidate,
    expected_size: &u64,
    expected_hash: Option<&[u8]>,
    hash_file: Option<&HashFileFn>,
) -> io::Result<bool> {
    let a_canon = a.is_canonical_path;
    let b_canon = b.is_canonical_path;
    if a_canon != b_canon {
        return Ok(a_canon);
    }

    let a_size = a.size == *expected_size;
    let b_size = b.size == *expected_size;
    if a_size != b_size {
        return Ok(a_size);
    }

    if let (Some(exp), Some(hasher)) = (expected_hash, hash_file) {
        let ah = if a_size { Some(hasher(&a.full)?) } else { None };
        let bh = if b_size { Some(hasher(&b.full)?) } else { None };
        let a_match = ah.as_deref().is_some_and(|h| h == exp);
        let b_match = bh.as_deref().is_some_and(|h| h == exp);
        if a_match != b_match {
            return Ok(a_match);
        }
    }

    if a.mtime_ns != b.mtime_ns {
        return Ok(a.mtime_ns > b.mtime_ns);
    }

    Ok(a.full.to_string_lossy() < b.full.to_string_lossy())
}

fn same_fs_entry(a: &Path, b: &Path) -> bool {
    let ca = std::fs::canonicalize(a);
    let cb = std::fs::canonicalize(b);
    match (ca, cb) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
