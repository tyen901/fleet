use crate::core::run_config_blocking;
use crate::Core;
use fleet_domain::{AppSettings, InventoryIgnoreRules};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsField {
    ReleaseChannel,
    ThemeMode,
    Arma3GameDir,
    Arma3LaunchMethod,
    Arma3CustomLaunchTemplate,
    Arma3DefaultArgs,
    TelemetryConsent,
}

pub fn effective_settings_defaults() -> AppSettings {
    normalize_settings(AppSettings::default())
}

impl Core {
    pub async fn load_settings(&self) -> anyhow::Result<AppSettings> {
        let loaded = run_config_blocking(self.config_repo(), |c| c.load_settings()).await?;
        let normalized = normalize_settings(loaded.clone());
        if settings_changed_after_normalize(&loaded, &normalized) {
            let to_save = normalized.clone();
            run_config_blocking(self.config_repo(), move |c| c.save_settings(&to_save)).await?;
        }
        Ok(normalized)
    }

    pub async fn save_settings(&self, settings: AppSettings) -> anyhow::Result<()> {
        let settings = normalize_settings(settings);
        run_config_blocking(self.config_repo(), move |c| c.save_settings(&settings)).await
    }

    pub async fn reset_settings(&self) -> anyhow::Result<()> {
        run_config_blocking(self.config_repo(), |c| c.delete_settings()).await
    }

    pub async fn settings_save(&self, settings: AppSettings) -> Result<(), crate::ApiError> {
        self.save_settings(settings)
            .await
            .map_err(|e| crate::ApiError::new("error", e.to_string()))?;

        if let Ok(settings) = self.load_settings().await {
            self.update_state(|state| {
                state.settings = settings;
            });
        }

        Ok(())
    }

    pub async fn settings_reset_field(&self, field: SettingsField) -> Result<(), crate::ApiError> {
        let mut settings = self
            .load_settings()
            .await
            .map_err(|e| crate::ApiError::new("error", e.to_string()))?;
        let defaults = effective_settings_defaults();
        apply_field_default(&mut settings, &defaults, field);

        self.settings_save(settings).await
    }

    pub async fn reset_to_defaults(&self) -> Result<(), crate::ApiError> {
        self.reset_settings()
            .await
            .map_err(|e| crate::ApiError::new("error", e.to_string()))?;

        let settings = self
            .load_settings()
            .await
            .map_err(|e| crate::ApiError::new("error", e.to_string()))?;

        self.update_state(|state| {
            state.settings = settings;
        });

        Ok(())
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
            state.sync = None;
            state.profile_states.clear();
            state.last_sync_by_profile.clear();
            state.last_launch = None;
        });

        Ok(())
    }
}

fn normalize_settings(mut settings: AppSettings) -> AppSettings {
    if settings.arma3_default_args.trim().is_empty() {
        settings.arma3_default_args = crate::features::arma3::DEFAULT_ARMA3_ARGS.to_string();
    }
    settings.arma3_launch_method = settings
        .arma3_launch_method
        .normalize_for_current_platform();
    settings.inventory_ignore_rules =
        InventoryIgnoreRules::from_settings_value(&settings.inventory_ignore_rules)
            .to_multiline_string();
    settings
}

fn apply_field_default(settings: &mut AppSettings, defaults: &AppSettings, field: SettingsField) {
    match field {
        SettingsField::ReleaseChannel => {
            settings.release_channel = defaults.release_channel.clone();
        }
        SettingsField::ThemeMode => {
            settings.theme_mode = defaults.theme_mode.clone();
        }
        SettingsField::Arma3GameDir => {
            settings.arma3_game_dir = defaults.arma3_game_dir.clone();
        }
        SettingsField::Arma3LaunchMethod => {
            settings.arma3_launch_method = defaults.arma3_launch_method;
        }
        SettingsField::Arma3CustomLaunchTemplate => {
            settings.arma3_custom_launch_template = defaults.arma3_custom_launch_template.clone();
        }
        SettingsField::Arma3DefaultArgs => {
            settings.arma3_default_args = defaults.arma3_default_args.clone();
        }
        SettingsField::TelemetryConsent => {
            settings.telemetry_consent = defaults.telemetry_consent;
        }
    }
}

fn settings_changed_after_normalize(before: &AppSettings, after: &AppSettings) -> bool {
    before.arma3_default_args != after.arma3_default_args
        || before.arma3_launch_method != after.arma3_launch_method
        || before.inventory_ignore_rules != after.inventory_ignore_rules
}

#[cfg(test)]
mod tests {
    use super::{
        apply_field_default, effective_settings_defaults, normalize_settings, SettingsField,
    };
    use crate::Core;
    use fleet_domain::{AppSettings, Arma3LaunchMethod};
    use std::sync::Mutex;

