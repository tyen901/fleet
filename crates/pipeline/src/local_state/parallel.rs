#[cfg(test)]
use fleet_domain::{LocalStateProgress, LocalStateStage};
use fleet_inventory::InventoryError;
use rayon::prelude::*;
#[cfg(test)]
use std::sync::Arc;

pub(crate) const DEFAULT_CHUNK_SIZE: usize = 256;
const MAX_WORKERS: usize = 8;

pub(crate) fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get().min(MAX_WORKERS))
        .unwrap_or(1)
        .max(1)
}

pub(crate) fn execute_chunked<T, R, F>(
    items: &[T],
    workers: usize,
    chunk_size: usize,
    process_chunk: F,
) -> Result<Vec<R>, InventoryError>
where
    T: Sync,
    R: Send,
    F: Fn(&[T]) -> Result<R, InventoryError> + Sync + Send,
{
    if items.is_empty() {
        return Ok(Vec::new());
    }

    if workers <= 1 || items.len() <= chunk_size {
        return items
            .chunks(chunk_size)
            .map(process_chunk)
            .collect::<Result<Vec<_>, _>>();
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .map_err(anyhow::Error::new)
        .map_err(InventoryError::Other)?;

    pool.install(|| {
        items
            .par_chunks(chunk_size)
            .map(process_chunk)
            .collect::<Vec<_>>()
            .into_iter()
            .collect()
    })
}

#[cfg(test)]
pub(crate) struct ChunkProgressReporter {
    sink: Option<Arc<dyn Fn(LocalStateProgress) + Send + Sync>>,
    stage: LocalStateStage,
    total: u64,
    seen: u64,
}

#[cfg(test)]
pub(crate) fn chunk_progress_reporter(
    sink: Option<Arc<dyn Fn(LocalStateProgress) + Send + Sync>>,
    stage: LocalStateStage,
    total: u64,
) -> ChunkProgressReporter {
    ChunkProgressReporter {
        sink,
        stage,
        total,
        seen: 0,
    }
}

#[cfg(test)]
impl ChunkProgressReporter {
    pub(crate) fn advance_by(&mut self, count: usize) {
        self.seen = self.seen.saturating_add(count as u64).min(self.total);
        self.emit();
    }

    pub(crate) fn final_flush(&self) {
        self.emit();
    }

    fn emit(&self) {
        let Some(sink) = self.sink.as_ref() else {
            return;
        };
        sink(LocalStateProgress {
            stage: self.stage,
            files_total: self.total,
            files_seen: self.seen,
            files_scanned: self.seen,
            bytes_scanned: 0,
            bytes_total: 0,
        });
    }
}
