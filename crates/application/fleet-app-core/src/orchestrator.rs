use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;

use crate::app_core::DomainEvent;
use crate::domain::{AppSettings, Profile};
use crate::pipeline::{PipelineRunEvent, PipelineRunId, PipelineStep, StepStatus};
use crate::ports::SyncPipelinePort;

use fleet_core::SyncPlan;
use fleet_pipeline::{
    DefaultSyncEngine, ProgressTracker, SyncMode, SyncOptions, SyncRequest, TransferSnapshot,
};

pub struct PipelineOrchestrator {
    engine: Arc<DefaultSyncEngine>,
    tx: mpsc::Sender<DomainEvent>,
    cancel: Option<CancellationToken>,
}

#[derive(Debug, Clone, Copy)]
enum CheckKind {
    LocalIntegrity,
    RemoteUpdate,
}

impl PipelineOrchestrator {
    pub fn new(engine: Arc<DefaultSyncEngine>, tx: mpsc::Sender<DomainEvent>) -> Self {
        Self {
            engine,
            tx,
            cancel: None,
        }
    }

    pub fn cancel(&mut self) {
        if let Some(token) = self.cancel.take() {
            token.cancel();
        }
    }

    pub fn start_local_integrity_check(
        &mut self,
        profile: Profile,
        settings: AppSettings,
        run_id: PipelineRunId,
    ) -> anyhow::Result<()> {
        self.start_check_worker(profile, settings, run_id, CheckKind::LocalIntegrity)
    }

    pub fn start_remote_update_check(
        &mut self,
        profile: Profile,
        settings: AppSettings,
        run_id: PipelineRunId,
    ) -> anyhow::Result<()> {
        self.start_check_worker(profile, settings, run_id, CheckKind::RemoteUpdate)
    }

