#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("the interactive card exceeds its size limit")]
    TooLarge,
    #[error("invalid interactive card JSON")]
    Json(#[from] serde_json::Error),
    #[error("invalid interactive card: {0}")]
    Invalid(&'static str),
}