    struct EnvVarGuard {
        key: &'static str,
        old: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
            let old = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, old }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.old.take() {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn non_default_settings() -> AppSettings {
        AppSettings {
            release_channel: "dev".to_string(),
            theme_mode: "light".to_string(),
            arma3_game_dir: "/tmp/arma3".to_string(),
            arma3_launch_method: Arma3LaunchMethod::Custom,
            arma3_custom_launch_template: "custom-run $ARGS $MODS".to_string(),
            arma3_default_args: "-window".to_string(),
            telemetry_consent: Some(false),
            ..normalize_settings(AppSettings::default())
        }
    }

    #[test]
    fn effective_settings_defaults_matches_runtime_normalization() {
        let expected = normalize_settings(AppSettings::default());
        let actual = effective_settings_defaults();
        assert_eq!(actual.arma3_default_args, expected.arma3_default_args);
        assert_eq!(actual.arma3_launch_method, expected.arma3_launch_method);
        assert_eq!(
            actual.inventory_ignore_rules,
            expected.inventory_ignore_rules
        );
    }

    #[test]
    fn reset_release_channel_field_restores_default() {
        let defaults = effective_settings_defaults();
        let mut settings = non_default_settings();
        apply_field_default(&mut settings, &defaults, SettingsField::ReleaseChannel);
        assert_eq!(settings.release_channel, defaults.release_channel);
    }

    #[test]
    fn reset_theme_mode_field_restores_default() {
        let defaults = effective_settings_defaults();
        let mut settings = non_default_settings();
        apply_field_default(&mut settings, &defaults, SettingsField::ThemeMode);
        assert_eq!(settings.theme_mode, defaults.theme_mode);
    }

    #[test]
    fn reset_arma3_game_dir_field_restores_default() {
        let defaults = effective_settings_defaults();
        let mut settings = non_default_settings();
        apply_field_default(&mut settings, &defaults, SettingsField::Arma3GameDir);
        assert_eq!(settings.arma3_game_dir, defaults.arma3_game_dir);
    }

    #[test]
    fn reset_arma3_launch_method_field_restores_default() {
        let defaults = effective_settings_defaults();
        let mut settings = non_default_settings();
        apply_field_default(&mut settings, &defaults, SettingsField::Arma3LaunchMethod);
        assert_eq!(settings.arma3_launch_method, defaults.arma3_launch_method);
    }

    #[test]
    fn reset_arma3_custom_launch_template_field_restores_default() {
        let defaults = effective_settings_defaults();
        let mut settings = non_default_settings();
        apply_field_default(
            &mut settings,
            &defaults,
            SettingsField::Arma3CustomLaunchTemplate,
        );
        assert_eq!(
            settings.arma3_custom_launch_template,
            defaults.arma3_custom_launch_template
        );
    }

    #[test]
    fn reset_arma3_default_args_field_restores_default() {
        let defaults = effective_settings_defaults();
        let mut settings = non_default_settings();
        apply_field_default(&mut settings, &defaults, SettingsField::Arma3DefaultArgs);
        assert_eq!(settings.arma3_default_args, defaults.arma3_default_args);
    }

    #[test]
    fn reset_telemetry_consent_field_restores_default() {
        let defaults = effective_settings_defaults();
        let mut settings = non_default_settings();
        apply_field_default(&mut settings, &defaults, SettingsField::TelemetryConsent);
        assert_eq!(settings.telemetry_consent, defaults.telemetry_consent);
    }

    #[test]
    fn reset_to_defaults_resets_only_settings() {
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().expect("env lock");

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp_dir.path());

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        runtime.block_on(async {
            let core = Core::spawn_threaded_default().expect("core");
            let profile = fleet_domain::Profile {
                id: "abcd".to_string(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/destination".to_string(),
                ..Default::default()
            };
            core.profile_save(profile).await.expect("save profile");

            let mut changed = core.load_settings().await.expect("load settings");
            changed.theme_mode = "light".to_string();
            changed.release_channel = "dev".to_string();
            changed.telemetry_consent = Some(false);
            core.settings_save(changed).await.expect("save settings");

            core.reset_to_defaults().await.expect("reset defaults");

            let profiles = core.list_profiles().await.expect("list profiles");
            assert_eq!(profiles.profiles.len(), 1);

            let settings = core.load_settings().await.expect("load settings");
            let defaults = effective_settings_defaults();
            assert_eq!(settings.theme_mode, defaults.theme_mode);
            assert_eq!(settings.release_channel, defaults.release_channel);
            assert_eq!(settings.telemetry_consent, defaults.telemetry_consent);
        });
    }
}
