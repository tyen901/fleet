use std::collections::HashMap;
use std::sync::Arc;

use crate::manifest::ValidatedModManifest;
use crate::plan::{plan_mod, Plan, PlanError};
use crate::ports::Checksummer;

#[derive(Debug)]
pub(crate) enum PlannerError {
    UnsafeOnDisk {
        mod_id: String,
        rel_path: String,
        message: String,
    },
    Other(anyhow::Error),
}

pub(crate) struct CacheHint {
    pub(crate) mod_id: String,
    pub(crate) rel_path: String,
    pub(crate) size: u64,
    pub(crate) mtime_ns: i64,
    pub(crate) checksum: Vec<u8>,
}

pub(crate) async fn plan_mod_spawn_blocking(
    checkout_root: &std::path::Path,
    manifest: ValidatedModManifest,
    cache: HashMap<String, crate::model::FileState>,
    supports_ranges: bool,
    tuning: crate::model::RepairTuning,
    checksummer: Arc<dyn Checksummer>,
) -> Result<Result<(Plan, Vec<CacheHint>), PlannerError>, crate::model::EngineError> {
    let checkout_root = checkout_root.to_path_buf();
    let plan_res = tokio::task::spawn_blocking(move || {
        let plan_res = plan_mod(
            &checkout_root,
            &manifest,
            &cache,
            supports_ranges,
            &tuning,
            checksummer.as_ref(),
        );
        match plan_res {
            Ok((plan, hints)) => {
                let hints = hints
                    .into_iter()
                    .map(|h| CacheHint {
                        mod_id: h.mod_id,
                        rel_path: h.rel_path,
                        size: h.size,
                        mtime_ns: h.mtime_ns,
                        checksum: h.checksum,
                    })
                    .collect();
                Ok((plan, hints))
            }
            Err(PlanError::UnsafeOnDisk {
                mod_id,
                rel_path,
                source,
            }) => Err(PlannerError::UnsafeOnDisk {
                mod_id,
                rel_path,
                message: source.to_string(),
            }),
            Err(e) => Err(PlannerError::Other(e.into())),
        }
    })
    .await
    .map_err(|e| crate::model::EngineError::Internal(anyhow::anyhow!(e.to_string())))?;

    Ok(plan_res)
}

