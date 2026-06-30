use crate::config::Config;
use crate::database::Database;
use crate::idle::{IdleMonitor, IdleState};
use crate::notifications::{self, IdleWarningResult, ResumePromptResult};
use crate::timer::{Timer, TimerState};
use crate::tray::{TrayEvent, WorkTimerTray};
use anyhow::{Context, Result};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;
use tracing::{error, info};

/// Unified application events handled by the main state machine loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    Tray(TrayEvent),
    IdleTime(Duration),
    IdleWarning(IdleWarningResult),
    ResumePrompt(ResumePromptResult),
    Tick,
}

/// Holds and coordinates all shared state and background threads.
pub struct AppState {
    db: Database,
    timer: Timer,
    event_tx: Sender<AppEvent>,
    event_rx: Receiver<AppEvent>,
    tray_handle: Option<ksni::Handle<WorkTimerTray>>,
    idle_state: IdleState,
    peak_idle_duration: Duration,
}

impl AppState {
    /// Creates a new AppState instance.
    pub fn new(db: Database) -> Result<Self> {
        let timer = Timer::new(&db).context("Failed to initialize timer state")?;
        let (event_tx, event_rx) = channel();

        let mut idle_state = IdleState::Stopped;
        match timer.state() {
            TimerState::Running => {
                idle_state = IdleState::Monitoring;
            }
            TimerState::Paused => {
                idle_state = IdleState::ManualPaused;
            }
            _ => {}
        }

        Ok(Self {
            db,
            timer,
            event_tx,
            event_rx,
            tray_handle: None,
            idle_state,
            peak_idle_duration: Duration::ZERO,
        })
    }

    /// Gets a sender clone for the unified event channel.
    #[allow(dead_code)]
    pub fn event_tx(&self) -> Sender<AppEvent> {
        self.event_tx.clone()
    }

