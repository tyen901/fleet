use fleet_core::{AppState, Core};
use tokio::sync::watch;

#[derive(Clone)]
pub struct FleetBridge {
    core: Core,
    pub state_rx: watch::Receiver<AppState>,
}

impl FleetBridge {
    pub fn new() -> anyhow::Result<Self> {
        let core = Core::spawn_threaded_default()?;
        let state_rx = core.subscribe_state();
        Ok(Self { core, state_rx })
    }

    pub fn core(&self) -> Core {
        self.core.clone()
    }

    pub fn get_snapshot(&self) -> AppState {
        self.state_rx.borrow().clone()
    }
}
