use anyhow::{Context, Result};
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::screensaver::ConnectionExt;
use x11rb::rust_connection::RustConnection;

/// The internal state of our idle detection state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleState {
    Monitoring,
    WarningSent,
    AutoPaused,
    ResumePrompted,
    ManualPaused,
    Stopped,
}

/// Queries X11 to determine the user's idle time in milliseconds.
pub struct IdleMonitor {
    conn: RustConnection,
    root: u32,
}

impl IdleMonitor {
    /// Creates a new connection to the X11 server and initializes the root window.
    pub fn new() -> Result<Self> {
        let (conn, screen_num) = x11rb::connect(None)
            .context("Failed to connect to X11 server. Ensure you are running under X11/XFCE.")?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        Ok(Self { conn, root })
    }

    /// Queries the MIT-SCREEN-SAVER extension to return the idle duration.
    pub fn get_idle_time(&self) -> Result<Duration> {
        let reply = self
            .conn
            .screensaver_query_info(self.root)
            .context("Failed to query screensaver extension. Is MIT-SCREEN-SAVER loaded?")?
            .reply()
            .context("Failed to receive screensaver query reply")?;

        Ok(Duration::from_millis(reply.ms_since_user_input as u64))
    }
}
