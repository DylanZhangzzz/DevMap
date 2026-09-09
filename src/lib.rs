pub mod adapter;
pub mod application;
pub mod canonical;
pub mod capture;
pub mod cli;
pub mod commands;
pub mod context;
pub mod dock;
pub mod dock_asset;
pub mod domain;
pub mod error;
pub mod events;
pub(crate) mod fs_security;
pub mod git;
mod git_process;
pub mod git_relationship;
pub mod git_topology;
pub mod hook;
pub mod journal;
pub mod mcp;
pub mod mutation;
pub mod mutation_proxy;
pub mod presence;
pub mod proxy;
pub mod route_plan;
pub mod runtime;
pub mod store;
mod summary;
pub mod viewer;
pub mod worktrees;

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
        Command::Runtime(args) => runtime::dispatch(args),
        Command::Storage { command } => store::migration::dispatch(command),
        Command::Init(args) => commands::init(args),
        Command::CommonGround {
            command: CommonGroundCommand::Approve(args),
        } => commands::approve(args),
        Command::Status(args) => commands::status(args),
        Command::Agents(args) => dock::agents(args),
        Command::View(args) => match args.live {
            true => viewer::run_live_shared(&args.source),
            false => Err(DevMapError::UnsupportedCommand("canonical topology viewer")),
        },
        Command::Adapter { command } => dispatch_adapter(command),
        Command::Hook { command } => dispatch_hook(command),
        Command::Mcp(args) => {
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            mcp::serve_mcp_shared(&args.source, stdin.lock(), stdout.lock())?;
            Ok(CommandOutput {
                stdout: String::new(),
                exit_code: 0,
            })
        }
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
            hook::handle_hook_shared(args, &mut stdin)
        }
    }
}