    fn start_check_worker(
        &mut self,
        profile: Profile,
        settings: AppSettings,
        run_id: PipelineRunId,
        kind: CheckKind,
    ) -> anyhow::Result<()> {
        self.cancel();
        let token = CancellationToken::new();
        self.cancel = Some(token.clone());

        let tx = self.tx.clone();
        let engine = self.engine.clone();

        let thread_name = match kind {
            CheckKind::LocalIntegrity => "fleet-check-local",
            CheckKind::RemoteUpdate => "fleet-check-remote",
        };

        std::thread::Builder::new()
            .name(thread_name.into())
            .spawn(move || {
                let rt = match crate::async_runtime::runtime() {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = tx.blocking_send(DomainEvent::PipelineEvent {
                            run_id,
                            ev: PipelineRunEvent::Failed {
                                message: format!("Failed to start async runtime: {e}"),
                            },
                        });
                        return;
                    }
                };

                rt.block_on(async move {
                    let _ = tx
                        .send(DomainEvent::PipelineEvent {
                            run_id,
                            ev: PipelineRunEvent::Started {
                                profile_id: profile.id.clone(),
                            },
                        })
                        .await;

                    let root_path = std::path::Path::new(&profile.local_path);
                    let manifest_path = root_path.join(".fleet-local-manifest.json");
                    let summary_path = root_path.join(".fleet-local-summary.json");
                    let is_cold = !manifest_path.exists() || !summary_path.exists();

                    let mode = match kind {
                        CheckKind::LocalIntegrity => SyncMode::FastCheck,
                        CheckKind::RemoteUpdate => {
                            if is_cold {
                                SyncMode::SmartVerify
                            } else {
                                SyncMode::FastCheck
                            }
                        }
                    };

                    let options = SyncOptions {
                        max_threads: settings.max_threads,
                        rate_limit_bytes: None,
                        cache_root: None,
                    };

                    let req = SyncRequest {
                        repo_url: profile.repo_url.clone(),
                        local_root: camino::Utf8PathBuf::from(profile.local_path.clone()),
                        mode,
                        options,
                        profile_id: Some(profile.id.clone()),
                    };

                    let _ = tx
                        .send(DomainEvent::PipelineEvent {
                            run_id,
                            ev: PipelineRunEvent::StepChanged {
                                step: PipelineStep::Scan,
                                status: StepStatus::Running,
                                detail: "Scanning local files...".into(),
                            },
                        })
                        .await;

                    let tx_progress = tx.clone();
                    let on_progress = Box::new(move |stats| {
                        let _ = tx_progress.try_send(DomainEvent::PipelineEvent {
                            run_id,
                            ev: PipelineRunEvent::ScanStats { stats },
                        });
                    });

                    let local_res = tokio::select! {
                        _ = token.cancelled() => {
                            let _ = tx.send(DomainEvent::PipelineEvent { run_id, ev: PipelineRunEvent::Cancelled }).await;
                            return;
                        }
                        res = engine.scan_local_state(&req, Some(on_progress)) => res
                    };

                    let local_state = match local_res {
                        Ok(s) => {
                            let _ = tx
                                .send(DomainEvent::PipelineEvent {
                                    run_id,
                                    ev: PipelineRunEvent::StepChanged {
                                        step: PipelineStep::Scan,
                                        status: StepStatus::Succeeded,
                                        detail: "Scan complete".into(),
                                    },
                                })
                                .await;
                            s
                        }
                        Err(e) => {
                            let _ = tx
                                .send(DomainEvent::PipelineEvent {
                                    run_id,
                                    ev: PipelineRunEvent::Failed {
                                        message: e.to_string(),
                                    },
                                })
                                .await;
                            return;
                        }
                    };

                    let existing_mods = || {
                        std::fs::read_dir(&profile.local_path)
                            .ok()
                            .into_iter()
                            .flatten()
                            .filter_map(|res| res.ok())
                            .filter_map(|entry| {
                                let path = entry.path();
                                if !path.is_dir() {
                                    return None;
                                }
                                let name = path.file_name()?.to_string_lossy().to_string();
                                if name.starts_with('@') {
                                    Some(name)
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                    };

                    match kind {
                        CheckKind::LocalIntegrity => {
                            let _ = tx
                                .send(DomainEvent::PipelineEvent {
                                    run_id,
                                    ev: PipelineRunEvent::StepChanged {
                                        step: PipelineStep::Fetch,
                                        status: StepStatus::Skipped,
                                        detail: "Using cached local state".into(),
                                    },
                                })
                                .await;

                            let _ = tx
                                .send(DomainEvent::PipelineEvent {
                                    run_id,
                                    ev: PipelineRunEvent::StepChanged {
                                        step: PipelineStep::Diff,
                                        status: StepStatus::Running,
                                        detail: "Checking local integrity...".into(),
                                    },
                                })
                                .await;

                            let plan_res = engine.compute_local_integrity_plan(&req, &local_state);
                            match plan_res {
                                Ok(plan) => {
                                    let diff_stats = (plan.downloads.len(), plan.deletes.len());
                                    let _ = tx
                                        .send(DomainEvent::PipelineEvent {
                                            run_id,
                                            ev: PipelineRunEvent::PlanReady {
                                                plan,
                                                diff_stats,
                                                existing_mods: existing_mods(),
                                            },
                                        })
                                        .await;
                                }
                                Err(e) => {
                                    let _ = tx
                                        .send(DomainEvent::PipelineEvent {
                                            run_id,
                                            ev: PipelineRunEvent::Failed {
                                                message: e.to_string(),
                                            },
                                        })
                                        .await;
                                }
                            }
                            return;
                        }
                        CheckKind::RemoteUpdate => {}
                    }

                    let _ = tx
                        .send(DomainEvent::PipelineEvent {
                            run_id,
                            ev: PipelineRunEvent::StepChanged {
                                step: PipelineStep::Fetch,
                                status: StepStatus::Running,
                                detail: "Fetching manifest...".into(),
                            },
                        })
                        .await;

                    let fetch_res = tokio::select! {
                        _ = token.cancelled() => {
                            let _ = tx.send(DomainEvent::PipelineEvent { run_id, ev: PipelineRunEvent::Cancelled }).await;
                            return;
                        }
                        res = engine.fetch_remote_state(&req) => res
                    };

                    let fetch_res = match fetch_res {
                        Ok(r) => {
                            let _ = tx
                                .send(DomainEvent::PipelineEvent {
                                    run_id,
                                    ev: PipelineRunEvent::StepChanged {
                                        step: PipelineStep::Fetch,
                                        status: StepStatus::Succeeded,
                                        detail: "Manifest fetched".into(),
                                    },
                                })
                                .await;
                            r
                        }
                        Err(e) => {
                            let _ = tx
                                .send(DomainEvent::PipelineEvent {
                                    run_id,
                                    ev: PipelineRunEvent::Failed {
                                        message: e.to_string(),
                                    },
                                })
                                .await;
                            return;
                        }
                    };

                    let _ = tx
                        .send(DomainEvent::PipelineEvent {
                            run_id,
                            ev: PipelineRunEvent::StepChanged {
                                step: PipelineStep::Diff,
                                status: StepStatus::Running,
                                detail: "Analyzing...".into(),
                            },
                        })
                        .await;

                    let plan_res = engine.compute_plan(&fetch_res.manifest, &local_state, &req);
                    match plan_res {
                        Ok(plan) => {
                            let diff_stats = (plan.downloads.len(), plan.deletes.len());
                            let _ = tx
                                .send(DomainEvent::PipelineEvent {
                                    run_id,
                                    ev: PipelineRunEvent::PlanReady {
                                        plan,
                                        diff_stats,
                                        existing_mods: existing_mods(),
                                    },
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(DomainEvent::PipelineEvent {
                                    run_id,
                                    ev: PipelineRunEvent::Failed {
                                        message: e.to_string(),
                                    },
                                })
                                .await;
                        }
                    }
                });
            })
            .context("Failed to spawn background check worker thread")?;

        Ok(())
    }

    pub fn start_sync(
        &mut self,
        profile: Profile,
        plan: SyncPlan,
        settings: AppSettings,
        run_id: PipelineRunId,
    ) -> anyhow::Result<()> {
        self.cancel();
        let token = CancellationToken::new();
        self.cancel = Some(token.clone());

        let tx = self.tx.clone();
        let engine = self.engine.clone();

        std::thread::Builder::new()
            .name("fleet-sync".into())
            .spawn(move || {
                let rt = match crate::async_runtime::runtime() {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = tx.blocking_send(DomainEvent::PipelineEvent {
                            run_id,
                            ev: PipelineRunEvent::Failed {
                                message: format!("Failed to start async runtime: {e}"),
                            },
                        });
                        return;
                    }
                };

                rt.block_on(async move {
                    let _ = tx
                        .send(DomainEvent::PipelineEvent {
                            run_id,
                            ev: PipelineRunEvent::Started {
                                profile_id: profile.id.clone(),
                            },
                        })
                        .await;

                    let req = SyncRequest {
                        repo_url: profile.repo_url.clone(),
                        local_root: camino::Utf8PathBuf::from(profile.local_path.clone()),
                        mode: SyncMode::CacheOnly,
                        options: SyncOptions {
                            max_threads: settings.max_threads,
                            rate_limit_bytes: if settings.speed_limit_enabled {
                                Some(settings.max_speed_bytes)
                            } else {
                                None
                            },
                            cache_root: None,
                        },
                        profile_id: Some(profile.id.clone()),
                    };

                    let _ = tx
                        .send(DomainEvent::PipelineEvent {
                            run_id,
                            ev: PipelineRunEvent::StepChanged {
                                step: PipelineStep::Execute,
                                status: StepStatus::Running,
                                detail: "Synchronizing content...".into(),
                            },
                        })
                        .await;

                    let (prog_tx, mut prog_rx) = mpsc::channel(100);
                    let mut tracker = ProgressTracker::new(&plan);
                    let mut latest: Option<TransferSnapshot> = None;
                    let mut ticker = interval(Duration::from_millis(100));

                    let work_fut = engine.execute_with_plan(&req, plan.clone(), Some(prog_tx));

                    tokio::pin!(work_fut);

                    loop {
                        tokio::select! {
                            _ = token.cancelled() => {
                                let _ = tx.send(DomainEvent::PipelineEvent { run_id, ev: PipelineRunEvent::Cancelled }).await;
                                return;
                            }
                            res = &mut work_fut => {
                                if let Some(snap) = latest.take() {
                                    let _ = tx.send(DomainEvent::PipelineEvent { run_id, ev: PipelineRunEvent::TransferProgress { snapshot: snap } }).await;
                                }
                                match res {
                                    Ok(_r) => {
                                        let _ = tx.send(DomainEvent::PipelineEvent { run_id, ev: PipelineRunEvent::Completed }).await;
                                    }
                                    Err(e) => {
                                        let _ = tx.send(DomainEvent::PipelineEvent { run_id, ev: PipelineRunEvent::Failed { message: e.to_string() } }).await;
                                    }
                                }
                                break;
                            }
                            maybe_ev = prog_rx.recv() => {
                                if let Some(ev) = maybe_ev {
                                    tracker.update(ev);
                                    latest = Some(tracker.get_snapshot());
                                }
                            }
                            _ = ticker.tick() => {
                                if let Some(snap) = latest.clone() {
                                    let _ = tx.try_send(DomainEvent::PipelineEvent { run_id, ev: PipelineRunEvent::TransferProgress { snapshot: snap } });
                                }
                            }
                        }
                    }

                    let scan_req = SyncRequest {
                        repo_url: profile.repo_url.clone(),
                        local_root: camino::Utf8PathBuf::from(profile.local_path.clone()),
                        mode: SyncMode::SmartVerify,
                        options: SyncOptions {
                            max_threads: settings.max_threads,
                            rate_limit_bytes: None,
                            cache_root: None,
                        },
                        profile_id: Some(profile.id.clone()),
                    };

                    let latest_stats: Arc<std::sync::Mutex<Option<fleet_scanner::ScanStats>>> =
                        Arc::new(std::sync::Mutex::new(None));
                    let stats_ref = latest_stats.clone();
                    let on_progress = Box::new(move |stats: fleet_scanner::ScanStats| {
                        if let Ok(mut guard) = stats_ref.lock() {
                            *guard = Some(stats);
                        }
                    });

                    let _ = engine.scan_local_state(&scan_req, Some(on_progress)).await;
                    let stats_opt = latest_stats.lock().ok().and_then(|guard| guard.clone());
                    if let Some(stats) = stats_opt {
                        let _ = tx
                            .send(DomainEvent::PipelineEvent {
                                run_id,
                                ev: PipelineRunEvent::ScanStats { stats },
                            })
                            .await;
                    }
                });
            })
            .context("Failed to spawn background sync worker thread")?;

        Ok(())
    }
}

impl SyncPipelinePort for PipelineOrchestrator {
    fn validate_repo_url_blocking(&self, repo_url: &str) -> anyhow::Result<()> {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let repo = repo_url.to_string();
            let engine = self.engine.clone();
            tokio::task::block_in_place(move || {
                handle.block_on(async move { engine.validate_repo_url(&repo).await })
            })?;
        } else {
            crate::async_runtime::runtime()?.block_on(self.engine.validate_repo_url(repo_url))?;
        }
        Ok(())
    }
}
