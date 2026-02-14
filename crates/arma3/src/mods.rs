use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// How mod paths should be represented in Arma arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModPathStyle {
    /// Native paths (Windows runs the exe directly; or you want raw paths for Steam args).
    Native,
    /// Proton-friendly: prefix with `Z:` and use backslashes (HEMTT does similar).
    ProtonZDrive,
}

#[derive(Debug, Clone)]
pub struct ModList {
    paths: Vec<PathBuf>,
}

impl ModList {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Validate that all paths exist and are directories; dedupe.
    pub fn validate_and_normalize(mut paths: Vec<PathBuf>) -> Result<Self> {
        // Keep stable order but remove duplicates (basic).
        // You can sort/dedup if you prefer.
        paths.retain(|p| !p.as_os_str().is_empty());

        let mut out: Vec<PathBuf> = Vec::with_capacity(paths.len());
        for p in paths {
            if !p.exists() {
                return Err(Error::InvalidModDir {
                    path: p,
                    reason: "does not exist",
                });
            }
            if !p.is_dir() {
                return Err(Error::InvalidModDir {
                    path: p,
                    reason: "not a directory",
                });
            }
            if !out.contains(&p) {
                out.push(p);
            }
        }
        Ok(Self { paths: out })
    }

    /// Build Arma 3 mod argument(s).
    ///
    /// Many launchers use a single `-mod=path1;path2;path3` value.
    pub fn to_mod_arg(&self, style: ModPathStyle) -> String {
        let joined = self
            .paths
            .iter()
            .map(|p| render_mod_path(p, style))
            .collect::<Vec<_>>()
            .join(";");

        format!("-mod={joined}")
    }
}

fn render_mod_path(path: &Path, style: ModPathStyle) -> String {
    match style {
        ModPathStyle::Native => path.display().to_string(),
        ModPathStyle::ProtonZDrive => {
            // Convert /a/b/c -> Z:\a\b\c
            let s = path.display().to_string().replace('/', "\\");
            format!("Z:{s}")
        }
    }
}
