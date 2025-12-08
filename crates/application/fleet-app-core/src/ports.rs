use crate::domain::{AppSettings, Profile};

pub trait ProfilesRepo: Send + Sync + 'static {
    fn load(&self) -> anyhow::Result<Vec<Profile>>;
    fn save(&self, profiles: &[Profile]) -> anyhow::Result<()>;
}

pub trait SettingsRepo: Send + Sync + 'static {
    fn load(&self) -> anyhow::Result<AppSettings>;
    fn save(&self, settings: &AppSettings) -> anyhow::Result<()>;
}

pub trait LauncherPort: Send + Sync + 'static {
    fn launch(
        &self,
        exe_path: &str,
        params: &str,
        template: &str,
        mods: &[camino::Utf8PathBuf],
    ) -> anyhow::Result<()>;
}

pub trait SyncPipelinePort: Send + Sync + 'static {
    fn validate_repo_url_blocking(&self, repo_url: &str) -> anyhow::Result<()>;
}
