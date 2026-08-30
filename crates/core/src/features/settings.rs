use crate::core::run_config_blocking;
use crate::Core;
use fleet_domain::{normalize_app_settings, AppSettings};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsField {
    Arma3GameDir,
    Arma3LaunchMethod,
    Arma3CustomLaunchTemplate,
    Arma3DefaultArgs,
    TelemetryConsent,
    AutoCheckProfilesOnStartup,
    AutoCheckOnStartup,
    ShowProfileIcons,
}

pub fn effective_settings_defaults() -> AppSettings {
    normalize_settings(AppSettings::default())
}

struct SettingsFieldSpec {
    is_non_default: fn(&AppSettings, &AppSettings) -> bool,
}

fn settings_field_spec(field: SettingsField) -> SettingsFieldSpec {
    match field {
        SettingsField::Arma3GameDir => SettingsFieldSpec {
            is_non_default: |settings, defaults| {
                settings.arma3.arma3_game_dir != defaults.arma3.arma3_game_dir
            },
        },
        SettingsField::Arma3LaunchMethod => SettingsFieldSpec {
            is_non_default: |settings, defaults| {
                settings.arma3.arma3_launch_method != defaults.arma3.arma3_launch_method
            },
        },
        SettingsField::Arma3CustomLaunchTemplate => SettingsFieldSpec {
            is_non_default: |settings, defaults| {
                settings.arma3.arma3_custom_launch_template
                    != defaults.arma3.arma3_custom_launch_template
            },
        },
        SettingsField::Arma3DefaultArgs => SettingsFieldSpec {
            is_non_default: |settings, defaults| {
                settings.arma3.arma3_default_args != defaults.arma3.arma3_default_args
            },
        },
        SettingsField::TelemetryConsent => SettingsFieldSpec {
            is_non_default: |settings, defaults| {
                settings.privacy.telemetry_consent != defaults.privacy.telemetry_consent
            },
        },
        SettingsField::AutoCheckProfilesOnStartup => SettingsFieldSpec {
            is_non_default: |settings, defaults| {
                settings.startup.auto_check_profiles_on_startup
                    != defaults.startup.auto_check_profiles_on_startup
            },
        },
        SettingsField::AutoCheckOnStartup => SettingsFieldSpec {
            is_non_default: |settings, defaults| {
                settings.updates.auto_check_on_startup != defaults.updates.auto_check_on_startup
            },
        },
        SettingsField::ShowProfileIcons => SettingsFieldSpec {
            is_non_default: |settings, defaults| {
                settings.ui.show_profile_icons != defaults.ui.show_profile_icons
            },
        },
    }
}

pub fn settings_field_is_non_default(
    field: SettingsField,
    settings: &AppSettings,
    defaults: &AppSettings,
) -> bool {
    (settings_field_spec(field).is_non_default)(settings, defaults)
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
        let settings = normalize_settings(settings);
        self.update_state(|state| {
            state.settings = settings.clone();
        });
        run_config_blocking(self.config_repo(), move |c| c.save_settings(&settings))
            .await
            .map_err(|e| crate::ApiError::new("error", e.to_string()))?;

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
            state.selected_profile_id = None;
            state.profile_runtime_by_id.clear();
        });

        Ok(())
    }
}

fn normalize_settings(mut settings: AppSettings) -> AppSettings {
    settings = normalize_app_settings(settings);
    settings
}

fn settings_changed_after_normalize(before: &AppSettings, after: &AppSettings) -> bool {
    [
        SettingsField::Arma3GameDir,
        SettingsField::Arma3LaunchMethod,
        SettingsField::Arma3CustomLaunchTemplate,
        SettingsField::Arma3DefaultArgs,
        SettingsField::TelemetryConsent,
        SettingsField::AutoCheckProfilesOnStartup,
        SettingsField::AutoCheckOnStartup,
        SettingsField::ShowProfileIcons,
    ]
    .into_iter()
    .any(|field| settings_field_is_non_default(field, after, before))
}

#[cfg(test)]
mod tests {
    use super::{effective_settings_defaults, normalize_settings, settings_field_is_non_default};
    use crate::test_support::{EnvVarGuard, ENV_VAR_LOCK};
    use crate::{Core, SettingsField};
    use fleet_domain::{AppSettings, TelemetryPreference};

    #[test]
    fn effective_settings_defaults_matches_runtime_normalization() {
        let expected = normalize_settings(AppSettings::default());
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
    fn concurrent_settings_save_keeps_valid_state() {
        let _guard = ENV_VAR_LOCK.lock().expect("env lock");

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("FLEET_CONFIG_DIR", temp_dir.path());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()
            .expect("runtime");

        runtime.block_on(async {
            let core = Core::spawn_threaded_default().expect("core");
            let mut base = core.load_settings().await.expect("load settings");
            base.arma3.arma3_default_args = String::new();
            core.settings_save(base).await.expect("seed settings");

            let mut a = core.load_settings().await.expect("load A");
            a.startup.auto_check_profiles_on_startup = false;
            let mut b = core.load_settings().await.expect("load B");
            b.privacy.telemetry_consent = TelemetryPreference::Allowed;

            let (ra, rb) = tokio::join!(core.settings_save(a), core.settings_save(b));
            ra.expect("save A");
            rb.expect("save B");

            let final_settings = core.load_settings().await.expect("load final");
            assert!(!final_settings.arma3.arma3_default_args.trim().is_empty());
            assert!(matches!(
                final_settings.privacy.telemetry_consent,
                TelemetryPreference::Unset
                    | TelemetryPreference::Allowed
                    | TelemetryPreference::Denied
            ));
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
