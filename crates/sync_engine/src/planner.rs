use crate::events::{EventSink, SyncEvent};
use crate::index::{self, LocalIndex};
use crate::remote::RemoteRepo;
use crate::safe_path::{safe_join, validate_rel_path};
use crate::types::*;
use anyhow::Result;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone, Debug)]
pub struct PlannedOp {
    pub mod_id: String,
    pub kind: OpKind,
    pub estimated_bytes: u64,
}

#[derive(Clone, Debug)]
pub enum OpKind {
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

pub struct Plan {
    pub ops: Vec<PlannedOp>,
    pub total_bytes: u64,
}

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

    pub async fn build(
        self,
        repo: RepoSpec,
        idx: &mut LocalIndex,
        supports_ranges: bool,
    ) -> Result<Plan> {
        let enabled: HashSet<String> = self.enabled_mods.iter().cloned().collect();
        let available: HashSet<String> = repo.mods.iter().map(|m| m.mod_id.clone()).collect();

        self.sink.push(SyncEvent::PlanningStarted {
            mods_enabled: enabled.len(),
        });

        for m in &enabled {
            if !available.contains(m) {
                self.sink.push(SyncEvent::Warning {
                    message: format!("enabled mod not found in repo spec: {m}"),
                });
            }
        }

        let scan_workers = self.tuning.scan_concurrency.max(1);
        let scan_sem = Arc::new(Semaphore::new(scan_workers));

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
                    self.sink.push(SyncEvent::ModFinished { mod_id });
                    continue;
                }
            };

            let mod_root = self.checkout_root.join(&mod_id);

            let mut expected_paths: HashSet<PathBuf> = HashSet::new();
            let mut scans = FuturesUnordered::new();

            for fe in manifest.files.into_iter() {
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

                let indexed = match idx.get(&abs_path) {
                    Ok(v) => v,
                    Err(e) => {
                        self.sink.push(SyncEvent::Warning {
                            message: format!("index get failed for {}: {e}", abs_path.display()),
                        });
                        None
                    }
                };

                let permit = scan_sem.clone().acquire_owned().await?;
                let checksummer = self.checksummer.clone();
                let tuning = self.tuning.clone();
                let rel_path = fe.rel_path.clone();
                let mod_id_cl = mod_id.clone();
                let abs_path_cl = abs_path.clone();

                scans.push(tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    scan_one_file(
                        &mod_id_cl,
                        &rel_path,
                        abs_path_cl,
                        fe,
                        indexed,
                        checksummer.as_ref(),
                        &tuning,
                        supports_ranges,
                    )
                }));
            }

            while let Some(res) = scans.next().await {
                let outcome = res??;
                match outcome {
                    ScanOutcome::UpToDate {
                        mod_id,
                        rel_path,
                        abs_path,
                        size,
                        mtime_ns,
                        file_checksum,
                        needs_index_write,
                    } => {
                        self.sink.push(SyncEvent::FileUpToDate {
                            mod_id,
                            path: rel_path.clone(),
                        });

                        if needs_index_write {
                            if let Err(e) =
                                idx.upsert_known(&abs_path, size, mtime_ns, &file_checksum)
                            {
                                self.sink.push(SyncEvent::Warning {
                                    message: format!(
                                        "index upsert failed for {}: {e}",
                                        abs_path.display()
                                    ),
                                });
                            }
                        }
                    }
                    ScanOutcome::NeedsTransfer {
                        mod_id,
                        rel_path,
                        abs_path,
                        target,
                        estimated_bytes,
                    } => {
                        ops.push(PlannedOp {
                            mod_id: mod_id.clone(),
                            kind: OpKind::EnsureFile {
                                mod_id,
                                rel_path,
                                abs_path,
                                manifest: target,
                            },
                            estimated_bytes,
                        });
                    }
                }
            }

            if self.tuning.delete_extraneous {
                let mod_root_cl = mod_root.clone();
                let expected = expected_paths;
                let extraneous = tokio::task::spawn_blocking(move || {
                    find_extraneous_files(&mod_root_cl, &expected)
                })
                .await??;

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
        self.sink.push(SyncEvent::PlanningFinished {
            ops: ops.len(),
            total_bytes,
        });

        Ok(Plan { ops, total_bytes })
    }
}

