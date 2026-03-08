use fleet_domain::health::{AssessScope, OperationKind};
use fleet_domain::ProfileId;

pub(crate) fn operation_kind_label(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Assess(AssessScope::Local) => "assess_local",
        OperationKind::Assess(AssessScope::Remote) => "assess_remote",
        OperationKind::Sync => "sync",
        OperationKind::RebuildInventory => "rebuild_inventory",
        OperationKind::Clean => "clean",
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

pub(crate) fn log_operation_spawn_requested(
    profile_id: &ProfileId,
    operation_kind: OperationKind,
    session_id: u64,
) {
    tracing::info!(
        flow_kind = operation_kind_label(operation_kind),
        profile_id = %profile_id,
        session_id = session_id,
        op = "spawn",
        "flow session spawn requested"
    );
}

pub(crate) fn log_operation_rejected_duplicate(
    profile_id: &ProfileId,
    operation_kind: OperationKind,
) {
    tracing::warn!(
        flow_kind = operation_kind_label(operation_kind),
        profile_id = %profile_id,
        op = "spawn",
        outcome = "rejected",
        reason = "profile_session_exists",
        "flow session rejected because one is already running"
    );
}

pub(crate) fn log_operation_started(
    profile_id: &ProfileId,
    operation_kind: OperationKind,
    session_id: u64,
) {
    tracing::info!(
        flow_kind = operation_kind_label(operation_kind),
        profile_id = %profile_id,
        session_id = session_id,
        op = "start",
        outcome = "ok",
        "flow session started event emitted"
    );
}

pub(crate) fn log_operation_cancel_requested(
    profile_id: &ProfileId,
    operation_kind: OperationKind,
    session_id: u64,
) {
    tracing::info!(
        flow_kind = operation_kind_label(operation_kind),
        profile_id = %profile_id,
        session_id = session_id,
        op = "cancel",
        outcome = "requested",
        "flow cancel requested"
    );
}

pub(crate) fn log_operation_cancel_unknown(session_id: u64) {
    tracing::warn!(
        flow_kind = "unknown",
        session_id = session_id,
        op = "cancel",
        outcome = "noop",
        reason = "unknown_session",
        "flow cancel ignored for unknown session"
    );
}

pub(crate) fn log_terminal_result(
    profile_id: &ProfileId,
    operation_kind: OperationKind,
    session_id: u64,
    outcome: &'static str,
    reason: Option<&str>,
) {
    match outcome {
        "failed" => tracing::error!(
            flow_kind = operation_kind_label(operation_kind),
            profile_id = %profile_id,
            session_id = session_id,
            op = "terminal",
            outcome = outcome,
            reason = reason.unwrap_or("error"),
            "flow session terminated"
        ),
        _ => tracing::info!(
            flow_kind = operation_kind_label(operation_kind),
            profile_id = %profile_id,
            session_id = session_id,
            op = "terminal",
            outcome = outcome,
            reason = reason.unwrap_or(""),
            "flow session terminated"
        ),
    }
}

pub(crate) fn log_operation_cleanup(
    profile_id: &ProfileId,
    operation_kind: OperationKind,
    session_id: u64,
) {
    tracing::debug!(
        flow_kind = operation_kind_label(operation_kind),
        profile_id = %profile_id,
        session_id = session_id,
        op = "cleanup",
        outcome = "ok",
        "flow session cleanup complete"
    );
}
