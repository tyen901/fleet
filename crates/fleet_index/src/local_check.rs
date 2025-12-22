use crate::path_safety::{normalize_rel_path, validate_mod_id, validate_rel_path};
use crate::safe_fs::ensure_no_symlink_ancestors;
use crate::types::IndexError;
use crate::FleetIndex;
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct LocalCheckOptions {
    pub max_issues: usize,
}

impl Default for LocalCheckOptions {
    fn default() -> Self {
        Self { max_issues: 500 }
    }
}

#[derive(Clone, Debug)]
pub enum LocalCheckOutcome {
    NoDesiredState,
    NoBaseline { state_id: String },
    Report(LocalCheckReport),
}

#[derive(Clone, Debug)]
pub struct LocalCheckReport {
    pub state_id: String,
    pub expected_files: u64,
    pub ok: u64,
    pub missing: u64,
    pub wrong_size: u64,
    pub not_a_file: u64,
    pub unsafe_path: u64,
    pub issues: Vec<LocalIssue>,
}

#[derive(Clone, Debug)]
pub enum LocalIssueKind {
    Missing,
    WrongSize { expected: u64, got: u64 },
    NotAFile,
    UnsafePath,
}

#[derive(Clone, Debug)]
pub struct LocalIssue {
    pub mod_id: String,
    pub rel_path: String,
    pub kind: LocalIssueKind,
}

impl FleetIndex {
    pub fn local_check(
        &self,
        checkout_root: &Path,
        opts: LocalCheckOptions,
    ) -> Result<LocalCheckOutcome, IndexError> {
        let Some(desired) = self.get_desired_state()? else {
            return Ok(LocalCheckOutcome::NoDesiredState);
        };

        if !self.baseline_exists(&desired.state_id)? {
            return Ok(LocalCheckOutcome::NoBaseline {
                state_id: desired.state_id,
            });
        }

        let mut report = LocalCheckReport {
            state_id: desired.state_id.clone(),
            expected_files: 0,
            ok: 0,
            missing: 0,
            wrong_size: 0,
            not_a_file: 0,
            unsafe_path: 0,
            issues: Vec::new(),
        };

        // Auto-fix casing silently as a pre-step (per mod) so checks aren't polluted by casing artifacts.
        let mut expected_by_mod: HashMap<String, Vec<(String, u64, Option<Vec<u8>>)>> =
            HashMap::new();
        self.expected_for_each(&desired.state_id, |expected| {
            expected_by_mod
                .entry(expected.mod_id.clone())
                .or_default()
                .push((expected.rel_path.clone(), expected.size, None));
            Ok(())
        })?;
        let sweep_tuning = fleet_fs_case::CaseFixTuning::default();
        for (mod_id, expected) in &expected_by_mod {
            let _ = fleet_fs_case::case_sweep_and_fix(
                checkout_root,
                mod_id,
                expected,
                &sweep_tuning,
                None,
            );
        }

        self.expected_for_each(&desired.state_id, |expected| {
            report.expected_files += 1;

            let mod_id = expected.mod_id.clone();
            let rel_norm = normalize_rel_path(&expected.rel_path);

                if let Err(_err) =
                    validate_mod_id(&mod_id).and_then(|_| validate_rel_path(&rel_norm))
                {
                    report.unsafe_path += 1;
                    push_issue(
                        &mut report.issues,
                        opts.max_issues,
                        LocalIssue {
                            mod_id: mod_id.clone(),
                            rel_path: rel_norm,
                            kind: LocalIssueKind::UnsafePath,
                        },
                    );
                    return Ok(());
                }

            let abs_path = checkout_root.join(&mod_id).join(&rel_norm);

            // Mirror the operational rule: ancestor symlink/reparse-point is unsafe-on-disk.
            if let Some(parent) = abs_path.parent() {
                let mod_root = checkout_root.join(&mod_id);
                if ensure_no_symlink_ancestors(&mod_root, parent).is_err() {
                    report.unsafe_path += 1;
                    push_issue(
                        &mut report.issues,
                        opts.max_issues,
                        LocalIssue {
                            mod_id: mod_id.clone(),
                            rel_path: rel_norm,
                            kind: LocalIssueKind::UnsafePath,
                        },
                    );
                    return Ok(());
                }
            }
            match std::fs::symlink_metadata(&abs_path) {
                Ok(md) => {
                    let ft = md.file_type();
                    if ft.is_symlink() || !ft.is_file() {
                        report.not_a_file += 1;
                        push_issue(
                            &mut report.issues,
                            opts.max_issues,
                        LocalIssue {
                            mod_id: mod_id.clone(),
                            rel_path: rel_norm,
                            kind: LocalIssueKind::NotAFile,
                        },
                    );
                    return Ok(());
                    }

                    let got = md.len();
                    if got != expected.size {
                        report.wrong_size += 1;
                        push_issue(
                            &mut report.issues,
                            opts.max_issues,
                        LocalIssue {
                            mod_id: mod_id.clone(),
                            rel_path: rel_norm,
                            kind: LocalIssueKind::WrongSize {
                                expected: expected.size,
                                got,
                                },
                            },
                        );
                        return Ok(());
                    }

                    report.ok += 1;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    report.missing += 1;
                    push_issue(
                        &mut report.issues,
                        opts.max_issues,
                        LocalIssue {
                            mod_id: mod_id.clone(),
                            rel_path: rel_norm,
                            kind: LocalIssueKind::Missing,
                        },
                    );
                }
                Err(e) => return Err(IndexError::Io(e)),
            }

            Ok(())
        })?;

        Ok(LocalCheckOutcome::Report(report))
    }
}

fn push_issue(issues: &mut Vec<LocalIssue>, max: usize, issue: LocalIssue) {
    if issues.len() < max {
        issues.push(issue);
    }
}
