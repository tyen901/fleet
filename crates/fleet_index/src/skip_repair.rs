use crate::local_check::{LocalIssue, LocalIssueKind};
use crate::path_safety::{normalize_rel_path, validate_mod_id, validate_rel_path};
use crate::safe_fs::ensure_no_symlink_ancestors;
use crate::store::file_mtime_ns;
use crate::types::IndexError;
use crate::FleetIndex;
use rusqlite::params;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct SkipRepairPolicy {
    pub max_issues: usize,
}

impl Default for SkipRepairPolicy {
    fn default() -> Self {
        Self { max_issues: 500 }
    }
}

#[derive(Clone, Debug)]
pub enum SkipRepairDecision {
    Skippable(SkipRepairEvidence),
    NotSkippable {
        reason: SkipRepairReason,
        evidence: SkipRepairEvidence,
    },
}

#[derive(Clone, Debug)]
pub enum SkipRepairReason {
    NoDesiredState,
    NotVerified,
    VerifiedStateMismatch,
    NoBaseline,
    LocalCheckFailed,
    CacheMissing,
    MtimeMismatch,
}

#[derive(Clone, Debug, Default)]
pub struct SkipRepairEvidence {
    pub state_id: Option<String>,
    pub verified_state_id: Option<String>,
    pub verified_at_ns: Option<i64>,

    pub expected_files: u64,

    pub local_missing: u64,
    pub local_wrong_size: u64,
    pub local_not_a_file: u64,
    pub local_unsafe_path: u64,

    pub cache_missing: u64,
    pub mtime_mismatch: u64,

    pub issues: Vec<LocalIssue>,
}