enum ScanOutcome {
    UpToDate {
        mod_id: String,
        rel_path: String,
        abs_path: PathBuf,
        size: u64,
        mtime_ns: u128,
        file_checksum: Vec<u8>,
        needs_index_write: bool,
    },
    NeedsTransfer {
        mod_id: String,
        rel_path: String,
        abs_path: PathBuf,
        target: FileTarget,
        estimated_bytes: u64,
    },
}

fn scan_one_file(
    mod_id: &str,
    rel_path: &str,
    abs_path: PathBuf,
    fe: FileEntry,
    indexed: Option<index::IndexedFile>,
    checksummer: &dyn Checksummer,
    tuning: &SyncTuning,
    supports_ranges: bool,
) -> Result<ScanOutcome> {
    let md = match std::fs::metadata(&abs_path) {
        Ok(m) => m,
        Err(_) => {
            let target = FileTarget {
                size: fe.size,
                file_checksum: fe.file_checksum.clone(),
                parts: fe.parts.clone(),
                strategy: RepairStrategy::Full,
                parts_to_fetch: Vec::new(),
            };
            return Ok(ScanOutcome::NeedsTransfer {
                mod_id: mod_id.to_string(),
                rel_path: rel_path.to_string(),
                abs_path,
                estimated_bytes: fe.size,
                target,
            });
        }
    };

    if !md.is_file() || md.len() != fe.size {
        let target = FileTarget {
            size: fe.size,
            file_checksum: fe.file_checksum.clone(),
            parts: fe.parts.clone(),
            strategy: RepairStrategy::Full,
            parts_to_fetch: Vec::new(),
        };
        return Ok(ScanOutcome::NeedsTransfer {
            mod_id: mod_id.to_string(),
            rel_path: rel_path.to_string(),
            abs_path,
            estimated_bytes: fe.size,
            target,
        });
    }

    let mtime_ns = index::file_mtime_ns(&md).unwrap_or(0);

    let index_hit = indexed.as_ref().is_some_and(|ix| {
        ix.size == fe.size && ix.mtime_ns == mtime_ns && ix.checksum == fe.file_checksum.bytes
    });
    if index_hit {
        return Ok(ScanOutcome::UpToDate {
            mod_id: mod_id.to_string(),
            rel_path: rel_path.to_string(),
            abs_path,
            size: fe.size,
            mtime_ns,
            file_checksum: fe.file_checksum.bytes,
            needs_index_write: false,
        });
    }

    let ranges: Vec<(u64, u64)> = fe.parts.iter().map(|p| (p.offset, p.len)).collect();
    let got = checksummer.hash_ranges(&abs_path, &ranges)?;

    let mut parts_to_fetch: Vec<FilePart> = Vec::new();
    let mut bad_bytes: u64 = 0;
    for (i, p) in fe.parts.iter().enumerate() {
        if got[i] != p.checksum.bytes {
            parts_to_fetch.push(p.clone());
            bad_bytes += p.len;
        }
    }

    if parts_to_fetch.is_empty() {
        return Ok(ScanOutcome::UpToDate {
            mod_id: mod_id.to_string(),
            rel_path: rel_path.to_string(),
            abs_path,
            size: fe.size,
            mtime_ns,
            file_checksum: fe.file_checksum.bytes,
            needs_index_write: true,
        });
    }

    let ratio = if fe.size == 0 {
        0.0
    } else {
        (bad_bytes as f32) / (fe.size as f32)
    };

    let mut target = FileTarget {
        size: fe.size,
        file_checksum: fe.file_checksum,
        parts: fe.parts,
        strategy: RepairStrategy::Full,
        parts_to_fetch: Vec::new(),
    };

    if supports_ranges && ratio <= tuning.patch_max_bad_ratio {
        target.strategy = RepairStrategy::Patch;
        target.parts_to_fetch = parts_to_fetch;
    } else {
        target.strategy = RepairStrategy::Full;
        target.parts_to_fetch.clear();
    }

    let estimated_bytes = if target.strategy.is_patch() {
        target.parts_to_fetch.iter().map(|p| p.len).sum()
    } else {
        target.size
    };

    Ok(ScanOutcome::NeedsTransfer {
        mod_id: mod_id.to_string(),
        rel_path: rel_path.to_string(),
        abs_path,
        target,
        estimated_bytes,
    })
}

fn find_extraneous_files(mod_root: &Path, expected: &HashSet<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !mod_root.exists() {
        return Ok(out);
    }
    for ent in walkdir::WalkDir::new(mod_root).into_iter().filter_map(Result::ok) {
        let p = ent.path().to_path_buf();
        if ent.file_type().is_file() && !expected.contains(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

