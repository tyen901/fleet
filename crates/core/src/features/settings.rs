use crate::core::run_config_blocking;
use crate::Core;
use fleet_domain::{normalize_app_settings, AppSettings, ThemeMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsField {
    ReleaseChannel,
    ThemeMode,
    Arma3GameDir,
    Arma3LaunchMethod,
    Arma3CustomLaunchTemplate,
    Arma3DefaultArgs,
    TelemetryConsent,
    AutoCheckOnStartup,
    ShowProfileIcons,
}

pub fn effective_settings_defaults() -> AppSettings {
    normalize_settings(AppSettings::default())
}

struct SettingsFieldSpec {
    apply_default: fn(&mut AppSettings, &AppSettings),
    is_non_default: fn(&AppSettings, &AppSettings) -> bool,
}

fn settings_field_spec(field: SettingsField) -> SettingsFieldSpec {
    match field {
        SettingsField::ReleaseChannel => SettingsFieldSpec {
            apply_default: |settings, defaults| {
                settings.updates.release_channel = defaults.updates.release_channel;
            },
            is_non_default: |settings, defaults| {
                settings.updates.release_channel != defaults.updates.release_channel
            },
        },
        SettingsField::ThemeMode => SettingsFieldSpec {
            apply_default: |settings, defaults| {
                settings.appearance.theme_mode = defaults.appearance.theme_mode;
            },
            is_non_default: |settings, defaults| {
                settings.appearance.theme_mode != defaults.appearance.theme_mode
            },
        },
        SettingsField::Arma3GameDir => SettingsFieldSpec {
            apply_default: |settings, defaults| {
                settings.arma3.arma3_game_dir = defaults.arma3.arma3_game_dir.clone();
            },
            is_non_default: |settings, defaults| {
                settings.arma3.arma3_game_dir != defaults.arma3.arma3_game_dir
            },
        },
        SettingsField::Arma3LaunchMethod => SettingsFieldSpec {
            apply_default: |settings, defaults| {
                settings.arma3.arma3_launch_method = defaults.arma3.arma3_launch_method;
            },
            is_non_default: |settings, defaults| {
                settings.arma3.arma3_launch_method != defaults.arma3.arma3_launch_method
            },
        },
        SettingsField::Arma3CustomLaunchTemplate => SettingsFieldSpec {
            apply_default: |settings, defaults| {
                settings.arma3.arma3_custom_launch_template =
                    defaults.arma3.arma3_custom_launch_template.clone();
            },
            is_non_default: |settings, defaults| {
                settings.arma3.arma3_custom_launch_template
                    != defaults.arma3.arma3_custom_launch_template
            },
        },
        SettingsField::Arma3DefaultArgs => SettingsFieldSpec {
            apply_default: |settings, defaults| {
                settings.arma3.arma3_default_args = defaults.arma3.arma3_default_args.clone();
            },
            is_non_default: |settings, defaults| {
                settings.arma3.arma3_default_args != defaults.arma3.arma3_default_args
            },
        },
        SettingsField::TelemetryConsent => SettingsFieldSpec {
            apply_default: |settings, defaults| {
                settings.privacy.telemetry_consent = defaults.privacy.telemetry_consent;
            },
            is_non_default: |settings, defaults| {
                settings.privacy.telemetry_consent != defaults.privacy.telemetry_consent
            },
        },
        SettingsField::AutoCheckOnStartup => SettingsFieldSpec {
            apply_default: |settings, defaults| {
                settings.updates.auto_check_on_startup = defaults.updates.auto_check_on_startup;
            },
            is_non_default: |settings, defaults| {
                settings.updates.auto_check_on_startup != defaults.updates.auto_check_on_startup
            },
        },
        SettingsField::ShowProfileIcons => SettingsFieldSpec {
            apply_default: |settings, defaults| {
                settings.ui.show_profile_icons = defaults.ui.show_profile_icons;
            },
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

    pub async fn settings_set_theme_mode(
        &self,
        theme_mode: ThemeMode,
    ) -> Result<(), crate::ApiError> {
        let mut settings = self.read_state(|state| state.settings.clone());
        settings.appearance.theme_mode = theme_mode;
        self.settings_save(settings).await
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

fn apply_field_default(settings: &mut AppSettings, defaults: &AppSettings, field: SettingsField) {
    (settings_field_spec(field).apply_default)(settings, defaults);
}

fn settings_changed_after_normalize(before: &AppSettings, after: &AppSettings) -> bool {
    [
        SettingsField::ReleaseChannel,
        SettingsField::ThemeMode,
        SettingsField::Arma3GameDir,
        SettingsField::Arma3LaunchMethod,
        SettingsField::Arma3CustomLaunchTemplate,
        SettingsField::Arma3DefaultArgs,
        SettingsField::TelemetryConsent,
        SettingsField::AutoCheckOnStartup,
        SettingsField::ShowProfileIcons,
    ]
    .into_iter()
    .any(|field| settings_field_is_non_default(field, after, before))
        || before.sync.local_state_ignore_rules != after.sync.local_state_ignore_rules
}

#[cfg(test)]
mod tests {
    use super::{effective_settings_defaults, normalize_settings};
    use crate::test_support::{EnvVarGuard, ENV_VAR_LOCK};
    use crate::Core;
    use fleet_domain::{AppSettings, ReleaseChannel, TelemetryPreference, ThemeMode};

    #[test]
    fn effective_settings_defaults_matches_runtime_normalization() {
        let expected = normalize_settings(AppSettings::default());
        let actual = effective_settings_defaults();
        assert_eq!(actual.appearance.theme_mode, expected.appearance.theme_mode);
        assert_eq!(
            actual.arma3.arma3_default_args,
            expected.arma3.arma3_default_args
        );
        assert_eq!(
            actual.arma3.arma3_launch_method,
            expected.arma3.arma3_launch_method
        );
        assert_eq!(
            actual.sync.local_state_ignore_rules,
            expected.sync.local_state_ignore_rules
        );
    }

    #[test]
    fn normalize_settings_uses_default_theme_for_unknown_key() {
        let settings = AppSettings {
            appearance: fleet_domain::AppearanceSettings {
                theme_mode: serde_json::from_str::<ThemeMode>("\"legacy-dark\"")
                    .expect("theme deserialize"),
            },
            ..AppSettings::default()
        };
        let settings = normalize_settings(settings);
        assert_eq!(settings.appearance.theme_mode, ThemeMode::default());
    }

    #[test]
    fn reset_to_defaults_resets_only_settings() {
        let _guard = ENV_VAR_LOCK.lock().expect("env lock");

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
            changed.appearance.theme_mode = ThemeMode::Pluto;
            changed.updates.release_channel = ReleaseChannel::Dev;
            changed.privacy.telemetry_consent = TelemetryPreference::Denied;
            core.settings_save(changed).await.expect("save settings");

            core.reset_to_defaults().await.expect("reset defaults");

            let profiles = core.list_profiles().await.expect("list profiles");
            assert_eq!(profiles.profiles.len(), 1);

            let settings = core.load_settings().await.expect("load settings");
            let defaults = effective_settings_defaults();
            assert_eq!(
                settings.appearance.theme_mode,
                defaults.appearance.theme_mode
            );
            assert_eq!(
                settings.updates.release_channel,
                defaults.updates.release_channel
            );
            assert_eq!(
                settings.privacy.telemetry_consent,
                defaults.privacy.telemetry_consent
            );
        });
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
            a.updates.release_channel = ReleaseChannel::Dev;
            let mut b = core.load_settings().await.expect("load B");
            b.privacy.telemetry_consent = TelemetryPreference::Allowed;

            let (ra, rb) = tokio::join!(core.settings_save(a), core.settings_save(b));
            ra.expect("save A");
            rb.expect("save B");

            let final_settings = core.load_settings().await.expect("load final");
            assert!(!final_settings.arma3.arma3_default_args.trim().is_empty());
            assert!(matches!(
                final_settings.updates.release_channel,
                ReleaseChannel::Stable | ReleaseChannel::Dev
            ));
            assert!(matches!(
                final_settings.privacy.telemetry_consent,
                TelemetryPreference::Unset
                    | TelemetryPreference::Allowed
                    | TelemetryPreference::Denied
            ));
        });
    }
}
