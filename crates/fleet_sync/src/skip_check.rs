#![allow(dead_code)]

use crate::fs::{ensure_no_symlink_ancestors_blocking, safe_join_mod_file};
use crate::ports::StateStore;
use crate::util::file_mtime_ns;
use anyhow::Result;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct SkipCheckPolicy {
    pub max_issues: usize,
}

impl Default for SkipCheckPolicy {
    fn default() -> Self {
        Self { max_issues: 500 }
    }
}

#[derive(Clone, Debug)]
pub enum SkipCheckDecision {
    Skippable(SkipCheckEvidence),
    NotSkippable {
        reason: SkipCheckReason,
        evidence: SkipCheckEvidence,
    },
}

#[derive(Clone, Debug)]
pub enum SkipCheckReason {
    NoDesiredState,
    NotVerified,
    VerifiedStateMismatch,
    NoBaseline,
    LocalCheckFailed,
    CacheMissing,
    MtimeMismatch,
    ChecksumMismatch,
}

#[derive(Clone, Debug, Default)]
pub struct SkipCheckEvidence {
    pub state_id: Option<String>,
    pub verified_state_id: Option<String>,
    pub verified_at: Option<i64>,

    pub expected_files: u64,

    pub local_missing: u64,
    pub local_wrong_size: u64,
    pub local_not_a_file: u64,
    pub local_unsafe_path: u64,

    pub cache_missing: u64,
    pub mtime_mismatch: u64,
    pub checksum_mismatch: u64,

    pub issues: Vec<SkipCheckIssue>,
}

#[derive(Clone, Debug)]
pub enum SkipCheckIssueKind {
    Missing,
    WrongSize { expected: u64, got: u64 },
    NotAFile,
    UnsafePath,
}

#[derive(Clone, Debug)]
pub struct SkipCheckIssue {
    pub mod_id: String,
    pub rel_path: String,
    pub kind: SkipCheckIssueKind,
}

fn push_issue(issues: &mut Vec<SkipCheckIssue>, max: usize, issue: SkipCheckIssue) {
    if issues.len() < max {
        issues.push(issue);
    }
}

