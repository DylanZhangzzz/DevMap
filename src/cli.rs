use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "devmap",
    version,
    about = "Evidence-backed development maps for humans and AI agents"
)]
pub struct Cli {
    /// DevMap-owned hook binding marker (used by installed host configuration).
    #[arg(long, global = true, hide = true)]
    pub binding_id: Option<String>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a reviewable Common Ground draft.
    Init(InitArgs),
    /// Review and approve Common Ground.
    CommonGround {
        #[command(subcommand)]
        command: CommonGroundCommand,
    },
    /// Verify and summarize a Context Repository.
    Status(StatusArgs),
    /// Plan, install, verify, or remove a project-local host adapter.
    Adapter {
        #[command(subcommand)]
        command: AdapterCommand,
    },
    /// Normalize a native host lifecycle event into capture events.
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },
    /// Run the generic MCP capture endpoint.
    Mcp(McpArgs),
}

#[derive(Debug, Subcommand)]
pub enum AdapterCommand {
    /// Show the bindings an adapter would install.
    Plan(AdapterPlanArgs),
    /// Install project-local adapter bindings.
    Install(AdapterInstallArgs),
    /// Verify installed adapter bindings and capabilities.
    Verify(AdapterVerifyArgs),
    /// Remove DevMap-owned adapter bindings.
    Uninstall(AdapterUninstallArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AdapterHost {
    Codex,
    Claude,
    #[value(name = "generic-mcp")]
    GenericMcp,
}

#[derive(Debug, Args)]
pub struct AdapterPlanArgs {
    #[arg(long)]
    pub source: PathBuf,
    #[arg(long)]
    pub host: AdapterHost,
    /// Plan an installation or a removal. The emitted digest approves only this action.
    #[arg(long, value_enum, default_value_t = AdapterPlanAction::Install)]
    pub action: AdapterPlanAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AdapterPlanAction {
    Install,
    Uninstall,
}

#[derive(Debug, Args)]
pub struct AdapterInstallArgs {
    #[arg(long)]
    pub source: PathBuf,
    #[arg(long)]
    pub host: AdapterHost,
    /// Exact digest emitted by `adapter plan` after review.
    #[arg(long)]
    pub plan_digest: String,
}

#[derive(Debug, Args)]
pub struct AdapterVerifyArgs {
    #[arg(long)]
    pub source: PathBuf,
    #[arg(long)]
    pub host: Option<AdapterHost>,
}

#[derive(Debug, Args)]
pub struct AdapterUninstallArgs {
    #[arg(long)]
    pub source: PathBuf,
    #[arg(long)]
    pub host: AdapterHost,
    /// Exact digest emitted by `adapter plan --action uninstall` after review.
    #[arg(long)]
    pub plan_digest: String,
}

#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// Read and persist one normalized native host lifecycle event from standard input.
    Handle(HookHandleArgs),
}

#[derive(Debug, Args)]
pub struct HookHandleArgs {
    #[arg(long, default_value = ".")]
    pub source: PathBuf,
    #[arg(long)]
    pub host: AdapterHost,
    #[arg(long)]
    pub event: String,
}

#[derive(Debug, Args)]
pub struct McpArgs {
    #[arg(long)]
    pub source: PathBuf,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(long)]
    pub source: PathBuf,
    #[arg(long)]
    pub context: PathBuf,
    #[arg(long)]
    pub goal: String,
    #[arg(long)]
    pub requirement: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum CommonGroundCommand {
    /// Approve and promote the current draft.
    Approve(ApproveArgs),
}

#[derive(Debug, Args)]
pub struct ApproveArgs {
    #[arg(long)]
    pub context: PathBuf,
    #[arg(long)]
    pub actor: String,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    #[arg(long)]
    pub context: PathBuf,
    #[arg(long)]
    pub json: bool,
}
