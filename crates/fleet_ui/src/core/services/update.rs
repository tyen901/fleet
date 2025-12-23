use crate::core::types::{AppError, RequestId};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use velopack::{UpdateCheck, UpdateInfo};

#[derive(Debug, Clone)]
pub enum UpdateState {
    NotConfigured,
    Idle {
        status: String,
        available: Option<UpdateInfo>,
    },
    Checking {
        request: RequestId,
    },
    Downloading {
        request: RequestId,
        progress: Option<f32>,
    },
    Failed {
        error: AppError,
    },
}

#[derive(Debug, Clone)]
pub struct UpdateSnapshot {
    pub state: UpdateState,
}

pub trait UpdateService: Send + Sync {
    fn snapshot(&self) -> UpdateSnapshot;
    fn check(&self) -> RequestId;
    fn apply(&self) -> RequestId;
    fn clear_error(&self);
}

fn normalize_base_url(s: String) -> String {
    let mut t = s.trim().to_string();
    while t.ends_with('/') {
        t.pop();
    }
    t
}

pub fn update_base_url() -> Option<String> {
    if let Ok(u) = std::env::var("FLEET_UPDATE_URL") {
        let u = normalize_base_url(u);
        if !u.is_empty() {
            return Some(u);
        }
    }
    if let Some(u) = option_env!("FLEET_UPDATE_URL") {
        let u = normalize_base_url(u.to_string());
        if !u.is_empty() {
            return Some(u);
        }
    }
    None
}

struct UpdateInner {
    snap: UpdateSnapshot,
    // last successful check result
    available: Option<UpdateInfo>,
    busy: bool,
}

pub struct FleetUpdateService {
    tokio: tokio::runtime::Handle,
    req: AtomicU64,
    inner: Arc<RwLock<UpdateInner>>,
}

impl FleetUpdateService {
    pub fn new(tokio: tokio::runtime::Handle) -> Arc<Self> {
        let configured = update_base_url().is_some();
        Arc::new(Self {
            tokio,
            req: AtomicU64::new(1),
            inner: Arc::new(RwLock::new(UpdateInner {
                snap: UpdateSnapshot {
                    state: if configured {
                        UpdateState::Idle {
                            status: "Not checked".into(),
                            available: None,
                        }
                    } else {
                        UpdateState::NotConfigured
                    },
                },
                available: None,
                busy: false,
            })),
        })
    }

    fn set_failed(inner: &mut UpdateInner, msg: impl Into<String>) {
        inner.busy = false;
        inner.snap.state = UpdateState::Failed {
            error: AppError::new("update_failed", msg.into()),
        };
    }
}

impl UpdateService for FleetUpdateService {
    fn snapshot(&self) -> UpdateSnapshot {
        self.inner.read().snap.clone()
    }

    fn check(&self) -> RequestId {
        let request = RequestId(self.req.fetch_add(1, Ordering::Relaxed));
        let Some(base_url) = update_base_url() else {
            let mut inner = self.inner.write();
            inner.snap.state = UpdateState::NotConfigured;
            return request;
        };

        {
            let mut inner = self.inner.write();
            if inner.busy {
                return request;
            }
            inner.busy = true;
            inner.snap.state = UpdateState::Checking { request };
        }

        let inner = Arc::clone(&self.inner);

        self.tokio.spawn_blocking(move || {
            let res = (|| -> Result<UpdateCheck, String> {
                let source = velopack::sources::HttpSource::new(&base_url);
                let um =
                    velopack::UpdateManager::new(source, None, None).map_err(|e| e.to_string())?;
                um.check_for_updates().map_err(|e| e.to_string())
            })();

            let mut guard = inner.write();
            guard.busy = false;

            match res {
                Err(e) => Self::set_failed(&mut guard, e),
                Ok(UpdateCheck::RemoteIsEmpty | UpdateCheck::NoUpdateAvailable) => {
                    guard.available = None;
                    guard.snap.state = UpdateState::Idle {
                        status: "No update available".into(),
                        available: None,
                    };
                }
                Ok(UpdateCheck::UpdateAvailable(info)) => {
                    guard.available = Some(info.clone());
                    guard.snap.state = UpdateState::Idle {
                        status: "Update available".into(),
                        available: Some(info),
                    };
                }
            }
        });

        request
    }

    fn apply(&self) -> RequestId {
        let request = RequestId(self.req.fetch_add(1, Ordering::Relaxed));
        let Some(base_url) = update_base_url() else {
            let mut inner = self.inner.write();
            inner.snap.state = UpdateState::NotConfigured;
            return request;
        };

        let info = {
            let mut inner = self.inner.write();
            if inner.busy {
                return request;
            }
            let Some(info) = inner.available.clone() else {
                inner.snap.state = UpdateState::Idle {
                    status: "No update to apply".into(),
                    available: None,
                };
                return request;
            };
            inner.busy = true;
            inner.snap.state = UpdateState::Downloading {
                request,
                progress: None,
            };
            info
        };

        let inner = Arc::clone(&self.inner);

        self.tokio.spawn_blocking(move || {
            let res = (|| -> Result<(), String> {
                let source = velopack::sources::HttpSource::new(&base_url);
                let um =
                    velopack::UpdateManager::new(source, None, None).map_err(|e| e.to_string())?;

                let (ptx, prx) = std::sync::mpsc::channel::<i16>();
                std::thread::spawn({
                    let inner = Arc::clone(&inner);
                    move || {
                        for p in prx {
                            let p = (p as i32).clamp(0, 100) as f32 / 100.0;
                            let mut guard = inner.write();
                            if let UpdateState::Downloading { request, .. } =
                                guard.snap.state.clone()
                            {
                                guard.snap.state = UpdateState::Downloading {
                                    request,
                                    progress: Some(p),
                                };
                            }
                        }
                    }
                });

                um.download_updates(&info, Some(ptx))
                    .map_err(|e| e.to_string())?;
                um.apply_updates_and_restart(&info)
                    .map_err(|e| e.to_string())?;
                Ok(())
            })();

            let mut guard = inner.write();
            guard.busy = false;

            if let Err(e) = res {
                Self::set_failed(&mut guard, e);
            }
        });

        request
    }

    fn clear_error(&self) {
        let mut inner = self.inner.write();
        if matches!(inner.snap.state, UpdateState::Failed { .. }) {
            inner.snap.state = UpdateState::Idle {
                status: "Not checked".into(),
                available: inner.available.clone(),
            };
        }
    }
}
