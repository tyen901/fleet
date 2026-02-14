use crate::{Error, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub enum NonAsciiPolicy {
    Error,
    Skip,
}

#[derive(Debug, Clone)]
pub struct ScanPolicy {
    pub include_hidden: bool,
    pub non_ascii: NonAsciiPolicy,

    /// .gitignore-style path patterns (one per line).
    /// Examples: `repo.json`, `tmp/`, `mods/cache/*`.
    pub ignore_patterns: Vec<String>,
}

impl Default for ScanPolicy {
    fn default() -> Self {
        Self {
            include_hidden: false,
            non_ascii: NonAsciiPolicy::Error,
            ignore_patterns: vec![],
        }
    }
}

impl ScanPolicy {
    pub fn should_include_rel_path(&self, rel_forward_slash: &str, is_dir: bool) -> bool {
        if self.matches_ignore_patterns(rel_forward_slash, is_dir) {
            return false;
        }

        if !self.include_hidden && has_hidden_component(rel_forward_slash) {
            return false;
        }

        if is_dir {
            return true;
        }

        true
    }

    pub fn with_ignore_patterns(patterns: Vec<String>) -> Self {
        Self {
            ignore_patterns: patterns,
            ..Default::default()
        }
    }

    pub fn set_ignore_patterns(&mut self, patterns: Vec<String>) {
        self.ignore_patterns = patterns;
    }

    pub fn rel_path_forward_slash(&self, root: &Path, full: &Path) -> Result<String> {
        let rel = full.strip_prefix(root).map_err(|_| {
            Error::InvalidInput(format!(
                "path is not under root: root={} path={}",
                root.display(),
                full.display()
            ))
        })?;

        let s = rel.to_string_lossy().replace('\\', "/");

        if !s.is_ascii() {
            match self.non_ascii {
                NonAsciiPolicy::Skip => return Err(Error::InvalidInput("SKIP".to_string())),
                NonAsciiPolicy::Error => {
                    return Err(Error::InvalidInput(format!(
                        "non-ascii path not allowed: {s}"
                    )))
                }
            }
        }

        Ok(s)
    }

    pub fn normalize_root(root: impl AsRef<Path>) -> PathBuf {
        root.as_ref().to_path_buf()
    }
}

impl ScanPolicy {
    fn matches_ignore_patterns(&self, rel_forward_slash: &str, is_dir: bool) -> bool {
        self.ignore_patterns
            .iter()
            .any(|pattern| pattern_matches(rel_forward_slash, is_dir, pattern))
    }
}

fn pattern_matches(rel: &str, is_dir: bool, pattern: &str) -> bool {
    let raw = pattern.trim();
    if raw.is_empty() {
        return false;
    }
    let dir_rule = raw.ends_with('/');
    let pattern = raw.trim_matches('/');
    if pattern.is_empty() {
        return false;
    }

    // Folder rules: trailing slash in source pattern maps to "ignore this dir subtree".
    if dir_rule {
        let p = pattern;
        if p.contains('/') {
            return rel == p || rel.starts_with(&format!("{p}/"));
        }
        return rel
            .split('/')
            .any(|component| component.eq_ignore_ascii_case(p));
    }

    let has_wildcard = pattern.contains('*') || pattern.contains('?');
    if has_wildcard {
        return wildcard_match(pattern, rel);
    }

    if pattern.contains('/') {
        return rel == pattern || (is_dir && rel.starts_with(&format!("{pattern}/")));
    }

    if let Some(base) = rel.rsplit('/').next() {
        return base.eq_ignore_ascii_case(pattern);
    }

    false
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();

    let (mut pi, mut ti) = (0_usize, 0_usize);
    let mut star_idx = None::<usize>;
    let mut match_idx = 0_usize;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star_idx = Some(pi);
            match_idx = ti;
            pi += 1;
        } else if let Some(star) = star_idx {
            pi = star + 1;
            match_idx += 1;
            ti = match_idx;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }

    pi == p.len()
}

fn has_hidden_component(rel_forward_slash: &str) -> bool {
    rel_forward_slash
        .split('/')
        .any(|c| c.starts_with('.') && c != "." && c != ".." && !c.is_empty())
}

#[cfg(test)]
mod tests {
    use super::ScanPolicy;

    #[test]
    fn ignore_rules_support_files_and_folders() {
        let mut policy = ScanPolicy::with_ignore_patterns(vec![
            "repo.json".to_string(),
            "tmp/".to_string(),
            "mods/cache/*".to_string(),
        ]);
        policy.include_hidden = true;

        assert!(!policy.should_include_rel_path("repo.json", false));
        assert!(!policy.should_include_rel_path("a/tmp/log.txt", false));
        assert!(!policy.should_include_rel_path("mods/cache/data.bin", false));
        assert!(policy.should_include_rel_path("mods/live/data.bin", false));
    }

    #[test]
    fn hidden_dir_behavior_is_controlled_by_include_hidden() {
        let mut policy = ScanPolicy {
            include_hidden: false,
            ..Default::default()
        };
        assert!(!policy.should_include_rel_path(".hidden/cache.bin", false));
        assert!(!policy.should_include_rel_path("mods/.hidden/cache.bin", false));

        policy.include_hidden = true;
        assert!(policy.should_include_rel_path(".hidden/cache.bin", false));
        assert!(policy.should_include_rel_path("mods/.hidden/cache.bin", false));
    }
}
