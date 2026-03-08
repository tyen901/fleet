mod await_session;
pub(crate) mod flow_logging;
mod flow_system;
mod runtime;

use crate::state::AppState;
use crate::storage::{profile_state_root_dir, ConfigRepo};
use fleet_domain::AppSettings;
use fleet_flow::FlowConfig;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, watch};

pub use flow_system::FlowSystem;

#[derive(Clone)]
pub struct Core {
    inner: Arc<CoreInner>,
}

struct CoreInner {
    flow: FlowSystem,
    config: Arc<ConfigRepo>,
    state: Mutex<AppState>,
    state_tx: watch::Sender<AppState>,
    startup_auto_check_enabled: bool,
}

impl Core {
    pub fn spawn_threaded_default() -> anyhow::Result<Self> {
        let core = Self::new_default()?;
        runtime::spawn_threaded(core.clone());
        Ok(core)
    }

    pub fn new_in_current_runtime_default() -> anyhow::Result<Self> {
        let core = Self::new_default_with_startup_checks(true)?;
        runtime::spawn_in_current(core.clone());
        Ok(core)
    }

    pub fn new_in_current_runtime_without_startup_checks() -> anyhow::Result<Self> {
        let core = Self::new_default_with_startup_checks(false)?;
        runtime::spawn_in_current(core.clone());
        Ok(core)
    }

    pub fn subscribe_state(&self) -> watch::Receiver<AppState> {
        self.inner.state_tx.subscribe()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<fleet_flow::FlowSessionEvent> {
        self.inner.flow.subscribe()
    }

    pub(crate) fn flow(&self) -> FlowSystem {
        self.inner.flow.clone()
    }

    pub(crate) fn config_repo(&self) -> Arc<ConfigRepo> {
        self.inner.config.clone()
    }

    pub(crate) fn update_state<F>(&self, f: F)
    where
        F: FnOnce(&mut AppState),
    {
        let mut guard = self.inner.state.lock().unwrap();
        f(&mut guard);
        publish_state(&mut guard, &self.inner.state_tx);
    }

    pub(crate) fn read_state<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&AppState) -> R,
    {
        let guard = self.inner.state.lock().unwrap();
        f(&guard)
    }

    pub(crate) fn replace_state(&self, state: AppState) {
        let mut guard = self.inner.state.lock().unwrap();
        *guard = state;
        publish_state(&mut guard, &self.inner.state_tx);
    }

    fn new_default() -> anyhow::Result<Self> {
        Self::new_default_with_startup_checks(true)
    }

    fn new_default_with_startup_checks(startup_auto_check_enabled: bool) -> anyhow::Result<Self> {
        let config = Arc::new(ConfigRepo::new_default()?);
        let flow = FlowSystem::new();
        let (state_tx, _state_rx) = watch::channel(AppState::default());
        let state = Mutex::new(AppState::default());

        Ok(Self {
            inner: Arc::new(CoreInner {
                flow,
                config,
                state,
                state_tx,
                startup_auto_check_enabled,
            }),
        })
    }

    pub(crate) fn flow_config_from_settings(settings: &AppSettings) -> FlowConfig {
        let mut cfg = FlowConfig::new_default();
        if let Ok(root) = profile_state_root_dir() {
            cfg.profile_state_root_dir = root;
        }
        cfg.local_state_config.ignore_rules_text = settings.sync.local_state_ignore_rules.clone();
        cfg
    }

    pub(crate) fn current_flow_config(&self) -> FlowConfig {
        self.read_state(|state| Self::flow_config_from_settings(&state.settings))
    }
}

pub(crate) fn publish_state(state: &mut AppState, state_tx: &watch::Sender<AppState>) {
    state.version = state.version.wrapping_add(1);
    let _ = state_tx.send(state.clone());
}

pub(crate) async fn run_config_blocking<T>(
    repo: Arc<ConfigRepo>,
    f: impl FnOnce(&ConfigRepo) -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(move || f(&repo)).await?
}
