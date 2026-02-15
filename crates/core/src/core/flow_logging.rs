use fleet_domain::ProfileId;
use fleet_flow::FlowKind;

pub(crate) fn flow_kind_label(kind: FlowKind) -> &'static str {
    match kind {
        FlowKind::Sync => "sync",
        FlowKind::Repair => "repair",
        FlowKind::Check => "check",
    }
}

pub(crate) fn log_flow_start_request(profile_id: &str, flow_kind: FlowKind, op: &str) {
    tracing::info!(
        flow_kind = flow_kind_label(flow_kind),
        profile_id = %profile_id,
        op = op,
        "flow start requested"
    );
}

pub(crate) fn log_flow_spawn_success(
    profile_id: &str,
    flow_kind: FlowKind,
    session_id: u64,
    op: &str,
) {
    tracing::info!(
        flow_kind = flow_kind_label(flow_kind),
        profile_id = %profile_id,
        session_id = session_id,
        op = op,
        outcome = "ok",
        "flow session started"
    );
}

pub(crate) fn log_flow_spawn_failure(
    profile_id: &str,
    flow_kind: FlowKind,
    op: &str,
    code: &str,
    reason: &str,
) {
    tracing::error!(
        flow_kind = flow_kind_label(flow_kind),
        profile_id = %profile_id,
        op = op,
        outcome = "failed",
        code = code,
        reason = reason,
        "flow start failed"
    );
}

pub(crate) fn log_session_spawn_requested(
    profile_id: &ProfileId,
    flow_kind: FlowKind,
    session_id: u64,
) {
    tracing::info!(
        flow_kind = flow_kind_label(flow_kind),
        profile_id = %profile_id,
        session_id = session_id,
        op = "spawn",
        "flow session spawn requested"
    );
}

pub(crate) fn log_session_rejected_duplicate(profile_id: &ProfileId, flow_kind: FlowKind) {
    tracing::warn!(
        flow_kind = flow_kind_label(flow_kind),
        profile_id = %profile_id,
        op = "spawn",
        outcome = "rejected",
        reason = "profile_session_exists",
        "flow session rejected because one is already running"
    );
}

pub(crate) fn log_session_started(profile_id: &ProfileId, flow_kind: FlowKind, session_id: u64) {
    tracing::info!(
        flow_kind = flow_kind_label(flow_kind),
        profile_id = %profile_id,
        session_id = session_id,
        op = "start",
        outcome = "ok",
        "flow session started event emitted"
    );
}

pub(crate) fn log_session_input_routed(
    profile_id: &ProfileId,
    flow_kind: FlowKind,
    session_id: u64,
    op: &str,
) {
    tracing::info!(
        flow_kind = flow_kind_label(flow_kind),
        profile_id = %profile_id,
        session_id = session_id,
        op = op,
        outcome = "ok",
        "flow input routed"
    );
}

pub(crate) fn log_session_input_rejected(session_id: u64, op: &str, reason: &str) {
    tracing::warn!(
        flow_kind = "unknown",
        session_id = session_id,
        op = op,
        outcome = "rejected",
        reason = reason,
        "flow input rejected"
    );
}

pub(crate) fn log_session_cancel_requested(
    profile_id: &ProfileId,
    flow_kind: FlowKind,
    session_id: u64,
) {
    tracing::info!(
        flow_kind = flow_kind_label(flow_kind),
        profile_id = %profile_id,
        session_id = session_id,
        op = "cancel",
        outcome = "requested",
        "flow cancel requested"
    );
}

pub(crate) fn log_session_cancel_unknown(session_id: u64) {
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
    flow_kind: FlowKind,
    session_id: u64,
    outcome: &'static str,
    reason: Option<&str>,
) {
    match outcome {
        "failed" => tracing::error!(
            flow_kind = flow_kind_label(flow_kind),
            profile_id = %profile_id,
            session_id = session_id,
            op = "terminal",
            outcome = outcome,
            reason = reason.unwrap_or("error"),
            "flow session terminated"
        ),
        _ => tracing::info!(
            flow_kind = flow_kind_label(flow_kind),
            profile_id = %profile_id,
            session_id = session_id,
            op = "terminal",
            outcome = outcome,
            reason = reason.unwrap_or(""),
            "flow session terminated"
        ),
    }
}

pub(crate) fn log_session_cleanup(profile_id: &ProfileId, flow_kind: FlowKind, session_id: u64) {
    tracing::debug!(
        flow_kind = flow_kind_label(flow_kind),
        profile_id = %profile_id,
        session_id = session_id,
        op = "cleanup",
        outcome = "ok",
        "flow session cleanup complete"
    );
}
