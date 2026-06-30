use crate::database::Database;
use anyhow::{Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// The states of the timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimerState {
    Idle,
    Running,
    Paused,
    Stopped,
}

/// The core timer struct managing states and transition validation.
pub struct Timer {
    state: TimerState,
    current_session_id: Option<i64>,
}

impl Timer {
    /// Instantiates a new Timer, recovering any dangling active session state from the database.
    pub fn new(db: &Database) -> Result<Self> {
        let mut state = TimerState::Idle;
        let mut current_session_id = None;

        if let Some(session) = db.get_last_session()? {
            if session.ended_at.is_none() {
                current_session_id = Some(session.id);
                if let Some(event_type) = db.get_last_event_type(session.id)? {
                    match event_type.as_str() {
                        "PAUSE" | "AUTO_PAUSE" => {
                            state = TimerState::Paused;
                            tracing::info!(
                                "Recovered active session {} in PAUSED state",
                                session.id
                            );
                        }
                        _ => {
                            state = TimerState::Running;
                            tracing::info!(
                                "Recovered active session {} in RUNNING state",
                                session.id
                            );
                        }
                    }
                } else {
                    state = TimerState::Running;
                    tracing::info!(
                        "Recovered active session {} in RUNNING state (no events found)",
                        session.id
                    );
                }
            } else {
                state = TimerState::Stopped;
                tracing::info!(
                    "Last session {} was closed. Starting in STOPPED state.",
                    session.id
                );
            }
        } else {
            tracing::info!("No existing session found. Starting in IDLE state.");
        }

        Ok(Self {
            state,
            current_session_id,
        })
    }

    /// Returns the current state of the timer.
    pub fn state(&self) -> TimerState {
        self.state
    }

    /// Returns the active session ID if any.
    pub fn current_session_id(&self) -> Option<i64> {
        self.current_session_id
    }

    /// Starts a new timer session. Valid from `Idle` or `Stopped`.
    pub fn start(&mut self, db: &Database) -> Result<()> {
        match self.state {
            TimerState::Idle | TimerState::Stopped => {
                let now = Utc::now();
                let session_id = db.create_session(now)?;
                db.log_event(session_id, "START", now)?;
                self.current_session_id = Some(session_id);
                self.state = TimerState::Running;
                tracing::info!("Timer started (session {})", session_id);
                Ok(())
            }
            _ => bail!("Cannot start timer from state: {:?}", self.state),
        }
    }

