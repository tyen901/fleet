use fleet_domain::health::{AssessScope, OperationKind};

pub(crate) fn operation_kind_label(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Assess(AssessScope::Local) => "assess_local",
        OperationKind::Assess(AssessScope::Remote) => "assess_remote",
        OperationKind::Sync => "sync",
    }
}

pub(crate) fn log_operation_start_request(
    profile_id: &str,
    operation_kind: OperationKind,
    op: &str,
) {
    tracing::info!(
        flow_kind = operation_kind_label(operation_kind),
        profile_id = %profile_id,
        op = op,
        "flow start requested"
    );
}

pub(crate) fn log_operation_spawn_success(
    profile_id: &str,
    operation_kind: OperationKind,
    session_id: u64,
    op: &str,
) {
    tracing::info!(
        flow_kind = operation_kind_label(operation_kind),
        profile_id = %profile_id,
        session_id = session_id,
        op = op,
        outcome = "ok",
        "flow session started"
    );
}

pub(crate) fn log_operation_spawn_failure(
    profile_id: &str,
    operation_kind: OperationKind,
    op: &str,
    code: &str,
    reason: &str,
) {
    tracing::error!(
        flow_kind = operation_kind_label(operation_kind),
        profile_id = %profile_id,
        op = op,
        outcome = "failed",
        code = code,
        reason = reason,
        "flow start failed"
    );
}
