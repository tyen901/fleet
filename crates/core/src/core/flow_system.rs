use super::flow_logging::{
    log_operation_cancel_requested, log_operation_cancel_unknown, log_operation_cleanup,
    log_operation_rejected_duplicate, log_operation_spawn_requested, log_operation_started,
    log_terminal_result, operation_kind_label,
};
use fleet_domain::health::OperationKind;
use fleet_domain::{ApiError, Profile, ProfileId, INVENTORY_REBUILD_REQUIRED_CODE};
use fleet_flow::{EventSink, FlowConfig, FlowEventKind, FlowResult, FlowSessionEvent};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct FlowSystem {
    events_tx: broadcast::Sender<FlowSessionEvent>,
    next_session_id: Arc<AtomicU64>,
    sessions: Arc<Mutex<Sessions>>,
}

struct Sessions {
    by_session: HashMap<u64, Session>,
    by_profile: HashMap<ProfileId, u64>,
}

struct Session {
    profile_id: ProfileId,
    operation: OperationKind,
    cancel: CancellationToken,
}

impl FlowSystem {
    pub fn new() -> Self {
        let (events_tx, _) = broadcast::channel(1024);
        Self {
            events_tx,
            next_session_id: Arc::new(AtomicU64::new(1)),
            sessions: Arc::new(Mutex::new(Sessions {
                by_session: HashMap::new(),
                by_profile: HashMap::new(),
            })),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<FlowSessionEvent> {
        self.events_tx.subscribe()
    }

    pub async fn spawn_operation_with_config(
        &self,
        cfg: FlowConfig,
        profile: Profile,
        operation: OperationKind,
    ) -> anyhow::Result<u64> {
        self.spawn_operation(cfg, profile.id.clone(), operation, move |ctx| async move {
            match operation {
                OperationKind::Sync => {
                    let summary = fleet_flow::flows::operation::run_sync_flow(
                        ctx.cfg, profile, ctx.cancel, ctx.sink,
                    )
                    .await?;
                    Ok(FlowResult::Sync(summary))
                }
                OperationKind::Repair => {
                    let summary = fleet_flow::flows::operation::run_repair_flow(
                        ctx.cfg, profile, ctx.cancel, ctx.sink,
                    )
                    .await?;
                    Ok(FlowResult::Repair(summary))
                }
                OperationKind::CheckLocal => {
                    let report = fleet_flow::flows::assess::run_assess_flow_with_sink(
                        ctx.cfg, profile, false, ctx.cancel, ctx.sink,
                    )
                    .await?;
                    Ok(FlowResult::Check(report))
                }
                OperationKind::RebuildInventory => {
                    let report = fleet_flow::flows::operation::run_rebuild_inventory_flow(
                        ctx.cfg, profile, ctx.cancel, ctx.sink,
                    )
                    .await?;
                    Ok(FlowResult::RebuildInventory(report))
                }
                OperationKind::CheckRemote => {
                    let report = fleet_flow::flows::assess::run_assess_flow_with_sink(
                        ctx.cfg, profile, true, ctx.cancel, ctx.sink,
                    )
                    .await?;
                    Ok(FlowResult::Check(report))
                }
                OperationKind::Clean => {
                    let report = fleet_flow::flows::operation::run_clean_flow(
                        ctx.cfg, profile, ctx.cancel, ctx.sink,
                    )
                    .await?;
                    Ok(FlowResult::Clean(report))
                }
            }
        })
        .await
    }

    pub async fn spawn_clean_operation_with_config(
        &self,
        cfg: FlowConfig,
        profile: Profile,
        remove_empty_parent_dirs: bool,
    ) -> anyhow::Result<u64> {
        self.spawn_operation(
            cfg,
            profile.id.clone(),
            OperationKind::Clean,
            move |ctx| async move {
                let report = fleet_flow::flows::operation::run_clean_flow_with_options(
                    ctx.cfg,
                    profile,
                    ctx.cancel,
                    ctx.sink,
                    fleet_flow::flows::operation::CleanFlowOptions {
                        remove_empty_parent_dirs,
                    },
                )
                .await?;
                Ok(FlowResult::Clean(report))
            },
        )
        .await
    }

    pub fn cancel_session(&self, session_id: u64) -> bool {
        let session_meta = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .by_session
                .get(&session_id)
                .map(|s| (s.profile_id.clone(), s.operation, s.cancel.clone()))
        };

        if let Some((profile_id, operation_kind, cancel)) = session_meta {
            log_operation_cancel_requested(&profile_id, operation_kind, session_id);
            cancel.cancel();
            true
        } else {
            log_operation_cancel_unknown(session_id);
            false
        }
    }

    async fn spawn_operation<F, Fut>(
        &self,
        cfg: FlowConfig,
        profile_id: ProfileId,
        operation: OperationKind,
        run: F,
    ) -> anyhow::Result<u64>
    where
        F: FnOnce(SpawnCtx) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = anyhow::Result<FlowResult>> + Send + 'static,
    {
        let session_id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        log_operation_spawn_requested(&profile_id, operation, session_id);

        let cancel = CancellationToken::new();

        {
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.by_profile.contains_key(&profile_id) {
                log_operation_rejected_duplicate(&profile_id, operation);
                anyhow::bail!("an operation is already running for this profile");
            }
            sessions.by_profile.insert(profile_id.clone(), session_id);
            sessions.by_session.insert(
                session_id,
                Session {
                    profile_id: profile_id.clone(),
                    operation,
                    cancel: cancel.clone(),
                },
            );
        }

        let _ = self.events_tx.send(FlowSessionEvent::new(
            session_id,
            profile_id.clone(),
            operation,
            FlowEventKind::Started,
        ));
        log_operation_started(&profile_id, operation, session_id);

        let sink: Arc<dyn EventSink> = Arc::new(PipelineSink {
            events_tx: self.events_tx.clone(),
            session_id,
            profile_id: profile_id.clone(),
            operation,
        });

        let sessions = self.sessions.clone();
        let events_tx = self.events_tx.clone();
        let operation_kind = operation_kind_label(operation);

        tokio::spawn(async move {
            let result = run(SpawnCtx {
                cfg,
                cancel: cancel.clone(),
                sink,
            })
            .await;

            {
                let mut guard = sessions.lock().unwrap();
                if let Some(session) = guard.by_session.remove(&session_id) {
                    guard.by_profile.remove(&session.profile_id);
                }
            }
            log_operation_cleanup(&profile_id, operation, session_id);

            let terminal_kind = match result {
                Ok(result) => {
                    log_terminal_result(&profile_id, operation, session_id, "finished", None);
                    FlowEventKind::Finished { result }
                }
                Err(e) => {
                    if cancel.is_cancelled() {
                        log_terminal_result(
                            &profile_id,
                            operation,
                            session_id,
                            "canceled",
                            Some("cancel_requested"),
                        );
                        FlowEventKind::Canceled
                    } else {
                        log_terminal_result(
                            &profile_id,
                            operation,
                            session_id,
                            "failed",
                            Some("flow_error"),
                        );
                        tracing::debug!(
                            flow_kind = operation_kind,
                            profile_id = %profile_id,
                            session_id = session_id,
                            op = "terminal",
                            outcome = "failed",
                            error = %e,
                            "flow terminal error details"
                        );
                        FlowEventKind::Failed {
                            error: map_flow_error(&e),
                        }
                    }
                }
            };

            let _ = events_tx.send(FlowSessionEvent::new(
                session_id,
                profile_id,
                operation,
                terminal_kind,
            ));
        });

        Ok(session_id)
    }
}

fn map_flow_error(err: &anyhow::Error) -> ApiError {
    if err
        .chain()
        .filter_map(|cause| cause.downcast_ref::<inventory::Error>())
        .any(inventory::Error::is_corrupted_database)
    {
        return ApiError::new(
            INVENTORY_REBUILD_REQUIRED_CODE,
            inventory::REBUILD_REQUIRED_MESSAGE,
        );
    }

    ApiError::new("pipeline_error", format!("{err:#}"))
}

struct SpawnCtx {
    cfg: FlowConfig,
    cancel: CancellationToken,
    sink: Arc<dyn EventSink>,
}

struct PipelineSink {
    events_tx: broadcast::Sender<FlowSessionEvent>,
    session_id: u64,
    profile_id: ProfileId,
    operation: OperationKind,
}

impl EventSink for PipelineSink {
    fn emit(&self, kind: FlowEventKind) {
        let _ = self.events_tx.send(FlowSessionEvent::new(
            self.session_id,
            self.profile_id.clone(),
            self.operation,
            kind,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::FlowSystem;
    use fleet_domain::health::OperationKind;
    use fleet_flow::{FlowConfig, FlowEventKind, FlowResult, FlowSessionEvent};
    use tokio::sync::oneshot;

    fn test_cfg() -> FlowConfig {
        FlowConfig::new_default()
    }

    async fn recv_matching(
        rx: &mut tokio::sync::broadcast::Receiver<FlowSessionEvent>,
        session_id: u64,
    ) -> FlowSessionEvent {
        loop {
            let ev = rx.recv().await.expect("event");
            if ev.session_id == session_id {
                return ev;
            }
        }
    }

    #[tokio::test]
    async fn spawn_operation_rejects_duplicate_profile_session() {
        let system = FlowSystem::new();
        let mut first_rx = system.subscribe();
        let cfg = test_cfg();
        let profile_id = "p1".to_string();
        let (tx, rx) = oneshot::channel::<()>();

        let session_id = system
            .spawn_operation(
                cfg,
                profile_id.clone(),
                OperationKind::Sync,
                move |_| async move {
                    let _ = rx.await;
                    Ok(FlowResult::Sync(fleet_domain::sync::SyncSummary {
                        profile_id: "p1".to_string(),
                        destination: String::new(),
                        manifest_source: String::new(),
                        duration_ms: 0,
                        bytes_reused: 0,
                        bytes_downloaded: 0,
                        files_finalized: 0,
                    }))
                },
            )
            .await
            .expect("session started");

        let duplicate = system
            .spawn_operation(
                FlowConfig::new_default(),
                profile_id,
                OperationKind::Repair,
                |_| async move { anyhow::bail!("not used") },
            )
            .await;
        assert!(duplicate.is_err());

        tx.send(()).expect("release");
        let _started = recv_matching(&mut first_rx, session_id).await;
        let terminal = recv_matching(&mut first_rx, session_id).await;
        assert!(matches!(terminal.kind, FlowEventKind::Finished { .. }));
    }

    #[tokio::test]
    async fn canceled_flow_emits_canceled_terminal_event() {
        let system = FlowSystem::new();
        let mut rx = system.subscribe();
        let session_id = system
            .spawn_operation(
                test_cfg(),
                "p2".to_string(),
                OperationKind::Sync,
                move |ctx| async move {
                    ctx.cancel.cancelled().await;
                    anyhow::bail!("canceled")
                },
            )
            .await
            .expect("session started");

        let _started = recv_matching(&mut rx, session_id).await;
        system.cancel_session(session_id);
        let terminal = recv_matching(&mut rx, session_id).await;
        assert!(matches!(terminal.kind, FlowEventKind::Canceled));
    }

    #[tokio::test]
    async fn failed_flow_emits_failed_terminal_event() {
        let system = FlowSystem::new();
        let mut rx = system.subscribe();
        let session_id = system
            .spawn_operation(
                test_cfg(),
                "p3".to_string(),
                OperationKind::Repair,
                |_| async move { anyhow::bail!("boom") },
            )
            .await
            .expect("session started");

        let _started = recv_matching(&mut rx, session_id).await;
        let terminal = recv_matching(&mut rx, session_id).await;
        let FlowEventKind::Failed { error } = terminal.kind else {
            panic!("expected failed terminal event");
        };
        assert_eq!(error.code, "pipeline_error");
        assert!(error.message.contains("boom"));
    }

    #[tokio::test]
    async fn corrupted_inventory_flow_emits_rebuild_required_error() {
        let system = FlowSystem::new();
        let mut rx = system.subscribe();
        let session_id = system
            .spawn_operation(
                test_cfg(),
                "p3".to_string(),
                OperationKind::Sync,
                |_| async move {
                    Err(anyhow::Error::new(inventory::Error::CorruptedDatabase(
                        "legacy inventory schema is no longer supported".to_string(),
                    )))
                },
            )
            .await
            .expect("session started");

        let _started = recv_matching(&mut rx, session_id).await;
        let terminal = recv_matching(&mut rx, session_id).await;
        let FlowEventKind::Failed { error } = terminal.kind else {
            panic!("expected failed terminal event");
        };
        assert_eq!(error.code, "inventory_rebuild_required");
        assert_eq!(error.message, inventory::REBUILD_REQUIRED_MESSAGE);
    }

    #[tokio::test]
    async fn check_operation_rejects_duplicate_for_same_profile() {
        let system = FlowSystem::new();
        let cfg = test_cfg();
        let profile_id = "p4".to_string();
        let (tx, rx) = oneshot::channel::<()>();

        let first_session = system
            .spawn_operation(
                cfg,
                profile_id.clone(),
                OperationKind::CheckLocal,
                move |_| async move {
                    let _ = rx.await;
                    Ok(FlowResult::Check(
                        fleet_domain::health::ProfileAssessmentReport {
                            profile_id: "p4".to_string(),
                            local_health: fleet_domain::health::LocalHealthState::Ready,
                            remote_freshness:
                                fleet_domain::health::RemoteFreshnessState::NotRelevant,
                            checked_at_unix_ms: 0,
                            expected_missing_in_inventory_count: 0,
                            inventory_unexpected_paths_count: 0,
                            unexpected_delete_paths: Vec::new(),
                        },
                    ))
                },
            )
            .await
            .expect("first check started");
        assert!(first_session > 0);

        let second_session = system
            .spawn_operation(
                FlowConfig::new_default(),
                profile_id,
                OperationKind::CheckLocal,
                |_| async move {
                    Ok(FlowResult::Check(
                        fleet_domain::health::ProfileAssessmentReport {
                            profile_id: "p4".to_string(),
                            local_health: fleet_domain::health::LocalHealthState::Ready,
                            remote_freshness:
                                fleet_domain::health::RemoteFreshnessState::NotRelevant,
                            checked_at_unix_ms: 0,
                            expected_missing_in_inventory_count: 0,
                            inventory_unexpected_paths_count: 0,
                            unexpected_delete_paths: Vec::new(),
                        },
                    ))
                },
            )
            .await;
        assert!(second_session.is_err());

        tx.send(()).expect("release first check");
    }
}
