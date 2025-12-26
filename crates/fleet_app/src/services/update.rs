//! Application update service and model.
//!
//! The update service provides a pull‑based interface around the
//! `velopack` update mechanism.  It owns an authoritative [`UpdateModel`]
//! and exposes a [`snapshot`] method to obtain the current update state.
//! The update process is asynchronous; state transitions (e.g. checking,
//! downloading, idle, failed) are written directly into the model.  No
//! update progress events are delivered to the UI via streams or logs.

use serde::Serialize;
use std::sync::{Arc, RwLock};

use crate::app::AppError;

use velopack::{UpdateCheck as VeloUpdateCheck, UpdateInfo};

/// Discriminated update state as consumed by the UI.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UpdateState {
    NotConfigured,
    Idle { status: String },
    Checking,
    Downloading { progress: Option<f32> },
    Failed { error: String },
}

/// Authoritative model for update state.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateModel {
    pub state: UpdateState,
    pub available: Option<UpdateInfo>,
}

/// Interface for performing application update checks and applying updates.
pub trait UpdateService: Send + Sync {
    /// Return a snapshot of the current update model.
    fn snapshot(&self) -> Arc<UpdateModel>;
    /// Begin an update check.  This method must return immediately; progress
    /// will be reflected in subsequent snapshots.
    fn check(&self) -> Result<(), AppError>;
    /// Begin applying a previously downloaded update.  This method must
    /// return immediately; progress will be reflected in subsequent snapshots.
    fn apply(&self) -> Result<(), AppError>;
    /// Clear the last recorded error from the model.
    fn clear_error(&self);
}

/// Concrete update service implementation.
pub struct FleetUpdateService {
    tokio: tokio::runtime::Handle,
    model: Arc<RwLock<Arc<UpdateModel>>>,
}

impl FleetUpdateService {
    pub fn new(tokio: tokio::runtime::Handle) -> Arc<Self> {
        // Determine whether updates are configured via the environment.
        let configured = update_base_url().is_some();
        let initial_state = if configured {
            UpdateState::Idle {
                status: "Not checked".to_string(),
            }
        } else {
            UpdateState::NotConfigured
        };
        let model = UpdateModel {
            state: initial_state,
            available: None,
        };
        Arc::new(Self {
            tokio,
            model: Arc::new(RwLock::new(Arc::new(model))),
        })
    }

    fn set_failed(model: &mut UpdateModel, msg: impl Into<String>) {
        // The backend does not provide a dedicated update error type in
        // `AppError`.  Represent update failures using the `InvalidInput`
        // variant to convey the error message to the UI.
        model.state = UpdateState::Failed { error: msg.into() };
    }
}

impl UpdateService for FleetUpdateService {
    fn snapshot(&self) -> Arc<UpdateModel> {
        self.model.read().expect("lock poisoned").clone()
    }

    fn check(&self) -> Result<(), AppError> {
        // If not configured, reflect directly in the model.
        let Some(base_url) = update_base_url() else {
            with_model_mut(&self.model, |model| {
                model.state = UpdateState::NotConfigured;
                model.available = None;
            });
            return Ok(());
        };
        let should_start = {
            let model = self.model.read().expect("lock poisoned");
            !matches!(
                model.state,
                UpdateState::Checking | UpdateState::Downloading { .. }
            )
        };
        if !should_start {
            return Ok(());
        }
        with_model_mut(&self.model, |model| {
            model.state = UpdateState::Checking;
        });
        let model = Arc::clone(&self.model);
        self.tokio.spawn_blocking(move || {
            let res = (|| -> Result<VeloUpdateCheck, String> {
                let source = velopack::sources::HttpSource::new(&base_url);
                let um =
                    velopack::UpdateManager::new(source, None, None).map_err(|e| e.to_string())?;
                um.check_for_updates().map_err(|e| e.to_string())
            })();
            with_model_mut(&model, |m| match res {
                Err(err) => Self::set_failed(m, err),
                Ok(VeloUpdateCheck::RemoteIsEmpty | VeloUpdateCheck::NoUpdateAvailable) => {
                    m.available = None;
                    m.state = UpdateState::Idle {
                        status: "No update available".to_string(),
                    };
                }
                Ok(VeloUpdateCheck::UpdateAvailable(info)) => {
                    m.available = Some(info);
                    m.state = UpdateState::Idle {
                        status: "Update available".to_string(),
                    };
                }
            });
        });
        Ok(())
    }

