mod cli;
mod doctor;
mod error;
mod input;

use clap::Parser;

use crate::cli::{Cli, Command};
use crate::error::AppError;

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Doctor) => {
            let results = doctor::run_doctor().await;
            doctor::print_doctor_results(&results);
        }
        None => {
            let source = input::resolve_input(cli.input.as_deref())?;
            println!("lpdwise — audio/video knowledge extraction");
            println!("Input source: {source:?}");
        }
    }

    Ok(())
}