    /// Pauses the running timer session. Valid from `Running`.
    pub fn pause(&mut self, db: &Database, is_auto: bool) -> Result<()> {
        match self.state {
            TimerState::Running => {
                let session_id = self.current_session_id.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Internal Error: Current session ID is missing in Running state"
                    )
                })?;
                let now = Utc::now();
                let event = if is_auto { "AUTO_PAUSE" } else { "PAUSE" };
                db.log_event(session_id, event, now)?;
                self.state = TimerState::Paused;
                tracing::info!("Timer paused ({}, session {})", event, session_id);
                Ok(())
            }
            _ => bail!("Cannot pause timer from state: {:?}", self.state),
        }
    }

    /// Resumes a paused timer session. Valid from `Paused`.
    pub fn resume(&mut self, db: &Database) -> Result<()> {
        match self.state {
            TimerState::Paused => {
                let session_id = self.current_session_id.ok_or_else(|| {
                    anyhow::anyhow!("Internal Error: Current session ID is missing in Paused state")
                })?;
                let now = Utc::now();
                db.log_event(session_id, "RESUME", now)?;
                self.state = TimerState::Running;
                tracing::info!("Timer resumed (session {})", session_id);
                Ok(())
            }
            _ => bail!("Cannot resume timer from state: {:?}", self.state),
        }
    }

    /// Stops the timer session. Valid from `Running` or `Paused`.
    pub fn stop(&mut self, db: &Database) -> Result<()> {
        match self.state {
            TimerState::Running | TimerState::Paused => {
                let session_id = self.current_session_id.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Internal Error: Current session ID is missing in {:?} state",
                        self.state
                    )
                })?;
                let now = Utc::now();
                db.log_event(session_id, "STOP", now)?;
                db.end_session(session_id, now)?;
                self.current_session_id = None;
                self.state = TimerState::Stopped;
                tracing::info!("Timer stopped (session {})", session_id);
                Ok(())
            }
            _ => bail!("Cannot stop timer from state: {:?}", self.state),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timer_transitions_and_db() -> Result<()> {
        let db = Database::in_memory()?;
        let mut timer = Timer::new(&db)?;

        // 1. Initial state
        assert_eq!(timer.state(), TimerState::Idle);
        assert_eq!(timer.current_session_id(), None);

        // 2. Start
        timer.start(&db)?;
        assert_eq!(timer.state(), TimerState::Running);
        let session_id = timer.current_session_id().unwrap();

        // Check DB has an active session
        let last_session = db.get_last_session()?.unwrap();
        assert_eq!(last_session.id, session_id);
        assert!(last_session.ended_at.is_none());
        assert_eq!(db.get_last_event_type(session_id)?.unwrap(), "START");

        // 3. Pause
        timer.pause(&db, false)?;
        assert_eq!(timer.state(), TimerState::Paused);
        assert_eq!(db.get_last_event_type(session_id)?.unwrap(), "PAUSE");

        // 4. Resume
        timer.resume(&db)?;
        assert_eq!(timer.state(), TimerState::Running);
        assert_eq!(db.get_last_event_type(session_id)?.unwrap(), "RESUME");

        // 5. Stop
        timer.stop(&db)?;
        assert_eq!(timer.state(), TimerState::Stopped);
        assert_eq!(timer.current_session_id(), None);

        // Check DB session is ended
        let last_session = db.get_last_session()?.unwrap();
        assert!(last_session.ended_at.is_some());
        assert_eq!(db.get_last_event_type(session_id)?.unwrap(), "STOP");

        Ok(())
    }

    #[test]
    fn test_invalid_transitions() -> Result<()> {
        let db = Database::in_memory()?;
        let mut timer = Timer::new(&db)?;

        // Cannot pause from Idle
        assert!(timer.pause(&db, false).is_err());
        // Cannot resume from Idle
        assert!(timer.resume(&db).is_err());
        // Cannot stop from Idle
        assert!(timer.stop(&db).is_err());

        timer.start(&db)?;
        // Cannot start again while running
        assert!(timer.start(&db).is_err());
        // Cannot resume while running
        assert!(timer.resume(&db).is_err());

        timer.pause(&db, false)?;
        // Cannot pause again while paused
        assert!(timer.pause(&db, false).is_err());
        // Cannot start while paused
        assert!(timer.start(&db).is_err());

        Ok(())
    }

    #[test]
    fn test_state_recovery() -> Result<()> {
        let db = Database::in_memory()?;

        // Start and pause a session
        {
            let mut timer = Timer::new(&db)?;
            timer.start(&db)?;
            timer.pause(&db, true)?; // Auto pause
        }

        // Recover state - should be Paused
        {
            let timer = Timer::new(&db)?;
            assert_eq!(timer.state(), TimerState::Paused);
            assert!(timer.current_session_id().is_some());
        }

        // Resume and leave running (e.g. application closed unexpectedly)
        {
            let mut timer = Timer::new(&db)?;
            timer.resume(&db)?;
        }

        // Recover state - should be Running
        {
            let timer = Timer::new(&db)?;
            assert_eq!(timer.state(), TimerState::Running);
        }

        // Stop session
        {
            let mut timer = Timer::new(&db)?;
            timer.stop(&db)?;
        }

        // Recover state - should be Stopped
        {
            let timer = Timer::new(&db)?;
            assert_eq!(timer.state(), TimerState::Stopped);
            assert!(timer.current_session_id().is_none());
        }

        Ok(())
    }
}
