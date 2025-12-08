use camino::Utf8PathBuf;
use fleet_infra::launcher::Launcher;

pub struct LauncherImpl;

impl Default for LauncherImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl LauncherImpl {
    pub fn new() -> Self {
        Self
    }

    pub fn launch(
        &self,
        exe_path: &str,
        params: &str,
        template: &str,
        mods: &[Utf8PathBuf],
    ) -> anyhow::Result<()> {
        let launcher = Launcher::new(
            exe_path.to_string(),
            params.to_string(),
            template.to_string(),
        );
        launcher.launch(mods.to_vec())?;
        Ok(())
    }
}
