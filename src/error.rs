use thiserror::Error;

#[derive(Debug, Error)]
pub enum DevMapError {
    #[error(transparent)]
    Cli(#[from] clap::Error),

    #[error("command is not implemented yet: {0}")]
    UnsupportedCommand(&'static str),

    #[error("floating point values are not allowed in canonical evidence")]
    FloatingPointNotCanonical,

    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}
