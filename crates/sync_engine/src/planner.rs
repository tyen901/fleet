use crate::events::{EventSink, SyncEvent};
use crate::remote::RemoteRepo;
use crate::safe_path::{safe_join, validate_rel_path};
use crate::types::*;
use anyhow::Result;
use futures::Stream;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct PlannedOp {
    pub mod_id: String,
    pub kind: OpKind,
    pub estimated_bytes: u64,
}

#[derive(Clone, Debug)]
pub enum OpKind {
    EnsureDir {
        abs_path: PathBuf,
    },
    EnsureFile {
        mod_id: String,
        rel_path: String,
        abs_path: PathBuf,
        manifest: FileTarget,
    },
    DeletePath {
        abs_path: PathBuf,
    },
}

pub type PlannedOpStream = Pin<Box<dyn Stream<Item = PlannedOp> + Send>>;

pub struct PlanBuilder {
    remote: Arc<dyn RemoteRepo>,
    checkout_root: PathBuf,
    enabled_mods: Vec<String>,
    tuning: SyncTuning,
    checksummer: Arc<dyn Checksummer>,
    sink: Arc<dyn EventSink>,
}

impl PlanBuilder {
    pub fn new(
        remote: Arc<dyn RemoteRepo>,
        checkout_root: PathBuf,
        enabled_mods: Vec<String>,
        tuning: SyncTuning,
        checksummer: Arc<dyn Checksummer>,
        sink: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            remote,
            checkout_root,
            enabled_mods,
            tuning,
            checksummer,
            sink,
        }
    }

    pub async fn build_stream(self, repo: RepoSpec) -> Result<(PlannedOpStream, u64)> {
        let enabled: HashSet<String> = self.enabled_mods.iter().cloned().collect();
        let available: HashSet<String> = repo.mods.iter().map(|m| m.mod_id.clone()).collect();

        for m in &enabled {
            if !available.contains(m) {
                self.sink.push(SyncEvent::Warning {
                    message: format!("enabled mod not found in repo spec: {m}"),
                });
            }
        }

        let mut ops: Vec<PlannedOp> = Vec::new();

        for mod_id in enabled.into_iter() {
            self.sink.push(SyncEvent::ModStarted {
                mod_id: mod_id.clone(),
            });

            let manifest = match self.remote.fetch_mod_manifest(&mod_id).await {
                Ok(m) => m,
                Err(e) => {
                    self.sink.push(SyncEvent::Error {
                        message: format!("failed to fetch manifest for {mod_id}: {e}"),
                    });
                    continue;
                }
            };

            let mod_root = self.checkout_root.join(&mod_id);
            ops.push(PlannedOp {
                mod_id: mod_id.clone(),
                kind: OpKind::EnsureDir {
                    abs_path: mod_root.clone(),
                },
                estimated_bytes: 0,
            });

            let mut expected_paths: HashSet<PathBuf> = HashSet::new();
            for fe in manifest.files.iter() {
                if validate_rel_path(&fe.rel_path).is_err() {
                    self.sink.push(SyncEvent::Error {
                        message: format!("invalid rel path in manifest: {}", fe.rel_path),
                    });
                    continue;
                }

                let abs_path = match safe_join(&mod_root, &fe.rel_path) {
                    Ok(p) => p,
                    Err(e) => {
                        self.sink.push(SyncEvent::Error {
                            message: format!("unsafe path {}: {e}", fe.rel_path),
                        });
                        continue;
                    }
                };

                expected_paths.insert(abs_path.clone());

                let target =
                    plan_file_target(&abs_path, fe, self.checksummer.as_ref(), &self.tuning);

                let estimated_bytes = if target.strategy.is_patch() {
                    target.parts_to_fetch.iter().map(|p| p.len).sum()
                } else {
                    fe.size
                };

                ops.push(PlannedOp {
                    mod_id: mod_id.clone(),
                    kind: OpKind::EnsureFile {
                        mod_id: mod_id.clone(),
                        rel_path: fe.rel_path.clone(),
                        abs_path,
                        manifest: target,
                    },
                    estimated_bytes,
                });
            }

            if self.tuning.delete_extraneous {
                let extraneous = find_extraneous_files(&mod_root, &expected_paths);
                for p in extraneous {
                    ops.push(PlannedOp {
                        mod_id: mod_id.clone(),
                        kind: OpKind::DeletePath { abs_path: p },
                        estimated_bytes: 0,
                    });
                }
            }

            self.sink.push(SyncEvent::ModFinished { mod_id });
        }

        let total_bytes: u64 = ops.iter().map(|o| o.estimated_bytes).sum();
        Ok((Box::pin(futures::stream::iter(ops)), total_bytes))
    }
}

fn plan_file_target(
    abs_path: &Path,
    fe: &FileEntry,
    checksummer: &dyn Checksummer,
    tuning: &SyncTuning,
) -> FileTarget {
    let mut parts_to_fetch = Vec::new();
    let mut strategy = RepairStrategy::Full;

    let md = std::fs::metadata(abs_path);
    if let Ok(md) = md {
        if md.is_file() && md.len() == fe.size {
            let mut bad_bytes: u64 = 0;
            for p in fe.parts.iter() {
                let ok = verify_part(abs_path, p, checksummer).unwrap_or(false);
                if !ok {
                    parts_to_fetch.push(p.clone());
                    bad_bytes += p.len;
                }
            }
            let ratio = if fe.size == 0 {
                0.0
            } else {
                (bad_bytes as f32) / (fe.size as f32)
            };
            if !parts_to_fetch.is_empty() && ratio <= tuning.patch_max_bad_ratio {
                strategy = RepairStrategy::Patch;
            } else if parts_to_fetch.is_empty() {
                strategy = RepairStrategy::Full;
            } else {
                strategy = RepairStrategy::Full;
                parts_to_fetch.clear();
            }
        } else {
            strategy = RepairStrategy::Full;
        }
    }

    FileTarget {
        size: fe.size,
        file_checksum: fe.file_checksum.clone(),
        parts: fe.parts.clone(),
        strategy,
        parts_to_fetch,
    }
}

fn verify_part(path: &Path, part: &FilePart, checksummer: &dyn Checksummer) -> Result<bool> {
    let got = checksummer.hash_range(path, part.offset, part.len)?;
    Ok(got == part.checksum.bytes)
}

fn find_extraneous_files(mod_root: &Path, expected: &HashSet<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for ent in walkdir::WalkDir::new(mod_root)
        .into_iter()
        .filter_map(Result::ok)
    {
        let p = ent.path().to_path_buf();
        if p.components().any(|c| c.as_os_str() == ".fleet") {
            continue;
        }
        if ent.file_type().is_file() && !expected.contains(&p) {
            out.push(p);
        }
    }
    out
}
