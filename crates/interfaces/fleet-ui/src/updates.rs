use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use velopack::{sources, UpdateCheck, UpdateManager};

const DEFAULT_UPDATE_URL: &str = "https://github.com/tyen901/fleet/releases/latest/download";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    UpdateAvailable { version: String },
    Downloading,
    Applying,
    Error { message: String },
}

#[derive(Clone, Debug)]
pub enum UpdateEvent {
    State(UpdateState),
}

pub fn update_feed_url() -> String {
    std::env::var("FLEET_UPDATE_URL").unwrap_or_else(|_| DEFAULT_UPDATE_URL.to_owned())
}

pub fn build_version_string() -> &'static str {
    option_env!("FLEET_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

pub fn installed_version_string() -> String {
    UpdateManager::new(sources::NoneSource {}, None, None)
        .map(|um| um.get_current_version_as_string())
        .unwrap_or_else(|_| build_version_string().to_owned())
}

pub struct UpdateClient {
    feed_url: String,
    busy: Arc<AtomicBool>,
    tx: mpsc::Sender<UpdateEvent>,
}

impl UpdateClient {
    pub fn new(feed_url: String) -> (Self, mpsc::Receiver<UpdateEvent>) {
        let (tx, rx) = mpsc::channel();
        (
            Self {
                feed_url,
                busy: Arc::new(AtomicBool::new(false)),
                tx,
            },
            rx,
        )
    }

    pub fn start_check(&self) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }

        let feed_url = self.feed_url.clone();
        let tx = self.tx.clone();
        let busy = self.busy.clone();

        thread::spawn(move || {
            let _ = tx.send(UpdateEvent::State(UpdateState::Checking));

            let state = match UpdateManager::new(sources::HttpSource::new(feed_url), None, None) {
                Ok(um) => match um.check_for_updates() {
                    Ok(UpdateCheck::UpdateAvailable(update)) => UpdateState::UpdateAvailable {
                        version: update.TargetFullRelease.Version,
                    },
                    Ok(UpdateCheck::NoUpdateAvailable) | Ok(UpdateCheck::RemoteIsEmpty) => {
                        UpdateState::UpToDate
                    }
                    Err(e) => UpdateState::Error {
                        message: e.to_string(),
                    },
                },
                Err(e) => UpdateState::Error {
                    message: e.to_string(),
                },
            };

            let _ = tx.send(UpdateEvent::State(state));
            busy.store(false, Ordering::SeqCst);
        });
    }

    pub fn start_apply(&self) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }

        let feed_url = self.feed_url.clone();
        let tx = self.tx.clone();
        let busy = self.busy.clone();

        thread::spawn(move || {
            let _ = tx.send(UpdateEvent::State(UpdateState::Downloading));

            let um = match UpdateManager::new(sources::HttpSource::new(feed_url), None, None) {
                Ok(um) => um,
                Err(e) => {
                    send_error(&tx, &busy, e.to_string());
                    return;
                }
            };

            let update = match um.check_for_updates() {
                Ok(UpdateCheck::UpdateAvailable(update)) => update,
                Ok(UpdateCheck::NoUpdateAvailable) | Ok(UpdateCheck::RemoteIsEmpty) => {
                    let _ = tx.send(UpdateEvent::State(UpdateState::UpToDate));
                    busy.store(false, Ordering::SeqCst);
                    return;
                }
                Err(e) => {
                    send_error(&tx, &busy, e.to_string());
                    return;
                }
            };

            if let Err(e) = um.download_updates(&update, None) {
                send_error(&tx, &busy, e.to_string());
                return;
            }

            let _ = tx.send(UpdateEvent::State(UpdateState::Applying));
            if let Err(e) = um.apply_updates_and_restart(&update) {
                send_error(&tx, &busy, e.to_string());
                return;
            }

            busy.store(false, Ordering::SeqCst);
        });
    }
}

fn send_error(tx: &mpsc::Sender<UpdateEvent>, busy: &Arc<AtomicBool>, message: String) {
    let _ = tx.send(UpdateEvent::State(UpdateState::Error { message }));
    busy.store(false, Ordering::SeqCst);
}
