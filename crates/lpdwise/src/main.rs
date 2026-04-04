mod cli;
mod error;

use clap::Parser;

use crate::cli::Cli;
use crate::error::AppError;

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let _cli = Cli::parse();
    println!("lpdwise — audio/video knowledge extraction");
    Ok(())
}
