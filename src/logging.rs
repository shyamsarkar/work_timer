use anyhow::{Context, Result};
use std::fs::{OpenOptions, create_dir_all};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Initializes the logging system, writing files to `~/.local/state/worktimer/app.log`.
pub fn init() -> Result<()> {
    let mut log_dir = dirs::state_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("state")))
        .context("Could not determine local state directory")?;

    log_dir.push("worktimer");
    create_dir_all(&log_dir).context("Failed to create log directory")?;

    let log_file_path = log_dir.join("app.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)
        .with_context(|| format!("Failed to open log file at {:?}", log_file_path))?;

    let file_layer = fmt::layer()
        .with_writer(file)
        .with_ansi(false) // Disable ANSI codes in files
        .with_target(true);

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .init();

    Ok(())
}
