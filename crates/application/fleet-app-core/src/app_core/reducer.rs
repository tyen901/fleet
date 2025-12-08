use crate::domain::{AppState, BootState, Route};
use crate::pipeline::{PipelineRunEvent, PipelineStep, StepStatus};

use super::events::DomainEvent;

pub fn reduce(mut state: AppState, ev: DomainEvent) -> AppState {
    match ev {
        DomainEvent::BootLoadingStarted => {
            state.boot = BootState::Loading;
        }

        DomainEvent::InitialStateLoaded { profiles, settings } => {
            state.profiles = profiles;
            state.settings = settings;
            state.selected_profile_id = state.profiles.first().map(|p| p.id.clone());
            state.route = state
                .selected_profile_id
                .clone()
                .map(Route::ProfileDashboard)
                .unwrap_or(Route::ProfileHub);
            state.boot = BootState::Ready;
        }

        DomainEvent::BootFailed { message } => {
            state.boot = BootState::Failed(message);
        }

        DomainEvent::RouteChanged(r) => state.route = r,

        DomainEvent::DraftOpened(p) => state.editor_draft = Some(p),
        DomainEvent::DraftCancelled => state.editor_draft = None,
        DomainEvent::DraftCommitted(p) => {
            if let Some(ix) = state.profiles.iter().position(|x| x.id == p.id) {
                state.profiles[ix] = p;
            } else {
                state.profiles.push(p);
            }
            state.editor_draft = None;
        }

        DomainEvent::PipelineEvent { run_id: _, ev } => apply_pipeline_event(&mut state, ev),

        DomainEvent::UserError(msg) => {
            state.pipeline.error = Some(msg);
        }
    }
    state
}

fn apply_pipeline_event(state: &mut AppState, ev: PipelineRunEvent) {
    match ev {
        PipelineRunEvent::Started { profile_id } => {
            state.pipeline.error = None;
            state.last_plan = None;
            state.pipeline = crate::pipeline::PipelineState::starting(profile_id)
                .with_run_id(state.pipeline.run_id);
        }

        PipelineRunEvent::StepChanged {
            step,
            status,
            detail,
        } => {
            state.pipeline.set_step_status(step, status);
            state.pipeline.details.insert(step, detail);
        }

        PipelineRunEvent::ScanStats { stats } => {
            state.pipeline.stats.scan = Some(stats);
        }

        PipelineRunEvent::TransferProgress { snapshot } => {
            state.pipeline.stats.transfer = Some(crate::pipeline::TransferProgressVm {
                downloaded_files: snapshot.downloaded_files,
                total_files: snapshot.total_files,
                downloaded_bytes: snapshot.downloaded_bytes,
                total_bytes: snapshot.total_bytes,
                speed_bps: snapshot.speed_bps,
                failed_count: snapshot.failed_count,
                active_files: snapshot
                    .in_flight
                    .into_iter()
                    .map(|f| crate::pipeline::ActiveTransferFileVm {
                        mod_name: f.mod_name,
                        rel_path: f.rel_path,
                        bytes_downloaded: f.bytes_downloaded,
                        total_bytes: f.total_bytes,
                    })
                    .collect(),
            });
        }

        PipelineRunEvent::PlanReady {
            plan,
            diff_stats,
            existing_mods,
        } => {
            state.last_plan = Some(plan);
            state.pipeline.stats.diff = Some(diff_stats);
            state.pipeline.plan_existing_mods = Some(existing_mods);
            state
                .pipeline
                .set_step_status(PipelineStep::Diff, StepStatus::Succeeded);
        }

        PipelineRunEvent::Completed => {
            state
                .pipeline
                .set_step_status(PipelineStep::Execute, StepStatus::Succeeded);
        }

        PipelineRunEvent::Failed { message } => {
            state.pipeline.error = Some(message);
            for step in [
                PipelineStep::Fetch,
                PipelineStep::Scan,
                PipelineStep::Diff,
                PipelineStep::Execute,
                PipelineStep::PostScan,
            ] {
                if state.pipeline.step_status(step) == StepStatus::Running {
                    state.pipeline.set_step_status(step, StepStatus::Failed);
                }
            }
        }

        PipelineRunEvent::Cancelled => {
            state.pipeline.error = Some("Operation cancelled by user".into());
            for step in [
                PipelineStep::Fetch,
                PipelineStep::Scan,
                PipelineStep::Diff,
                PipelineStep::Execute,
                PipelineStep::PostScan,
            ] {
                if state.pipeline.step_status(step) == StepStatus::Running {
                    state.pipeline.set_step_status(step, StepStatus::Failed);
                }
            }
        }
    }
}
