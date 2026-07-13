use clap::Parser;
use previously_on::config::Cli;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "previously_on=info".into()),
        )
        .with_target(false)
        .init();

    match previously_on::config::run(Cli::parse()).await {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!(
                "error: {}",
                previously_on::redaction::redact_excerpt(&format!("{error:#}"))
            );
            ExitCode::FAILURE
        }
    }
}
