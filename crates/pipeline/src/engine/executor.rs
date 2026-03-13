use crate::api::{OperationOutput, PipelineEventKind, PipelineNoticeLevel};
use crate::config::PipelineConfig;
use crate::engine::{layers, OperationContext, SessionControl};
use crate::operations;
use fleet_domain::health::OperationKind;
use fleet_domain::{ApiError, Profile};
use tower::ServiceExt;

pub async fn run_operation(
    config: PipelineConfig,
    session_id: u64,
    profile: Profile,
    operation: OperationKind,
    control: SessionControl,
) -> anyhow::Result<()> {
    let ctx = OperationContext::new(session_id, profile, operation, config, control);
    let emitter = ctx.emitter.clone();

    let service = layers::operation_service(operation);
    match service.oneshot(ctx).await {
        Ok(ctx) => {
            if let Some(report) = ctx.final_report {
                let output = match operation {
                    OperationKind::Assess(_) => OperationOutput::Assess(report),
                    OperationKind::Sync => OperationOutput::Sync(report),
                };
                emitter.emit(PipelineEventKind::Finished { output });
            } else if ctx.cancel.is_cancelled() {
                emitter.emit(PipelineEventKind::Canceled);
            } else {
                emitter.emit(PipelineEventKind::Failed {
                    error: ApiError::new("internal", "operation completed without final report"),
                });
            }
        }
        Err(err) => {
            if ctx_cancelled(&err) {
                emitter.emit(PipelineEventKind::Canceled);
            } else {
                let error = operations::map_error(&err);
                emitter.notice(
                    PipelineNoticeLevel::Error,
                    Some(error.code.clone()),
                    error.message.clone(),
                );
                emitter.emit(PipelineEventKind::Failed { error });
            }
        }
    }
    Ok(())
}

fn ctx_cancelled(err: &anyhow::Error) -> bool {
    err.to_string().contains("canceled")
}
