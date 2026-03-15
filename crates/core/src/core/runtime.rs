use super::operation_scheduler::{dispatch_auto_check, AutoCheckCoalescer};
use super::state_projection::{
    apply_event, should_refresh_profile_inventory_metrics, should_refresh_profile_repo_cache,
};
use super::{publish_state, Core};
use crate::state::AppState;
use crate::storage::config_root_dir;
use std::collections::BTreeMap;
use tokio::sync::broadcast;
use tracing::warn;

pub(crate) fn spawn_threaded(core: Core) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        rt.block_on(async move {
            run_core_loop(core).await;
        });
    });
}

pub(crate) fn spawn_in_current(core: Core) {
    tokio::spawn(async move {
        run_core_loop(core).await;
    });
}

async fn run_core_loop(core: Core) {
    let mut auto_check = AutoCheckCoalescer::default();

    let initial = match load_initial_state(&core).await {
        Ok(state) => state,
        Err(err) => {
            warn!(error = %err, "failed to load initial state; using defaults");
            AppState::default()
        }
    };
    let profile_ids: Vec<_> = initial.profiles.keys().cloned().collect();
    let auto_check_on_startup =
        initial.settings.startup.auto_assess_on_startup && initial.settings.ui.onboarding_completed;
    core.replace_state(initial);
    for profile_id in &profile_ids {
        core.spawn_profile_repo_cache_refresh(profile_id.clone(), false);
        core.spawn_profile_inventory_metrics_refresh(profile_id.clone());
        if auto_check_on_startup {
            auto_check.enqueue(profile_id, fleet_domain::health::OperationKind::CheckRepo);
            auto_check.enqueue(
                profile_id,
                fleet_domain::health::OperationKind::CheckInventory,
            );
        }
    }
    dispatch_auto_check(&core, &mut auto_check).await;

    let mut rx = core.pipeline().subscribe();

    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        };

        let now = ev.timestamp_ms;
        {
            let mut guard = core.inner.state.lock().unwrap();
            apply_event(&mut guard, &ev, now);
            auto_check.observe_event(&ev);
            publish_state(&mut guard, &core.inner.state_tx);
        }

        if should_refresh_profile_repo_cache(&ev) {
            core.spawn_profile_repo_cache_refresh(ev.profile_id.clone(), true);
        }
        if should_refresh_profile_inventory_metrics(&ev) {
            core.spawn_profile_inventory_metrics_refresh(ev.profile_id.clone());
        }
        dispatch_auto_check(&core, &mut auto_check).await;
    }
}

async fn load_initial_state(core: &Core) -> anyhow::Result<AppState> {
    let profiles_cfg =
        super::run_config_blocking(core.config_repo(), |c| c.load_profiles()).await?;
    let settings = core.load_settings().await?;
    let mut profiles = BTreeMap::new();
    for p in profiles_cfg.profiles {
        profiles.insert(p.id.clone(), p);
    }

    if let Ok(config_root) = config_root_dir() {
        let _ = std::fs::remove_file(config_root.join("runtime_state.json"));
    }

    let now = fleet_domain::time::now_unix_ms();
    let mut profile_runtime_by_id = BTreeMap::new();
    for (profile_id, profile) in profiles.iter() {
        let mut runtime = crate::state::ProfileRuntimeState::new(
            profile_id.clone(),
            now,
            !profile.source.trim().is_empty(),
        );
        crate::features::profiles::seed_missing_destination_inventory_hint(
            &mut runtime,
            profile,
            now,
        );
        profile_runtime_by_id.insert(profile_id.clone(), runtime);
    }

    Ok(AppState {
        version: 0,
        settings,
        profiles,
        selected_profile_id: None,
        profile_runtime_by_id,
    })
}
