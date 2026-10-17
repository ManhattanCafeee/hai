use std::process::ExitCode;

use clap::Parser;
use hai::cli::Cli;

#[tokio::main]
async fn main() -> ExitCode {
    match Cli::parse().execute().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
