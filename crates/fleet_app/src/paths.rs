use camino::Utf8PathBuf;
use directories::ProjectDirs;
use std::path::PathBuf;

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("io", "fleet-app", "fleet")
}

pub fn profiles_path() -> Result<Utf8PathBuf, std::io::Error> {
    if let Ok(p) = std::env::var("FLEET_PROFILES") {
        return Ok(Utf8PathBuf::from(p));
    }
    if let Some(dirs) = ProjectDirs::from("io", "fleet-app", "fleet") {
        let config_dir = dirs.config_dir();
        std::fs::create_dir_all(config_dir)?;
        return Utf8PathBuf::from_path_buf(config_dir.join("profiles.json"))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")));
    }
    Ok(Utf8PathBuf::from("profiles.json"))
}

pub fn settings_path() -> Result<Utf8PathBuf, std::io::Error> {
    if let Ok(p) = std::env::var("FLEET_SETTINGS") {
        return Ok(Utf8PathBuf::from(p));
    }
    if let Some(dirs) = ProjectDirs::from("io", "fleet-app", "fleet") {
        let config_dir = dirs.config_dir();
        std::fs::create_dir_all(config_dir)?;
        return Utf8PathBuf::from_path_buf(config_dir.join("settings.json"))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")));
    }
    Ok(Utf8PathBuf::from("settings.json"))
}

pub fn profile_data_dir(profile_id: &str) -> Result<PathBuf, std::io::Error> {
    let _ = uuid::Uuid::parse_str(profile_id)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid profile id"))?;

    if let Some(dirs) = project_dirs() {
        let p = dirs.data_dir().join("profiles").join(profile_id);
        std::fs::create_dir_all(&p)?;
        return Ok(p);
    }

    let p = PathBuf::from("profiles").join(profile_id);
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

pub fn profile_cache_dir(profile_id: &str) -> Result<PathBuf, std::io::Error> {
    let _ = uuid::Uuid::parse_str(profile_id)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid profile id"))?;

    if let Some(dirs) = project_dirs() {
        let p = dirs.cache_dir().join("profiles").join(profile_id);
        std::fs::create_dir_all(&p)?;
        return Ok(p);
    }

    let p = PathBuf::from("cache").join("profiles").join(profile_id);
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

pub fn profile_index_path(profile_id: &str) -> Result<PathBuf, std::io::Error> {
    Ok(profile_data_dir(profile_id)?.join("index.sqlite"))
}

pub fn profile_index_lock_path(profile_id: &str) -> Result<PathBuf, std::io::Error> {
    Ok(profile_data_dir(profile_id)?.join("index.sqlite.lock"))
}

pub fn profile_staging_dir(profile_id: &str) -> Result<PathBuf, std::io::Error> {
    Ok(profile_cache_dir(profile_id)?.join("staging"))
}
