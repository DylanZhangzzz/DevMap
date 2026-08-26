use thiserror::Error;

use std::path::PathBuf;

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

    #[error("invalid domain value: {0} must not be blank")]
    InvalidDomain(&'static str),

    #[error("path is not inside a Git repository: {0}")]
    NotGitRepository(PathBuf),

    #[error("Git command failed ({command}): {stderr}")]
    GitCommand { command: String, stderr: String },

    #[error("Git command returned non-UTF-8 output: {0}")]
    NonUtf8GitOutput(String),

    #[error("I/O operation failed: {0}")]
    Io(#[from] std::io::Error),
}
