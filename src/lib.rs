pub mod canonical;
pub mod cli;
pub mod commands;
pub mod context;
pub mod domain;
pub mod error;
pub mod git;

use std::ffi::OsString;

use clap::Parser;

use crate::cli::{Cli, Command, CommonGroundCommand};
use crate::error::DevMapError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
}

pub fn run<I, T>(args: I) -> Result<CommandOutput, DevMapError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;

    match cli.command {
        Command::Init(args) => commands::init(args),
        Command::CommonGround {
            command: CommonGroundCommand::Approve(_),
        } => Err(DevMapError::UnsupportedCommand("common-ground approve")),
        Command::Status(_) => Err(DevMapError::UnsupportedCommand("status")),
    }
}
