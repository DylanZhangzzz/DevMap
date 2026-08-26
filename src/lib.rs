pub mod cli;
pub mod error;

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

    let command = match cli.command {
        Command::Init(_) => "init",
        Command::CommonGround {
            command: CommonGroundCommand::Approve(_),
        } => "common-ground approve",
        Command::Status(_) => "status",
    };

    Err(DevMapError::UnsupportedCommand(command))
}