impl FleetIndex {
    pub fn evaluate_skip_repair(
        &self,
        checkout_root: &Path,
        policy: SkipRepairPolicy,
    ) -> Result<SkipRepairDecision, IndexError> {
        let desired = match self.get_desired_state()? {
            Some(s) => s,
            None => {
                return Ok(SkipRepairDecision::NotSkippable {
                    reason: SkipRepairReason::NoDesiredState,
                    evidence: SkipRepairEvidence::default(),
                })
            }
        };

        let verified = self.verified_get()?;
        let mut evidence = SkipRepairEvidence {
            state_id: Some(desired.state_id.clone()),
            verified_state_id: verified.as_ref().map(|v| v.state_id.clone()),
            verified_at_ns: verified.as_ref().map(|v| v.verified_at_ns),
            expected_files: 0,
            local_missing: 0,
            local_wrong_size: 0,
            local_not_a_file: 0,
            local_unsafe_path: 0,
            cache_missing: 0,
            mtime_mismatch: 0,
            issues: Vec::new(),
        };

        let Some(verified_state) = verified else {
            return Ok(SkipRepairDecision::NotSkippable {
                reason: SkipRepairReason::NotVerified,
                evidence,
            });
        };

        if verified_state.state_id != desired.state_id {
            return Ok(SkipRepairDecision::NotSkippable {
                reason: SkipRepairReason::VerifiedStateMismatch,
                evidence,
            });
        }

        if !self.baseline_exists(&desired.state_id)? {
            return Ok(SkipRepairDecision::NotSkippable {
                reason: SkipRepairReason::NoBaseline,
                evidence,
            });
        }

        let mut stmt = self.conn.prepare(
            "SELECT e.mod_id, e.rel_path, e.size, fs.size, fs.mtime_ns, fs.checksum \
             FROM expected_file e \
             LEFT JOIN file_state fs \
               ON fs.state_id = e.state_id \
              AND fs.mod_id = e.mod_id \
              AND fs.rel_path = e.rel_path \
             WHERE e.state_id = ?1 \
             ORDER BY e.mod_id, e.rel_path",
        )?;

        let mut rows = stmt.query(params![desired.state_id])?;
        while let Some(row) = rows.next()? {
            let mod_id: String = row.get(0)?;
            let rel_path: String = row.get(1)?;
            let expected_size_i64: i64 = row.get(2)?;
            let expected_size = u64::try_from(expected_size_i64)
                .map_err(|_| IndexError::Corrupt("size overflow".to_string()))?;

            evidence.expected_files += 1;

            let rel_norm = normalize_rel_path(&rel_path);
            if validate_mod_id(&mod_id)
                .and_then(|_| validate_rel_path(&rel_norm))
                .is_err()
            {
                evidence.local_unsafe_path += 1;
                push_issue(
                    &mut evidence.issues,
                    policy.max_issues,
                    LocalIssue {
                        mod_id,
                        rel_path: rel_norm,
                        kind: LocalIssueKind::UnsafePath,
                    },
                );
                continue;
            }

            let abs_path = checkout_root.join(&mod_id).join(&rel_norm);
            // Critical: prevent "skip" when any ancestor is a symlink/reparse-point.
            // This is the exact hole that allowed repair() to incorrectly succeed via skip.
            let mod_root = checkout_root.join(&mod_id);
            if let Some(parent) = abs_path.parent() {
                if ensure_no_symlink_ancestors(&mod_root, parent).is_err() {
                    evidence.local_unsafe_path += 1;
                    push_issue(
                        &mut evidence.issues,
                        policy.max_issues,
                        LocalIssue {
                            mod_id,
                            rel_path: rel_norm,
                            kind: LocalIssueKind::UnsafePath,
                        },
                    );
                    continue;
                }
            }
            let metadata = match std::fs::symlink_metadata(&abs_path) {
                Ok(md) => {
                    let ft = md.file_type();
                    if ft.is_symlink() || !ft.is_file() {
                        evidence.local_not_a_file += 1;
                        push_issue(
                            &mut evidence.issues,
                            policy.max_issues,
                            LocalIssue {
                                mod_id,
                                rel_path: rel_norm,
                                kind: LocalIssueKind::NotAFile,
                            },
                        );
                        continue;
                    }
                    md
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    evidence.local_missing += 1;
                    push_issue(
                        &mut evidence.issues,
                        policy.max_issues,
                        LocalIssue {
                            mod_id,
                            rel_path: rel_norm,
                            kind: LocalIssueKind::Missing,
                        },
                    );
                    continue;
                }
                Err(e) => return Err(IndexError::Io(e)),
            };

            let got_size = metadata.len();
            if got_size != expected_size {
                evidence.local_wrong_size += 1;
                push_issue(
                    &mut evidence.issues,
                    policy.max_issues,
                    LocalIssue {
                        mod_id,
                        rel_path: rel_norm,
                        kind: LocalIssueKind::WrongSize {
                            expected: expected_size,
                            got: got_size,
                        },
                    },
                );
                continue;
            }

            let cached_size: Option<i64> = row.get(3)?;
            let cached_mtime: Option<i64> = row.get(4)?;
            let cached_checksum: Option<Vec<u8>> = row.get(5)?;
            if cached_size.is_none() || cached_mtime.is_none() || cached_checksum.is_none() {
                evidence.cache_missing += 1;
                continue;
            }

            let Some(actual_mtime_ns) = file_mtime_ns(&metadata) else {
                evidence.mtime_mismatch += 1;
                continue;
            };

            if Some(actual_mtime_ns) != cached_mtime {
                evidence.mtime_mismatch += 1;
            }
        }

        let local_clean = evidence.local_missing == 0
            && evidence.local_wrong_size == 0
            && evidence.local_not_a_file == 0
            && evidence.local_unsafe_path == 0;
        if !local_clean {
            return Ok(SkipRepairDecision::NotSkippable {
                reason: SkipRepairReason::LocalCheckFailed,
                evidence,
            });
        }
        if evidence.cache_missing > 0 {
            return Ok(SkipRepairDecision::NotSkippable {
                reason: SkipRepairReason::CacheMissing,
                evidence,
            });
        }
        if evidence.mtime_mismatch > 0 {
            return Ok(SkipRepairDecision::NotSkippable {
                reason: SkipRepairReason::MtimeMismatch,
                evidence,
            });
        }

        Ok(SkipRepairDecision::Skippable(evidence))
    }
}

fn push_issue(issues: &mut Vec<LocalIssue>, max: usize, issue: LocalIssue) {
    if issues.len() < max {
        issues.push(issue);
    }
}
