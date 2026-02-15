use super::flow_logging::{
    flow_kind_label, log_session_cancel_requested, log_session_cancel_unknown, log_session_cleanup,
    log_session_input_rejected, log_session_input_routed, log_session_rejected_duplicate,
    log_session_spawn_requested, log_session_started, log_terminal_result,
};
use fleet_domain::{Profile, ProfileId};
use fleet_flow::{
    EventSink, FlowConfig, FlowEventKind, FlowInput, FlowKind, FlowResult, FlowSessionEvent,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};
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
    flow: FlowKind,
    cancel: CancellationToken,
    input_tx: mpsc::Sender<FlowInput>,
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

    pub async fn spawn_sync_with_config(
        &self,
        cfg: FlowConfig,
        profile: Profile,
    ) -> anyhow::Result<u64> {
        self.spawn_flow(
            cfg,
            profile.id.clone(),
            FlowKind::Sync,
            move |ctx| async move {
                let summary = fleet_flow::flows::operation::run_sync_flow(
                    ctx.cfg,
                    profile,
                    ctx.cancel,
                    ctx.input_rx,
                    ctx.sink,
                )
                .await?;
                Ok(FlowResult::Sync(summary))
            },
        )
        .await
    }

    pub async fn spawn_repair_with_config(
        &self,
        cfg: FlowConfig,
        profile: Profile,
    ) -> anyhow::Result<u64> {
        self.spawn_flow(
            cfg,
            profile.id.clone(),
            FlowKind::Repair,
            move |ctx| async move {
                let summary = fleet_flow::flows::operation::run_repair_flow(
                    ctx.cfg,
                    profile,
                    ctx.cancel,
                    ctx.input_rx,
                    ctx.sink,
                )
                .await?;
                Ok(FlowResult::Repair(summary))
            },
        )
        .await
    }

    pub async fn spawn_check_with_config(
        &self,
        cfg: FlowConfig,
        profile: Profile,
        include_remote: bool,
    ) -> anyhow::Result<u64> {
        self.spawn_flow(
            cfg,
            profile.id.clone(),
            FlowKind::Check,
            move |ctx| async move {
                let report = fleet_flow::flows::assess::run_assess_flow(
                    ctx.cfg,
                    profile,
                    include_remote,
                    ctx.cancel,
                )
                .await?;
                Ok(FlowResult::Check(report))
            },
        )
        .await
    }

    pub async fn send_input(&self, session_id: u64, input: FlowInput) -> anyhow::Result<()> {
        let session_meta = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .by_session
                .get(&session_id)
                .map(|s| (s.profile_id.clone(), s.flow, s.input_tx.clone()))
        }
        .ok_or_else(|| anyhow::anyhow!("unknown session"));

        let (profile_id, flow_kind, input_tx) = match session_meta {
            Ok(v) => v,
            Err(err) => {
                log_session_input_rejected(session_id, "send_input", "unknown_session");
                return Err(err);
            }
        };

        input_tx
            .send(input)
            .await
            .map_err(|_| anyhow::anyhow!("session input channel closed"))
            .inspect_err(|_err| {
                log_session_input_rejected(session_id, "send_input", "session_input_closed");
            })?;

        log_session_input_routed(&profile_id, flow_kind, session_id, "send_input");

        Ok(())
    }

    pub fn cancel_session(&self, session_id: u64) {
        let session_meta = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .by_session
                .get(&session_id)
                .map(|s| (s.profile_id.clone(), s.flow, s.cancel.clone()))
        };

        if let Some((profile_id, flow_kind, cancel)) = session_meta {
            log_session_cancel_requested(&profile_id, flow_kind, session_id);
            cancel.cancel();
        } else {
            log_session_cancel_unknown(session_id);
        }
    }

    async fn spawn_flow<F, Fut>(
        &self,
        cfg: FlowConfig,
        profile_id: ProfileId,
        flow: FlowKind,
        run: F,
    ) -> anyhow::Result<u64>
    where
        F: FnOnce(SpawnCtx) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = anyhow::Result<FlowResult>> + Send + 'static,
    {
        let session_id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        log_session_spawn_requested(&profile_id, flow, session_id);

        let cancel = CancellationToken::new();
        let (input_tx, input_rx) = mpsc::channel(16);

        {
            let mut sessions = self.sessions.lock().unwrap();
            if let Some(existing_session_id) = sessions.by_profile.get(&profile_id).copied() {
                let should_preempt_check = sessions
                    .by_session
                    .get(&existing_session_id)
                    .is_some_and(|existing| existing.flow == FlowKind::Check);

                if should_preempt_check {
                    if let Some(existing) = sessions.by_session.remove(&existing_session_id) {
                        existing.cancel.cancel();
                        sessions.by_profile.remove(&profile_id);
                        tracing::info!(
                            flow_kind = flow_kind_label(flow),
                            profile_id = %profile_id,
                            session_id = existing_session_id,
                            op = "spawn",
                            outcome = "preempted",
                            reason = "check_preempted_by_new_session",
                            "preempted existing check flow for new session"
                        );
                    }
                } else {
                    log_session_rejected_duplicate(&profile_id, flow);
                    anyhow::bail!("a flow is already running for this profile");
                }
            }
            sessions.by_profile.insert(profile_id.clone(), session_id);
            sessions.by_session.insert(
                session_id,
                Session {
                    profile_id: profile_id.clone(),
                    flow,
                    cancel: cancel.clone(),
                    input_tx,
                },
            );
        }

        let _ = self.events_tx.send(FlowSessionEvent::new(
            session_id,
            profile_id.clone(),
            flow,
            FlowEventKind::Started,
        ));
        log_session_started(&profile_id, flow, session_id);

        let sink: Arc<dyn EventSink> = Arc::new(PipelineSink {
            events_tx: self.events_tx.clone(),
            session_id,
            profile_id: profile_id.clone(),
            flow,
        });

        let sessions = self.sessions.clone();
        let events_tx = self.events_tx.clone();
        let flow_kind = flow_kind_label(flow);

        tokio::spawn(async move {
            let result = run(SpawnCtx {
                cfg,
                cancel: cancel.clone(),
                input_rx,
                sink,
            })
            .await;

            {
                let mut guard = sessions.lock().unwrap();
                if let Some(session) = guard.by_session.remove(&session_id) {
                    guard.by_profile.remove(&session.profile_id);
                }
            }
            log_session_cleanup(&profile_id, flow, session_id);

            let terminal_kind = match result {
                Ok(result) => {
                    log_terminal_result(&profile_id, flow, session_id, "finished", None);
                    FlowEventKind::Finished { result }
                }
                Err(e) => {
                    if cancel.is_cancelled() {
                        log_terminal_result(
                            &profile_id,
                            flow,
                            session_id,
                            "canceled",
                            Some("cancel_requested"),
                        );
                        FlowEventKind::Canceled
                    } else {
                        log_terminal_result(
                            &profile_id,
                            flow,
                            session_id,
                            "failed",
                            Some("flow_error"),
                        );
                        tracing::debug!(
                            flow_kind = flow_kind,
                            profile_id = %profile_id,
                            session_id = session_id,
                            op = "terminal",
                            outcome = "failed",
                            error = %e,
                            "flow terminal error details"
                        );
                        FlowEventKind::Failed {
                            error: format!("{e:#}"),
                        }
                    }
                }
            };

            let _ = events_tx.send(FlowSessionEvent::new(
                session_id,
                profile_id,
                flow,
                terminal_kind,
            ));
        });

        Ok(session_id)
    }
}

