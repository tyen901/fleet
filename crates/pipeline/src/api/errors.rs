#[derive(Debug, thiserror::Error)]
pub enum PipelineStartError {
    #[error("duplicate_session_id")]
    DuplicateSessionId,
}
