use camino::Utf8PathBuf;
use directories::ProjectDirs;

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

pub fn internal_index_dir() -> Result<std::path::PathBuf, std::io::Error> {
    if let Some(dirs) = ProjectDirs::from("io", "fleet-app", "fleet") {
        let p = dirs.data_dir().join("indices");
        std::fs::create_dir_all(&p)?;
        return Ok(p);
    }
    let p = std::path::PathBuf::from("indices");
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

pub fn internal_staging_dir() -> Result<std::path::PathBuf, std::io::Error> {
    if let Some(dirs) = ProjectDirs::from("io", "fleet-app", "fleet") {
        let p = dirs.cache_dir().join("staging");
        std::fs::create_dir_all(&p)?;
        return Ok(p);
    }
    let p = std::path::PathBuf::from("staging");
    std::fs::create_dir_all(&p)?;
    Ok(p)
}
