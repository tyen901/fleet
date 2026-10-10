use super::Core;
use crate::state::AppState;
use std::collections::BTreeMap;
use tracing::warn;

#[derive(Clone, Copy)]
pub(crate) enum StartupPolicy {
    Desktop,
    ExplicitCommand,
}

pub(crate) fn spawn_threaded(core: Core) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move { run_core_loop(core, StartupPolicy::Desktop).await });
    });
}

pub(crate) fn spawn_in_current(core: Core, startup_policy: StartupPolicy) {
    tokio::spawn(async move { run_core_loop(core, startup_policy).await });
}

async fn run_core_loop(core: Core, startup_policy: StartupPolicy) {
    let initial = match load_initial_state(&core).await {
        Ok(state) => state,
        Err(err) => {
            warn!(error = %err, "failed to load initial state; using defaults");
            AppState::default()
        }
    };
    let profile_ids: Vec<_> = initial.profiles.keys().cloned().collect();
    let auto_check_on_startup = initial.settings.startup.auto_check_profiles_on_startup;
    core.replace_state(initial);

    for profile_id in &profile_ids {
        core.spawn_profile_repo_cache_refresh(profile_id.clone());
        if auto_check_on_startup && matches!(startup_policy, StartupPolicy::Desktop) {
            let core_for_checks = core.clone();
            let profile_id_for_checks = profile_id.clone();
            tokio::spawn(async move {
                if let Ok(session) = core_for_checks
                    .start_operation(
                        profile_id_for_checks,
                        fleet_domain::health::OperationKind::Check,
                    )
                    .await
                {
                    let _ = core_for_checks.await_finished(session).await;
                }
            });
        }
    }

    // Keep the core Tokio runtime alive for background work spawned above.
    std::future::pending::<()>().await;
}

async fn load_initial_state(core: &Core) -> anyhow::Result<AppState> {
    let profiles_cfg =
        super::run_config_blocking(core.config_repo(), |c| c.load_profiles()).await?;
    let settings = core.load_settings().await?;
    let mut profiles = BTreeMap::new();
    for p in profiles_cfg.profiles {
        profiles.insert(p.id.clone(), p);
    }

    let now = fleet_domain::time::now_unix_ms();
    let mut profile_runtime_by_id = BTreeMap::new();
    for profile_id in profiles.keys() {
        let runtime = crate::state::ProfileRuntimeState::new(profile_id.clone(), now);
        profile_runtime_by_id.insert(profile_id.clone(), runtime);
    }

    Ok(AppState {
        version: 0,
        settings,
        profiles,
        profile_runtime_by_id,
    })
}
