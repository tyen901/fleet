use serde::{Deserialize, Serialize};

use crate::domain::ProfileId;
use fleet_pipeline::FetchStats;
use fleet_scanner::ScanStats;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct TransferProgressVm {
    pub downloaded_files: u64,
    pub total_files: u64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
    pub failed_count: u64,
    pub active_files: Vec<ActiveTransferFileVm>,
}

#[derive(Debug, Clone)]
pub struct ActiveTransferFileVm {
    pub mod_name: String,
    pub rel_path: String,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct PipelineStats {
    pub fetch: Option<FetchStats>,
    pub scan: Option<ScanStats>,
    pub diff: Option<(usize, usize)>,
    pub transfer: Option<TransferProgressVm>,
}

pub type PipelineRunId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineStep {
    Fetch,
    Scan,
    Diff,
    Execute,
    PostScan,
}

#[derive(Debug, Clone)]
pub enum PipelineRunEvent {
    Started {
        profile_id: ProfileId,
    },
    StepChanged {
        step: PipelineStep,
        status: super::StepStatus,
        detail: String,
    },
    ScanStats {
        stats: fleet_scanner::ScanStats,
    },
    TransferProgress {
        snapshot: fleet_pipeline::TransferSnapshot,
    },
    PlanReady {
        plan: fleet_core::SyncPlan,
        diff_stats: (usize, usize),
        existing_mods: Vec<String>,
    },
    Completed,
    Failed {
        message: String,
    },
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct PipelineState {
    pub run_id: Option<PipelineRunId>,
    pub active_profile_id: Option<ProfileId>,

    pub fetch_status: StepStatus,
    pub scan_status: StepStatus,
    pub diff_status: StepStatus,
    pub sync_status: StepStatus,

    pub stats: PipelineStats,
    pub details: HashMap<PipelineStep, String>,
    pub plan_existing_mods: Option<Vec<String>>,
    pub error: Option<String>,
}

impl PipelineState {
    pub fn idle() -> Self {
        Self {
            run_id: None,
            active_profile_id: None,
            fetch_status: StepStatus::Pending,
            scan_status: StepStatus::Pending,
            diff_status: StepStatus::Pending,
            sync_status: StepStatus::Pending,
            stats: PipelineStats::default(),
            details: HashMap::new(),
            plan_existing_mods: None,
            error: None,
        }
    }

    pub fn idle_for(profile_id: Option<ProfileId>) -> Self {
        Self {
            active_profile_id: profile_id,
            ..Self::idle()
        }
    }

    pub fn starting(profile_id: ProfileId) -> Self {
        Self {
            run_id: None,
            active_profile_id: Some(profile_id),
            fetch_status: StepStatus::Pending,
            scan_status: StepStatus::Pending,
            diff_status: StepStatus::Pending,
            sync_status: StepStatus::Pending,
            stats: PipelineStats::default(),
            details: HashMap::new(),
            plan_existing_mods: None,
            error: None,
        }
    }

    pub fn with_run_id(mut self, run_id: Option<PipelineRunId>) -> Self {
        self.run_id = run_id;
        self
    }

    pub fn step_status(&self, step: PipelineStep) -> StepStatus {
        match step {
            PipelineStep::Fetch => self.fetch_status,
            PipelineStep::Scan => self.scan_status,
            PipelineStep::Diff => self.diff_status,
            PipelineStep::Execute | PipelineStep::PostScan => self.sync_status,
        }
    }

    pub fn set_step_status(&mut self, step: PipelineStep, status: StepStatus) {
        match step {
            PipelineStep::Fetch => self.fetch_status = status,
            PipelineStep::Scan => self.scan_status = status,
            PipelineStep::Diff => self.diff_status = status,
            PipelineStep::Execute | PipelineStep::PostScan => self.sync_status = status,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(
            (
                self.fetch_status,
                self.scan_status,
                self.diff_status,
                self.sync_status
            ),
            (StepStatus::Running, _, _, _)
                | (_, StepStatus::Running, _, _)
                | (_, _, StepStatus::Running, _)
                | (_, _, _, StepStatus::Running)
        )
    }

    pub fn is_terminal(&self) -> bool {
        self.error.is_some()
            || matches!(
                self.sync_status,
                StepStatus::Succeeded | StepStatus::Skipped | StepStatus::Failed
            )
    }
}