pub fn evaluate_skip(
    store: &dyn StateStore,
    checkout_root: &Path,
    policy: SkipCheckPolicy,
) -> Result<SkipCheckDecision> {
    let desired = match store.desired_state_get()? {
        Some(s) => s,
        None => {
            return Ok(SkipCheckDecision::NotSkippable {
                reason: SkipCheckReason::NoDesiredState,
                evidence: SkipCheckEvidence::default(),
            })
        }
    };

    let verified = store.verified_get()?;
    let mut evidence = SkipCheckEvidence {
        state_id: Some(desired.state_id.clone()),
        verified_state_id: verified.as_ref().map(|v| v.state_id.clone()),
        verified_at: verified.as_ref().map(|v| v.verified_at.0),
        ..Default::default()
    };

    let Some(verified_state) = verified else {
        return Ok(SkipCheckDecision::NotSkippable {
            reason: SkipCheckReason::NotVerified,
            evidence,
        });
    };

    if verified_state.state_id != desired.state_id {
        return Ok(SkipCheckDecision::NotSkippable {
            reason: SkipCheckReason::VerifiedStateMismatch,
            evidence,
        });
    }

    if !store.baseline_exists(&desired.state_id)? {
        return Ok(SkipCheckDecision::NotSkippable {
            reason: SkipCheckReason::NoBaseline,
            evidence,
        });
    }

    let expected = store.expected_load_v2(&desired.state_id)?;

    let mut observed_by_mod: std::collections::HashMap<String, std::collections::HashMap<String, fleet_index::ObservedRow>> =
        std::collections::HashMap::new();
    for f in &expected {
        if observed_by_mod.contains_key(&f.mod_id) {
            continue;
        }
        let rows = store.observed_get_all_for_mod_v2(&desired.state_id, &f.mod_id)?;
        observed_by_mod.insert(f.mod_id.clone(), rows);
    }

    for f in expected {
        evidence.expected_files += 1;

        let abs_path = match safe_join_mod_file(checkout_root, &f.mod_id, &f.rel_path) {
            Ok(p) => p,
            Err(_) => {
                evidence.local_unsafe_path += 1;
                push_issue(
                    &mut evidence.issues,
                    policy.max_issues,
                    SkipCheckIssue {
                        mod_id: f.mod_id,
                        rel_path: f.rel_path,
                        kind: SkipCheckIssueKind::UnsafePath,
                    },
                );
                continue;
            }
        };

        if let Some(parent) = abs_path.parent() {
            let mod_root = checkout_root.join(&f.mod_id);
            if ensure_no_symlink_ancestors_blocking(&mod_root, parent).is_err() {
                evidence.local_unsafe_path += 1;
                push_issue(
                    &mut evidence.issues,
                    policy.max_issues,
                    SkipCheckIssue {
                        mod_id: f.mod_id,
                        rel_path: f.rel_path,
                        kind: SkipCheckIssueKind::UnsafePath,
                    },
                );
                continue;
            }
        }

        let md = match std::fs::symlink_metadata(&abs_path) {
            Ok(md) => md,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                evidence.local_missing += 1;
                push_issue(
                    &mut evidence.issues,
                    policy.max_issues,
                    SkipCheckIssue {
                        mod_id: f.mod_id,
                        rel_path: f.rel_path,
                        kind: SkipCheckIssueKind::Missing,
                    },
                );
                continue;
            }
            Err(e) => return Err(e.into()),
        };

        let ft = md.file_type();
        if ft.is_symlink() || !ft.is_file() {
            evidence.local_not_a_file += 1;
            push_issue(
                &mut evidence.issues,
                policy.max_issues,
                SkipCheckIssue {
                    mod_id: f.mod_id,
                    rel_path: f.rel_path,
                    kind: SkipCheckIssueKind::NotAFile,
                },
            );
            continue;
        }

        let got_size = md.len();
        if got_size != f.size {
            evidence.local_wrong_size += 1;
            push_issue(
                &mut evidence.issues,
                policy.max_issues,
                SkipCheckIssue {
                    mod_id: f.mod_id.clone(),
                    rel_path: f.rel_path.clone(),
                    kind: SkipCheckIssueKind::WrongSize {
                        expected: f.size,
                        got: got_size,
                    },
                },
            );
            continue;
        }

        let Some(actual_mtime_ns) = file_mtime_ns(&md) else {
            evidence.mtime_mismatch += 1;
            continue;
        };

        let observed_map = observed_by_mod.get(&f.mod_id).expect("observed map inserted");
        let cached = observed_map.get(&f.rel_path);
        let Some(cached) = cached else {
            evidence.cache_missing += 1;
            continue;
        };

        if cached.mtime_ns != actual_mtime_ns.0 || cached.size != got_size {
            evidence.mtime_mismatch += 1;
            continue;
        }

        let Some(file_md5) = cached.file_md5 else {
            evidence.cache_missing += 1;
            continue;
        };

        if file_md5 != f.file_md5 {
            evidence.checksum_mismatch += 1;
        }
    }

    let local_clean = evidence.local_missing == 0
        && evidence.local_wrong_size == 0
        && evidence.local_not_a_file == 0
        && evidence.local_unsafe_path == 0;

    if !local_clean {
        return Ok(SkipCheckDecision::NotSkippable {
            reason: SkipCheckReason::LocalCheckFailed,
            evidence,
        });
    }
    if evidence.cache_missing > 0 {
        return Ok(SkipCheckDecision::NotSkippable {
            reason: SkipCheckReason::CacheMissing,
            evidence,
        });
    }
    if evidence.mtime_mismatch > 0 {
        return Ok(SkipCheckDecision::NotSkippable {
            reason: SkipCheckReason::MtimeMismatch,
            evidence,
        });
    }
    if evidence.checksum_mismatch > 0 {
        return Ok(SkipCheckDecision::NotSkippable {
            reason: SkipCheckReason::ChecksumMismatch,
            evidence,
        });
    }

    Ok(SkipCheckDecision::Skippable(evidence))
}
