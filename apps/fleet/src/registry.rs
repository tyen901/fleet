use std::collections::HashSet;
use std::io::{Read, Write};

use camino::{Utf8Path, Utf8PathBuf};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    pub schema_version: u32,
    pub selected_profile: Option<String>,
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub repo_url: String,
    pub checkout_root: String,
    pub created_unix_s: i64,
    pub last_sync_unix_s: Option<i64>,
    #[serde(default)]
    pub arma3: Arma3Config,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Arma3Config {
    #[serde(default)]
    pub extra_args: String,
    #[serde(default)]
    pub enabled_mods: Vec<String>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            schema_version: 2,
            selected_profile: None,
            profiles: Vec::new(),
        }
    }
}

impl Registry {
    pub fn add_profile(&mut self, mut p: Profile) {
        let base = slugify(&p.name);
        let id = unique_slug(&base, self.profiles.iter().map(|x| x.id.clone()));
        p.id = id.clone();
        self.selected_profile = Some(id.clone());
        self.profiles.push(p);
    }

    pub fn remove_profile(&mut self, id: &str) -> bool {
        let before = self.profiles.len();
        self.profiles.retain(|p| p.id != id);
        let removed = self.profiles.len() != before;

        if removed && self.selected_profile.as_deref() == Some(id) {
            self.selected_profile = self.profiles.first().map(|p| p.id.clone());
        }

        removed
    }

    pub fn selected(&self) -> Option<&Profile> {
        let id = self.selected_profile.as_deref()?;
        self.profiles.iter().find(|p| p.id == id)
    }

    pub fn selected_mut(&mut self) -> Option<&mut Profile> {
        let id = self.selected_profile.clone()?;
        self.profiles.iter_mut().find(|p| p.id == id)
    }
}

pub fn registry_path() -> Result<Utf8PathBuf, std::io::Error> {
    let proj = ProjectDirs::from("com", "rts", "fleet")
        .ok_or_else(|| std::io::Error::other("no project dirs"))?;

    let base = proj.data_dir();
    let p = base
        .join("appdata")
        .join("a-safe-place-on-linux")
        .join("registry.json");

    Utf8PathBuf::from_path_buf(p).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "registry path not valid UTF-8",
        )
    })
}

pub fn load_registry(path: &Utf8Path) -> Result<Registry, std::io::Error> {
    if !path.exists() {
        return Ok(Registry::default());
    }

    let mut f = std::fs::File::open(path.as_std_path())?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;

    match serde_json::from_str::<Registry>(&s) {
        Ok(mut reg) => {
            if reg.schema_version < 2 {
                reg.schema_version = 2;
            }
            Ok(reg)
        }
        Err(e) => {
            let backup = path.with_extension(format!("corrupt-{}.json", unix_suffix()));
            let _ = std::fs::rename(path.as_std_path(), backup.as_std_path());
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to parse registry: {e} (moved aside)"),
            ))
        }
    }
}

pub fn save_registry_atomic(path: &Utf8Path, reg: &Registry) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent.as_std_path())?;
    }

    let tmp = path.with_extension("json.tmp");

    let bytes = serde_json::to_vec_pretty(reg).map_err(|e| std::io::Error::other(e.to_string()))?;

    {
        let mut f = std::fs::File::create(tmp.as_std_path())?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }

    std::fs::rename(tmp.as_std_path(), path.as_std_path())?;
    Ok(())
}

pub fn setup_checkout_root(path: &Utf8Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(path.as_std_path())?;
    std::fs::create_dir_all(path.join(".fleet").as_std_path())?;
    Ok(())
}

pub fn normalize_repo_url(s: &str) -> String {
    let t = s.trim();
    if t.ends_with('/') {
        t.to_string()
    } else {
        format!("{t}/")
    }
}

pub fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;

    for ch in input.trim().to_lowercase().chars() {
        let ok = ch.is_ascii_alphanumeric();
        if ok {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }

    while out.starts_with('-') {
        out.remove(0);
    }
    while out.ends_with('-') {
        out.pop();
    }

    if out.is_empty() {
        "profile".to_string()
    } else {
        out
    }
}

pub fn unique_slug(base: &str, existing: impl Iterator<Item = String>) -> String {
    let set: HashSet<String> = existing.collect();

    if !set.contains(base) {
        return base.to_string();
    }

    for i in 2..=10_000u32 {
        let cand = format!("{base}-{i}");
        if !set.contains(&cand) {
            return cand;
        }
    }

    format!("{base}-{}", unix_suffix())
}

fn unix_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