    fn apply(&self) -> Result<(), AppError> {
        // If not configured, reflect directly in the model.
        let Some(base_url) = update_base_url() else {
            with_model_mut(&self.model, |model| {
                model.state = UpdateState::NotConfigured;
                model.available = None;
            });
            return Ok(());
        };
        let info_opt = {
            let model = self.model.read().expect("lock poisoned");
            if matches!(
                model.state,
                UpdateState::Checking | UpdateState::Downloading { .. }
            ) {
                return Ok(());
            }
            model.available.clone()
        };
        if info_opt.is_none() {
            with_model_mut(&self.model, |model| {
                model.state = UpdateState::Idle {
                    status: "No update to apply".to_string(),
                };
                model.available = None;
            });
            return Ok(());
        }
        with_model_mut(&self.model, |model| {
            model.state = UpdateState::Downloading { progress: None };
        });
        let model = Arc::clone(&self.model);
        self.tokio.spawn_blocking(move || {
            let Some(info) = info_opt else {
                return;
            };
            // Kick off update download and apply.
            let res = (|| -> Result<(), String> {
                let source = velopack::sources::HttpSource::new(&base_url);
                let um =
                    velopack::UpdateManager::new(source, None, None).map_err(|e| e.to_string())?;
                let (ptx, prx) = std::sync::mpsc::channel::<i16>();
                // Spawn progress thread: update progress into model.
                std::thread::spawn({
                    let model = Arc::clone(&model);
                    move || {
                        while let Ok(p) = prx.recv() {
                            let progress = if p < 0 { None } else { Some(p as f32 / 100.0) };
                            with_model_mut(&model, |m| {
                                if let UpdateState::Downloading {
                                    progress: ref mut dest,
                                } = &mut m.state
                                {
                                    *dest = progress;
                                }
                            });
                        }
                    }
                });
                // Download and then request apply/exit.  Use `download_updates` for progress
                // notifications and then ask the manager to apply and exit the process.
                um.download_updates(&info, Some(ptx))
                    .map_err(|e| e.to_string())?;
                um.apply_updates_and_exit(&info)
                    .map_err(|e| e.to_string())?;
                Ok(())
            })();
            // Clear busy flag regardless of outcome.
            with_model_mut(&model, |m| {
                match res {
                    Ok(()) => {
                        // Apply success; the process will exit before the UI can update further.
                        m.state = UpdateState::Idle {
                            status: "Update applied".to_string(),
                        };
                    }
                    Err(err) => {
                        Self::set_failed(m, err);
                    }
                }
                // Clear available update on apply.
                m.available = None;
            });
        });
        Ok(())
    }

    fn clear_error(&self) {
        with_model_mut(&self.model, |m| {
            if let UpdateState::Failed { .. } = m.state {
                // Reset to idle without available update.
                m.state = UpdateState::Idle {
                    status: "Not checked".to_string(),
                };
                m.available = None;
            }
        });
    }
}

/// Determine the base URL for updates.  This mimics the logic from the UI
/// update service.  The environment variable `FLEET_UPDATE_URL` takes
/// precedence; otherwise we consult the compile‑time `option_env!`.
pub fn update_base_url() -> Option<String> {
    fn normalize(s: String) -> String {
        let mut t = s.trim().to_string();
        while t.ends_with('/') {
            t.pop();
        }
        t
    }
    if let Ok(u) = std::env::var("FLEET_UPDATE_URL") {
        let u = normalize(u);
        if !u.is_empty() {
            return Some(u);
        }
    }
    if let Some(u) = option_env!("FLEET_UPDATE_URL") {
        let u = normalize(u.to_string());
        if !u.is_empty() {
            return Some(u);
        }
    }
    None
}

fn with_model_mut<M: Clone>(slot: &RwLock<Arc<M>>, f: impl FnOnce(&mut M)) {
    let mut guard = slot.write().unwrap_or_else(|e| e.into_inner());
    let mut next = (**guard).clone();
    f(&mut next);
    *guard = Arc::new(next);
}
