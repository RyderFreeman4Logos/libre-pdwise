use clap::{Parser, Subcommand};

/// lpdwise — audio/video knowledge extraction CLI.
#[derive(Parser, Debug)]
#[command(name = "lpdwise", version, about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,

    /// Input source: file path or URL (when no subcommand is given).
    #[arg(global = true)]
    pub(crate) input: Option<String>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Check that required external tools are installed.
    Doctor,
}
