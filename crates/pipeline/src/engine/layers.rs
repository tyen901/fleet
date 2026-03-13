use crate::engine::OperationContext;
use crate::operations;
use fleet_domain::health::OperationKind;
use tower::{service_fn, util::BoxCloneService, ServiceExt};

pub fn operation_service(
    operation: OperationKind,
) -> BoxCloneService<OperationContext, OperationContext, anyhow::Error> {
    service_fn(move |ctx: OperationContext| async move {
        match operation {
            OperationKind::Assess(scope) => operations::run_assess(ctx, scope).await,
            OperationKind::Sync => operations::run_sync(ctx).await,
        }
    })
    .boxed_clone()
}
