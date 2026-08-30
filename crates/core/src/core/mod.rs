mod await_session;
mod runtime;

use crate::operations::{OperationRuntime, OperationSessionEvent};
use crate::state::{ensure_profile_runtime_mut, AppState};
use crate::storage::ConfigRepo;
use fleet_domain::health::{CancelResult, OperationKind};
use fleet_domain::{ApiError, ProfileId};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, watch};

#[derive(Clone)]
pub struct Core {
    pub(crate) inner: Arc<CoreInner>,
}

pub(crate) struct CoreInner {
    operations: OperationRuntime,
    next_session_id: AtomicU64,
    config: Arc<ConfigRepo>,
    state: Mutex<AppState>,
    state_tx: watch::Sender<AppState>,
}

impl Core {
    pub fn spawn_threaded_default() -> anyhow::Result<Self> {
        let core = Self::new_default()?;
        runtime::spawn_threaded(core.clone());
        Ok(core)
    }

    pub fn new_in_current_runtime_default() -> anyhow::Result<Self> {
        let core = Self::new_default()?;
        runtime::spawn_in_current(core.clone());
        Ok(core)
    }

    pub fn subscribe_state(&self) -> watch::Receiver<AppState> {
        self.inner.state_tx.subscribe()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<OperationSessionEvent> {
        self.inner.operations.subscribe()
    }

    pub(crate) fn operation_runtime(&self) -> OperationRuntime {
        self.inner.operations.clone()
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
        let config = Arc::new(ConfigRepo::new_default()?);
        let operations = OperationRuntime::new();
        let (state_tx, _state_rx) = watch::channel(AppState::default());
        let state = Mutex::new(AppState::default());

        Ok(Self {
            inner: Arc::new(CoreInner {
                operations,
                next_session_id: AtomicU64::new(1),
                config,
                state,
                state_tx,
            }),
        })
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> anyhow::Result<Self> {
        Self::new_default()
    }

    pub(crate) fn allocate_session_id(&self) -> u64 {
        self.inner.next_session_id.fetch_add(1, Ordering::Relaxed)
    }

    pub async fn start_operations_for_profiles(
        &self,
        profile_ids: Vec<ProfileId>,
        operations: Vec<OperationKind>,
    ) -> Vec<(ProfileId, OperationKind, ApiError)> {
        let mut failures = Vec::new();
        for profile_id in profile_ids {
            for operation in operations.iter().copied() {
                if let Err(err) = self.start_operation(profile_id.clone(), operation).await {
                    failures.push((profile_id.clone(), operation, err));
                }
            }
        }
        failures
    }

    pub async fn start_operation(
        &self,
        profile_id: ProfileId,
        operation: OperationKind,
    ) -> Result<u64, ApiError> {
        self.ensure_profile_loaded_for_start(&profile_id).await?;
        self.operation_runtime()
            .start(self.clone(), profile_id, operation)
    }

    pub fn cancel_session(&self, session_id: u64) -> Result<CancelResult, ApiError> {
        Ok(self.operation_runtime().cancel(session_id))
    }

    async fn ensure_profile_loaded_for_start(&self, profile_id: &str) -> Result<(), ApiError> {
        if self.read_state(|state| state.profiles.contains_key(profile_id)) {
            return Ok(());
        }

        let profile_id_owned = profile_id.to_string();
        let profile = self
            .load_profile(&profile_id_owned)
            .await
            .map_err(|e| ApiError::new("not_found", e.to_string()))?;

        let profile_id_owned = profile.id.clone();
        let profile_for_state = profile.clone();
        self.update_state(|state| {
            state
                .profiles
                .insert(profile_id_owned.clone(), profile_for_state);
            let now = fleet_domain::time::now_unix_ms();
            let _ = ensure_profile_runtime_mut(state, &profile_id_owned, now);
            if let Some(runtime) = state.profile_runtime_by_id.get_mut(&profile_id_owned) {
                runtime.recompute_status();
            }
        });
        Ok(())
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
