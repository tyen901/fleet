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
        let input_tx = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .by_session
                .get(&session_id)
                .map(|s| s.input_tx.clone())
        }
        .ok_or_else(|| anyhow::anyhow!("unknown session"))?;

        input_tx
            .send(input)
            .await
            .map_err(|_| anyhow::anyhow!("session input channel closed"))?;

        Ok(())
    }

    pub fn cancel_session(&self, session_id: u64) {
        let cancel = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .by_session
                .get(&session_id)
                .map(|s| s.cancel.clone())
        };
        if let Some(cancel) = cancel {
            cancel.cancel();
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

        let cancel = CancellationToken::new();
        let (input_tx, input_rx) = mpsc::channel(16);

        {
            let mut sessions = self.sessions.lock().unwrap();
            if sessions.by_profile.contains_key(&profile_id) {
                anyhow::bail!("a flow is already running for this profile");
            }
            sessions.by_profile.insert(profile_id.clone(), session_id);
            sessions.by_session.insert(
                session_id,
                Session {
                    profile_id: profile_id.clone(),
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

        let sink: Arc<dyn EventSink> = Arc::new(PipelineSink {
            events_tx: self.events_tx.clone(),
            session_id,
            profile_id: profile_id.clone(),
            flow,
        });

        let sessions = self.sessions.clone();
        let events_tx = self.events_tx.clone();

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

            let terminal_kind = match result {
                Ok(result) => FlowEventKind::Finished { result },
                Err(e) => {
                    if cancel.is_cancelled() {
                        FlowEventKind::Canceled
                    } else {
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
