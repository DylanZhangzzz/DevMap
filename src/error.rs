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

    #[error("Context Repository path is not empty: {0}")]
    ContextPathNotEmpty(PathBuf),

    #[error("path is not a DevMap Context Repository: {0}")]
    NotContextRepository(PathBuf),

    #[error("invalid canonical object kind: {0}")]
    InvalidObjectKind(String),

    #[error("content-address collision at {0}")]
    ContentAddressCollision(String),

    #[error("refusing to commit unexpected Context Repository paths: {0:?}")]
    UnexpectedContextPaths(Vec<String>),

    #[error("Git returned malformed porcelain status")]
    MalformedGitStatus,

    #[error("failed to format timestamp: {0}")]
    TimeFormat(#[from] time::error::Format),

    #[error("source and Context repositories must be independent (source: {source_path}, context: {context_path})")]
    RepositoriesOverlap {
        source_path: PathBuf,
        context_path: PathBuf,
    },

    #[error("invalid Context Repository path: {0}")]
    InvalidContextPath(PathBuf),

    #[error("invalid requirement locator: {0}")]
    InvalidRequirementLocator(String),

    #[error("requirement document is outside the source repository: {0}")]
    RequirementOutsideSource(PathBuf),

    #[error("requirement anchor '{anchor}' must match exactly once; found {matches}")]
    RequirementAnchorMatch { anchor: String, matches: usize },

    #[error("a conflicting Common Ground draft already exists")]
    ConflictingCommonGroundDraft,

    #[error("unexpected Context Repository branch: {0}")]
    UnexpectedContextBranch(String),

    #[error("Context Repository must be clean before approval: {0}")]
    ContextNotClean(String),

    #[error("Common Ground draft is missing")]
    MissingCommonGroundDraft,
}
