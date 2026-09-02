pub mod adapter;
pub mod canonical;
pub mod capture;
pub mod cli;
pub mod commands;
pub mod context;
pub mod domain;
pub mod error;
pub mod events;
pub mod git;
pub mod hook;
pub mod journal;

use std::ffi::OsString;

use clap::Parser;

use crate::cli::{AdapterCommand, Cli, Command, CommonGroundCommand, HookCommand};
use crate::error::DevMapError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
    pub exit_code: u8,
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
            command: CommonGroundCommand::Approve(args),
        } => commands::approve(args),
        Command::Status(args) => commands::status(args),
        Command::Adapter { command } => dispatch_adapter(command),
        Command::Hook { command } => dispatch_hook(command),
        Command::Mcp(_) => Err(DevMapError::UnsupportedCommand("mcp")),
    }
}

fn dispatch_adapter(command: AdapterCommand) -> Result<CommandOutput, DevMapError> {
    match command {
        AdapterCommand::Plan(args) => commands::adapter_plan(args),
        AdapterCommand::Install(args) => commands::adapter_install(args),
        AdapterCommand::Verify(args) => commands::adapter_verify(args),
        AdapterCommand::Uninstall(args) => commands::adapter_uninstall(args),
    }
}

fn dispatch_hook(command: HookCommand) -> Result<CommandOutput, DevMapError> {
    match command {
        HookCommand::Handle(args) => {
            let mut stdin = std::io::stdin();
            hook::handle_hook(args, &mut stdin)
        }
    }
}
