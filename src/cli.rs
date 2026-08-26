use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "devmap",
    version,
    about = "Evidence-backed development maps for humans and AI agents"
)]
pub struct Cli {
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

