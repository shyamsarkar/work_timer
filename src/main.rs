use anyhow::{Context, Result};
use std::env;

mod config;
mod database;
mod idle;
mod logging;
mod notifications;
mod state;
mod timer;
mod tray;

fn main() -> Result<()> {
    // 1. Initialize logging
    logging::init().context("Failed to initialize logging")?;

    tracing::info!("WorkTimer starting up");

    // 2. Load configuration and configure desktop launcher integration
    let cfg = config::load().context("Failed to load config")?;
    config::handle_desktop_integration()
        .context("Failed to configure desktop launcher integration")?;

    // 3. Open Database and run migrations
    let db = database::Database::new().context("Failed to open database")?;

    // 4. Initialize Timer (and recover state)
    let mut timer = timer::Timer::new(&db).context("Failed to initialize timer")?;

    // 5. Parse command line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 {
        let command = args[1].as_str();
        match command {
            "start" => {
                timer.start(&db).context("Failed to start timer")?;
                println!("State transitioned: {:?}", timer.state());
            }
            "pause" => {
                timer.pause(&db, false).context("Failed to pause timer")?;
                println!("State transitioned: {:?}", timer.state());
            }
            "auto-pause" => {
                timer
                    .pause(&db, true)
                    .context("Failed to auto-pause timer")?;
                println!("State transitioned: {:?}", timer.state());
            }
            "resume" => {
                timer.resume(&db).context("Failed to resume timer")?;
                println!("State transitioned: {:?}", timer.state());
            }
            "stop" => {
                timer.stop(&db).context("Failed to stop timer")?;
                println!("State transitioned: {:?}", timer.state());
            }
            "status" => {
                println!("Current State: {:?}", timer.state());
                if let Some(session_id) = timer.current_session_id() {
                    println!("Active Session ID: {}", session_id);
                } else {
                    println!("No active session.");
                }
                if let Some(session) = db.get_last_session()? {
                    println!("Last Session Details:");
                    println!("  ID:         {}", session.id);
                    println!("  Started At: {}", session.started_at.to_rfc3339());
                    if let Some(ended) = session.ended_at {
                        println!("  Ended At:   {}", ended.to_rfc3339());
                    } else {
                        println!("  Ended At:   (Active/Dangling)");
                    }
                } else {
                    println!("No sessions recorded yet.");
                }
            }
            _ => {
                eprintln!(
                    "Unknown command. Use: start | pause | auto-pause | resume | stop | status"
                );
                std::process::exit(1);
            }
        }
        tracing::info!("WorkTimer CLI command executed successfully");
        return Ok(());
    }

    // Default: Running in tray mode
    let mut app_state = state::AppState::new(db).context("Failed to initialize AppState")?;

    println!("WorkTimer tray application running. Use the system tray icon to interact.");
    app_state
        .run(cfg)
        .context("Error running AppState event loop")?;

    Ok(())
}
