//! High level UI/CLI services built around [`FleetApp`].
//!
//! These services implement the pull‑based snapshot contract described in the
//! Fleet UI architecture.  Each service owns exactly one authoritative model
//! (protected by an [`Arc<RwLock<…>`]) and exposes an inexpensive
//! [`snapshot()`] method to obtain a cheap clone of the current model.  The
//! models contain all state necessary for the UI to render without listening
//! to any event streams or logs.  Commands that mutate the model must update
//! the underlying model immediately; there are no event queues or
//! asynchronous state propagations to the UI.

use std::sync::Arc;

use crate::app::AppError;

pub mod data;
pub mod sync;
pub mod update;

/// A bundle of all services constructed from a single [`FleetApp`].
///
/// `FleetServices` groups together references to the [`DataService`],
/// [`SyncService`], and [`UpdateService`].  Each service is wrapped in
/// `Arc<dyn …>` so it can be cloned cheaply and passed throughout the UI.
pub struct FleetServices {
    pub data: Arc<dyn data::DataService>,
    pub sync: Arc<dyn sync::SyncService>,
    pub update: Arc<dyn update::UpdateService>,
}

/// Open the default registry and construct a full [`FleetServices`] bundle.
///
/// This helper uses [`crate::app::FleetApp::open_default_with_recovery`] to
/// initialise the backend.  It then wraps the resulting [`FleetApp`] in a
/// single `Arc<RwLock<…>` and constructs concrete service implementations
/// around it.  See [`data::FleetDataService`], [`sync::FleetSyncService`]
/// and [`update::FleetUpdateService`] for details.
pub fn open_default_with_recovery(
    rt: tokio::runtime::Handle,
) -> Result<(FleetServices, Option<String>), AppError> {
    let (app, warning) = crate::app::FleetApp::open_default_with_recovery();
    let app = Arc::new(std::sync::RwLock::new(app));

    // Construct concrete implementations.  These types are defined in the
    // submodules of this module.
    let data: Arc<dyn data::DataService> =
        data::FleetDataService::new(Arc::clone(&app), warning.clone());
    let sync = sync::FleetSyncService::new(Arc::clone(&app), rt.clone(), Arc::clone(&data));
    let update = update::FleetUpdateService::new(rt.clone());

    Ok((FleetServices { data, sync, update }, warning))
}
