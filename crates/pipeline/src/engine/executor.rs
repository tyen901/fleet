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
            if matches!(
                operations::find_operation_error(&err),
                Some(operations::OperationError::Canceled)
            ) {
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

#[cfg(test)]
mod tests {
    use super::run_operation;
    use crate::api::PipelineEventKind;
    use crate::config::PipelineConfig;
    use crate::engine::{EventEmitter, SessionControl};
    use fleet_domain::health::{AssessScope, OperationKind};
    use fleet_domain::Profile;
    use tokio::sync::broadcast;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn canceled_operation_emits_canceled_event() {
        let (tx, mut rx) = broadcast::channel(32);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let control = SessionControl {
            cancel,
            emitter: EventEmitter::new(
                tx,
                7,
                "p1".to_string(),
                OperationKind::Assess(AssessScope::Local),
            ),
        };

        run_operation(
            PipelineConfig::new_default(),
            7,
            Profile {
                id: "p1".to_string(),
                name: "Profile".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/profile".to_string(),
                ..Default::default()
            },
            OperationKind::Assess(AssessScope::Local),
            control,
        )
        .await
        .expect("run operation");

        while let Ok(event) = rx.try_recv() {
            if matches!(event.kind, PipelineEventKind::Canceled) {
                return;
            }
        }

        panic!("expected canceled terminal event");
    }
}
