use crate::core::run_config_blocking;
use crate::Core;
use fleet_domain::{normalize_app_settings, AppSettings};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsField {
    Arma3GameDir,
    Arma3LaunchMethod,
    Arma3CustomLaunchTemplate,
    Arma3DefaultArgs,
    AutoCheckProfilesOnStartup,
    AutoCheckOnStartup,
    ShowProfileIcons,
}

pub fn effective_settings_defaults() -> AppSettings {
    normalize_app_settings(AppSettings::default())
}

pub fn settings_field_is_non_default(
    field: SettingsField,
    settings: &AppSettings,
    defaults: &AppSettings,
) -> bool {
    match field {
        SettingsField::Arma3GameDir => {
            settings.arma3.arma3_game_dir != defaults.arma3.arma3_game_dir
        }
        SettingsField::Arma3LaunchMethod => {
            settings.arma3.arma3_launch_method != defaults.arma3.arma3_launch_method
        }
        SettingsField::Arma3CustomLaunchTemplate => {
            settings.arma3.arma3_custom_launch_template
                != defaults.arma3.arma3_custom_launch_template
        }
        SettingsField::Arma3DefaultArgs => {
            settings.arma3.arma3_default_args != defaults.arma3.arma3_default_args
        }
        SettingsField::AutoCheckProfilesOnStartup => {
            settings.startup.auto_check_profiles_on_startup
                != defaults.startup.auto_check_profiles_on_startup
        }
        SettingsField::AutoCheckOnStartup => {
            settings.updates.auto_check_on_startup != defaults.updates.auto_check_on_startup
        }
        SettingsField::ShowProfileIcons => {
            settings.ui.show_profile_icons != defaults.ui.show_profile_icons
        }
    }
}

impl Core {
    pub async fn load_settings(&self) -> anyhow::Result<AppSettings> {
        let _settings_guard = self.inner.settings_save_lock.lock().await;
        let loaded = run_config_blocking(self.config_repo(), |c| c.load_settings()).await?;
        let normalized = normalize_app_settings(loaded.clone());
        if loaded != normalized {
            let to_save = normalized.clone();
            run_config_blocking(self.config_repo(), move |c| c.save_settings(&to_save)).await?;
        }
        Ok(normalized)
    }

    pub async fn save_settings(&self, settings: AppSettings) -> Result<(), crate::ApiError> {
        let _save_guard = self.inner.settings_save_lock.lock().await;
        let settings = normalize_app_settings(settings);
        let settings_to_save = settings.clone();
        run_config_blocking(self.config_repo(), move |c| {
            c.save_settings(&settings_to_save)
        })
        .await
        .map_err(|error| crate::ApiError::new("settings_error", error.to_string()))?;
        self.update_state(|state| state.settings = settings);
        Ok(())
    }

    pub async fn reset_settings(&self) -> anyhow::Result<()> {
        let _settings_guard = self.inner.settings_save_lock.lock().await;
        run_config_blocking(self.config_repo(), |c| c.delete_settings()).await
    }

    pub async fn factory_reset(&self) -> Result<(), crate::ApiError> {
        self.reset_profiles()
            .await
            .map_err(|e| crate::ApiError::new("error", e.to_string()))?;
        self.reset_settings()
            .await
            .map_err(|e| crate::ApiError::new("error", e.to_string()))?;

        let settings = self
            .load_settings()
            .await
            .map_err(|e| crate::ApiError::new("error", e.to_string()))?;

        let profiles_cfg = self
            .list_profiles()
            .await
            .map_err(|e| crate::ApiError::new("error", e.to_string()))?;

        let mut profiles = std::collections::BTreeMap::new();
        for p in profiles_cfg.profiles {
            profiles.insert(p.id.clone(), p);
        }

        self.update_state(|state| {
            state.settings = settings;
            state.profiles = profiles;
            state.profile_runtime_by_id.clear();
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{effective_settings_defaults, settings_field_is_non_default};
    use crate::test_support::{EnvVarGuard, ENV_VAR_LOCK};
    use crate::{Core, SettingsField};
    use fleet_domain::{normalize_app_settings, AppSettings};

    #[test]
    fn effective_settings_defaults_matches_runtime_normalization() {
        let expected = normalize_app_settings(AppSettings::default());
        let actual = effective_settings_defaults();
        assert_eq!(
            actual.arma3.arma3_default_args,
            expected.arma3.arma3_default_args
        );
        assert_eq!(
            actual.arma3.arma3_launch_method,
            expected.arma3.arma3_launch_method
        );
    }

    #[test]
    fn concurrent_settings_saves_publish_the_final_persisted_settings() {
        let _guard = ENV_VAR_LOCK.lock().expect("env lock");

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp_dir.path());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .expect("runtime");

        runtime.block_on(async {
            let core = Core::new_for_test().expect("core");
            let base = core.load_settings().await.expect("load settings");
            let mut a = base.clone();
            a.startup.auto_check_profiles_on_startup = false;
            let mut b = base;
            b.ui.show_profile_icons = false;

            let (first, second) =
                tokio::join!(core.save_settings(a.clone()), core.save_settings(b.clone()));
            first.expect("save A");
            second.expect("save B");

            let persisted = core.load_settings().await.expect("load final settings");
            let published = core.read_state(|state| state.settings.clone());
            assert_eq!(published, persisted);
            assert!(persisted == a || persisted == b);
        });
    }

    #[test]
    fn failed_settings_save_preserves_published_state() {
        let _guard = ENV_VAR_LOCK.lock().expect("env lock");

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp_dir.path());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        runtime.block_on(async {
            let core = Core::new_for_test().expect("core");
            let mut persisted = core.load_settings().await.expect("load settings");
            persisted.ui.show_profile_icons = false;
            core.save_settings(persisted.clone())
                .await
                .expect("save settings");

            std::fs::remove_dir_all(temp_dir.path()).expect("remove config directory");
            std::fs::write(temp_dir.path(), "blocked").expect("block config path");

            let mut unsaved = persisted.clone();
            unsaved.startup.auto_check_profiles_on_startup = false;
            assert!(core.save_settings(unsaved).await.is_err());
            assert_eq!(core.read_state(|state| state.settings.clone()), persisted);

            std::fs::remove_file(temp_dir.path()).expect("remove config path blocker");
        });
    }

    #[test]
    fn auto_check_profiles_on_startup_reports_non_default() {
        let defaults = effective_settings_defaults();
        let mut settings = defaults.clone();
        settings.startup.auto_check_profiles_on_startup =
            !defaults.startup.auto_check_profiles_on_startup;

        assert!(settings_field_is_non_default(
            SettingsField::AutoCheckProfilesOnStartup,
            &settings,
            &defaults,
        ));
    }
}