    /// Starts the main event loop, initializing the tray interface and blocking until shutdown.
    pub fn run(&mut self, config: Config) -> Result<()> {
        info!("Starting main application event loop");

        // 1. Set up the system tray event channel
        let (tray_tx, tray_rx) = channel();
        let app_tx = self.event_tx.clone();

        // Spawn a thread to bridge TrayEvents into unified AppEvents
        std::thread::spawn(move || {
            while let Ok(tray_event) = tray_rx.recv() {
                if let Err(e) = app_tx.send(AppEvent::Tray(tray_event)) {
                    error!("Failed to forward tray event: {:?}", e);
                    break;
                }
            }
        });

        // Initialize and start the system tray
        let tray = WorkTimerTray::new(self.timer.state(), tray_tx);
        let tray_handle = tray.run();
        self.tray_handle = Some(tray_handle);
        self.sync_tray_ui();

        // Spawn a thread to send a Tick event every second
        let tick_tx = self.event_tx.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                if tick_tx.send(AppEvent::Tick).is_err() {
                    break; // main thread hung up
                }
            }
        });

        // 2. Set up X11 IdleMonitor if display environment is present
        let mut has_idle_monitor = false;
        let monitor = match IdleMonitor::new() {
            Ok(m) => {
                has_idle_monitor = true;
                Some(m)
            }
            Err(e) => {
                error!("X11 IdleMonitor initialization failed: {:?}", e);
                error!("Idle detection will be disabled.");
                None
            }
        };

        // Spawn the idle monitor sensor thread (checks every 5 seconds)
        if has_idle_monitor {
            let monitor = monitor.unwrap();
            let idle_tx = self.event_tx.clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(5));
                    match monitor.get_idle_time() {
                        Ok(idle_time) => {
                            if idle_tx.send(AppEvent::IdleTime(idle_time)).is_err() {
                                break; // main thread hung up
                            }
                        }
                        Err(e) => {
                            error!("Error querying idle time from X11: {:?}", e);
                        }
                    }
                }
            });
        }

        // Main event loop
        while let Ok(event) = self.event_rx.recv() {
            match event {
                AppEvent::Tray(tray_event) => {
                    info!("Received tray event: {:?}", tray_event);
                    match tray_event {
                        TrayEvent::Start => match self.timer.start(&self.db) {
                            Ok(()) => {
                                self.idle_state = IdleState::Monitoring;
                                notifications::show_simple(
                                    "Timer Started",
                                    "The work session has started.",
                                );
                            }
                            Err(e) => error!("Failed to start timer from tray: {:?}", e),
                        },
                        TrayEvent::Pause => match self.timer.pause(&self.db, false) {
                            Ok(()) => {
                                self.idle_state = IdleState::ManualPaused;
                                notifications::show_simple(
                                    "Timer Paused",
                                    "The work session has been paused.",
                                );
                            }
                            Err(e) => error!("Failed to pause timer from tray: {:?}", e),
                        },
                        TrayEvent::Resume => match self.timer.resume(&self.db) {
                            Ok(()) => {
                                self.idle_state = IdleState::Monitoring;
                                notifications::show_simple(
                                    "Timer Resumed",
                                    "The work session has resumed.",
                                );
                            }
                            Err(e) => error!("Failed to resume timer from tray: {:?}", e),
                        },
                        TrayEvent::Stop => match self.timer.stop(&self.db) {
                            Ok(()) => {
                                self.idle_state = IdleState::Stopped;
                                notifications::show_simple(
                                    "Timer Stopped",
                                    "The work session has been stopped.",
                                );
                            }
                            Err(e) => error!("Failed to stop timer from tray: {:?}", e),
                        },
                        TrayEvent::Exit => {
                            info!("Exit requested from tray context menu");
                            if matches!(
                                self.timer.state(),
                                TimerState::Running | TimerState::Paused
                            ) {
                                let _ = self.timer.stop(&self.db);
                            }
                            if let Some(ref handle) = self.tray_handle {
                                handle.shutdown();
                            }
                            break;
                        }
                    }

                    // Update the tray UI to match the new state
                    self.sync_tray_ui();
                }

                AppEvent::Tick => {
                    // Only update the tray UI on tick if the timer is running
                    if self.timer.state() == TimerState::Running {
                        self.sync_tray_ui();
                    }
                }

                AppEvent::IdleTime(duration) => {
                    let timer_state = self.timer.state();

                    // Track peak idle duration during any non-active phase
                    if timer_state == TimerState::Running && duration > self.peak_idle_duration {
                        self.peak_idle_duration = duration;
                    }

                    if timer_state == TimerState::Running
                        && self.idle_state == IdleState::Monitoring
                    {
                        let threshold = Duration::from_secs(config.idle_timeout_minutes * 60);
                        if duration >= threshold {
                            info!(
                                "User idle duration ({:?}) exceeded threshold ({:?}). Sending warning.",
                                duration, threshold
                            );
                            self.idle_state = IdleState::WarningSent;

                            // Show warning notification and handle response asynchronously
                            let warn_tx = self.event_tx.clone();
                            let (resp_tx, resp_rx) = channel();

                            std::thread::spawn(move || {
                                if let Ok(res) = resp_rx.recv() {
                                    let _ = warn_tx.send(AppEvent::IdleWarning(res));
                                }
                            });

                            let idle_time_str = format_duration_friendly(threshold);
                            notifications::show_idle_warning(
                                idle_time_str,
                                config.auto_pause_after_notification_seconds as u32,
                                resp_tx,
                            );
                        }
                    } else if timer_state == TimerState::Running
                        && self.idle_state == IdleState::WarningSent
                    {
                        // If user returns while warning is visible (idle duration drops below 5s)
                        if duration < Duration::from_secs(5) {
                            info!(
                                "Activity detected during warning period. Resetting to Monitoring."
                            );
                            self.idle_state = IdleState::Monitoring;
                            self.peak_idle_duration = Duration::ZERO;
                        }
                    } else if timer_state == TimerState::Paused
                        && self.idle_state == IdleState::AutoPaused
                    {
                        // User returned after auto pause
                        if duration < Duration::from_secs(5) {
                            info!("Activity detected after auto-pause. Prompting to resume.");
                            self.idle_state = IdleState::ResumePrompted;

                            let inactive_time_str =
                                format_duration_friendly(self.peak_idle_duration);
                            self.peak_idle_duration = Duration::ZERO;

                            // Show resume prompt and handle response asynchronously
                            let resume_tx = self.event_tx.clone();
                            let (resp_tx, resp_rx) = channel();

                            std::thread::spawn(move || {
                                if let Ok(res) = resp_rx.recv() {
                                    let _ = resume_tx.send(AppEvent::ResumePrompt(res));
                                }
                            });

                            notifications::show_resume_prompt(inactive_time_str, resp_tx);
                        }
                    }
                }

                AppEvent::IdleWarning(result) => {
                    if self.idle_state == IdleState::WarningSent {
                        info!("Idle warning result: {:?}", result);
                        match result {
                            IdleWarningResult::Continue => {
                                self.idle_state = IdleState::Monitoring;
                            }
                            IdleWarningResult::Pause => {
                                if let Ok(()) = self.timer.pause(&self.db, false) {
                                    self.idle_state = IdleState::ManualPaused;
                                    self.sync_tray_ui();
                                }
                            }
                            IdleWarningResult::Ignore => {
                                // Auto pause the timer
                                if let Ok(()) = self.timer.pause(&self.db, true) {
                                    self.idle_state = IdleState::AutoPaused;
                                    self.sync_tray_ui();
                                    notifications::show_simple(
                                        "Timer Paused Automatically",
                                        "The timer was paused due to inactivity.",
                                    );
                                }
                            }
                        }
                    }
                }

                AppEvent::ResumePrompt(result) => {
                    if self.idle_state == IdleState::ResumePrompted {
                        info!("Resume prompt result: {:?}", result);
                        match result {
                            ResumePromptResult::Resume => {
                                if let Ok(()) = self.timer.resume(&self.db) {
                                    self.idle_state = IdleState::Monitoring;
                                    self.sync_tray_ui();
                                    notifications::show_simple(
                                        "Timer Resumed",
                                        "The work session has resumed.",
                                    );
                                }
                            }
                            ResumePromptResult::RemainPaused => {
                                self.idle_state = IdleState::ManualPaused;
                            }
                        }
                    }
                }
            }
        }

        info!("Application event loop terminated");
        Ok(())
    }

    fn sync_tray_ui(&self) {
        if let Some(ref handle) = self.tray_handle {
            let new_state = self.timer.state();
            let elapsed = self.db.get_today_elapsed_time().unwrap_or(Duration::ZERO);
            handle.update(move |t| {
                t.update_state(new_state, elapsed);
            });
        }
    }
}

fn format_duration_friendly(d: Duration) -> String {
    let secs = d.as_secs();
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let rem_secs = secs % 60;

    if hours > 0 {
        if mins > 0 {
            format!("{}h {}m", hours, mins)
        } else {
            format!("{}h", hours)
        }
    } else if mins > 0 {
        if rem_secs > 0 {
            format!("{}m {}s", mins, rem_secs)
        } else {
            format!("{}m", mins)
        }
    } else {
        format!("{}s", rem_secs)
    }
}