struct SpawnCtx {
    cfg: FlowConfig,
    cancel: CancellationToken,
    input_rx: mpsc::Receiver<FlowInput>,
    sink: Arc<dyn EventSink>,
}

struct PipelineSink {
    events_tx: broadcast::Sender<FlowSessionEvent>,
    session_id: u64,
    profile_id: ProfileId,
    flow: FlowKind,
}

impl EventSink for PipelineSink {
    fn emit(&self, kind: FlowEventKind) {
        let _ = self.events_tx.send(FlowSessionEvent::new(
            self.session_id,
            self.profile_id.clone(),
            self.flow,
            kind,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::FlowSystem;
    use fleet_flow::{FlowConfig, FlowEventKind, FlowKind, FlowResult, FlowSessionEvent};
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
    async fn spawn_flow_rejects_duplicate_non_check_profile_session() {
        let system = FlowSystem::new();
        let mut first_rx = system.subscribe();
        let cfg = test_cfg();
        let profile_id = "p1".to_string();
        let (tx, rx) = oneshot::channel::<()>();

        let session_id = system
            .spawn_flow(
                cfg,
                profile_id.clone(),
                FlowKind::Sync,
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
            .spawn_flow(
                FlowConfig::new_default(),
                profile_id,
                FlowKind::Repair,
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
            .spawn_flow(
                test_cfg(),
                "p2".to_string(),
                FlowKind::Sync,
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
            .spawn_flow(
                test_cfg(),
                "p3".to_string(),
                FlowKind::Repair,
                |_| async move { anyhow::bail!("boom") },
            )
            .await
            .expect("session started");

        let _started = recv_matching(&mut rx, session_id).await;
        let terminal = recv_matching(&mut rx, session_id).await;
        assert!(matches!(terminal.kind, FlowEventKind::Failed { .. }));
    }

    #[tokio::test]
    async fn check_flow_preempts_existing_check_for_same_profile() {
        let system = FlowSystem::new();
        let cfg = test_cfg();
        let profile_id = "p4".to_string();
        let (tx, rx) = oneshot::channel::<()>();

        let first_session = system
            .spawn_flow(
                cfg,
                profile_id.clone(),
                FlowKind::Check,
                move |_| async move {
                    let _ = rx.await;
                    Ok(FlowResult::Check(
                        fleet_domain::health::ProfileAssessmentReport {
                            profile_id: "p4".to_string(),
                            local_health: fleet_domain::health::LocalHealthState::Ready,
                            remote_freshness:
                                fleet_domain::health::RemoteFreshnessState::NotRelevant,
                            checked_at_unix_ms: 0,
                            unexpected_delete_paths: Vec::new(),
                        },
                    ))
                },
            )
            .await
            .expect("first check started");

        let second_session = system
            .spawn_flow(
                FlowConfig::new_default(),
                profile_id,
                FlowKind::Check,
                |_| async move {
                    Ok(FlowResult::Check(
                        fleet_domain::health::ProfileAssessmentReport {
                            profile_id: "p4".to_string(),
                            local_health: fleet_domain::health::LocalHealthState::Ready,
                            remote_freshness:
                                fleet_domain::health::RemoteFreshnessState::NotRelevant,
                            checked_at_unix_ms: 0,
                            unexpected_delete_paths: Vec::new(),
                        },
                    ))
                },
            )
            .await
            .expect("second check started");

        assert_ne!(first_session, second_session);
        tx.send(()).expect("release first check");
    }
}
