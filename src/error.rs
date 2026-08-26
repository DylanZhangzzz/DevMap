use thiserror::Error;

#[derive(Debug, Error)]
pub enum DevMapError {
    #[error(transparent)]
    Cli(#[from] clap::Error),

    #[error("command is not implemented yet: {0}")]
    UnsupportedCommand(&'static str),
}

