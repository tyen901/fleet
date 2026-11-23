mod scanner;
use crate::inventory::Inventory;
use flux::{LocalFile, Outcome, Request};
use relative_path::RelativePath;
use std::{
    collections::BTreeMap,
    future::Future,
    io,
    sync::{atomic::AtomicU64, Arc},
};
use tokio_util::sync::CancellationToken;
#[derive(Debug, thiserror::Error)]
pub enum InspectionError {
    #[error("inventory: {0}")]
    Inventory(String),
    #[error(transparent)]
    File(#[from] io::Error),
    #[error(transparent)]
    Operation(#[from] flux::Error),
    #[error(transparent)]
    Worker(#[from] tokio::task::JoinError),
}
type Result<T> = std::result::Result<T, InspectionError>;
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Check,
    Verify,
    Prepare,
}
type Files = BTreeMap<Arc<RelativePath>, LocalFile>;
pub struct Prepared {
    request: Request,
    inventory: Inventory,
}
impl Prepared {
    pub fn materialize(
        self,
        options: flux::Options,
    ) -> (impl Future<Output = Result<Outcome>>, flux::ProgressMonitor) {
        let goal = self.request.goal.clone();
        let (run, monitor) = flux::materialize(self.request, options);
        (
            async move {
                let outcome = run.await?;
                tokio::task::spawn_blocking(move || {
                    let rows = outcome
                        .published
                        .iter()
                        .map(|(path, stamp)| {
                            (
                                path.clone(),
                                LocalFile {
                                    stamp: stamp.clone(),
                                    recipe: goal.files()[path].clone(),
                                },
                            )
                        })
                        .collect();
                    self.inventory
                        .update(&rows, (outcome.deleted_entries > 0).then_some(&goal))?;
                    Ok(outcome)
                })
                .await?
            },
            monitor,
        )
    }
}
async fn inspect(
    mut request: Request,
    mode: Mode,
    cancel: CancellationToken,
    bytes: Arc<AtomicU64>,
) -> Result<(bool, Option<Prepared>)> {
    tokio::task::spawn_blocking(move || {
        if cancel.is_cancelled() {
            return Err(flux::Error::Cancelled.into());
        }
        let root = scanner::root(&request.target, mode == Mode::Prepare)?;
        let Some(inventory) = Inventory::open(&root, mode != Mode::Check)? else {
            return Ok((false, None));
        };
        let (exact, files) =
            scanner::observe(&root, &inventory, &request.goal, mode, &cancel, &bytes)?;
        if cancel.is_cancelled() {
            return Err(flux::Error::Cancelled.into());
        }
        if exact || mode != Mode::Prepare {
            return Ok((exact, None));
        }
        request.local = files.into();
        Ok((exact, Some(Prepared { request, inventory })))
    })
    .await?
}
pub async fn check(request: Request, cancel: CancellationToken) -> Result<bool> {
    Ok(inspect(request, Mode::Check, cancel, Default::default())
        .await?
        .0)
}
pub async fn verify(
    request: Request,
    cancel: CancellationToken,
    bytes: Arc<AtomicU64>,
) -> Result<bool> {
    Ok(inspect(request, Mode::Verify, cancel, bytes).await?.0)
}
pub async fn prepare(
    request: Request,
    cancel: CancellationToken,
    bytes: Arc<AtomicU64>,
) -> Result<Option<Prepared>> {
    Ok(inspect(request, Mode::Prepare, cancel, bytes).await?.1)
}

#[cfg(test)]
mod tests;
