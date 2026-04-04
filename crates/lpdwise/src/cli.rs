use clap::Parser;

/// lpdwise — audio/video knowledge extraction CLI.
#[derive(Parser, Debug)]
#[command(name = "lpdwise", version, about)]
pub(crate) struct Cli {
    /// Input source: file path or URL.
    #[arg()]
    pub(crate) input: Option<String>,
}
