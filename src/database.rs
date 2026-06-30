use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, params};
use std::fs::create_dir_all;

/// Represents a session stored in the database.
#[derive(Debug, Clone)]
pub struct DbSession {
    pub id: i64,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// Database wrapper managing the connection to the SQLite database.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Creates an in-memory database for testing purposes.
    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory database")?;
        let mut db = Database { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Opens the SQLite database at `~/.local/share/worktimer/worktimer.db` and runs migrations.
    pub fn new() -> Result<Self> {
        let mut db_dir = dirs::data_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("share")))
            .context("Could not determine local data directory")?;

        db_dir.push("worktimer");
        create_dir_all(&db_dir).context("Failed to create database directory")?;

        let db_path = db_dir.join("worktimer.db");
        tracing::info!("Opening database at {:?}", db_path);

        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open database at {:?}", db_path))?;

        let mut db = Database { conn };
        if let Err(e) = db.migrate() {
            tracing::error!("Database migration failed: {:?}", e);
            return Err(e);
        }

        Ok(db)
    }

    /// Performs the database migrations to set up the sessions and events tables.
    fn migrate(&mut self) -> Result<()> {
        self.conn
            .execute("PRAGMA foreign_keys = ON;", [])
            .context("Failed to enable foreign keys")?;

        let user_version: i32 = self
            .conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .context("Failed to query user_version")?;

        if user_version < 1 {
            tracing::info!("Running database migration (user_version = 0 -> 1)");
            let tx = self
                .conn
                .transaction()
                .context("Failed to start transaction")?;

            tx.execute(
                "CREATE TABLE IF NOT EXISTS sessions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    started_at TEXT NOT NULL,
                    ended_at TEXT,
                    created_at TEXT NOT NULL
                );",
                [],
            )
            .context("Failed to create sessions table")?;

            tx.execute(
                "CREATE TABLE IF NOT EXISTS events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id INTEGER NOT NULL,
                    event_type TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
                );",
                [],
            )
            .context("Failed to create events table")?;

            tx.execute("PRAGMA user_version = 1;", [])
                .context("Failed to set user_version to 1")?;

            tx.commit()
                .context("Failed to commit migration transaction")?;
            tracing::info!("Database migration completed successfully");
        }

        Ok(())
    }

    /// Creates a new session in the database and returns its ID.
    pub fn create_session(&self, started_at: DateTime<Utc>) -> Result<i64> {
        let now_str = Utc::now().to_rfc3339();
        let started_str = started_at.to_rfc3339();

        self.conn
            .execute(
                "INSERT INTO sessions (started_at, created_at) VALUES (?1, ?2)",
                params![started_str, now_str],
            )
            .context("Failed to insert session into database")?;

        let id = self.conn.last_insert_rowid();
        tracing::debug!("Created session id: {}", id);
        Ok(id)
    }

    /// Ends an active session by updating its ended_at column.
    pub fn end_session(&self, session_id: i64, ended_at: DateTime<Utc>) -> Result<()> {
        let ended_str = ended_at.to_rfc3339();

        self.conn
            .execute(
                "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
                params![ended_str, session_id],
            )
            .context("Failed to update session end time in database")?;

        tracing::debug!("Ended session id: {}", session_id);
        Ok(())
    }

    /// Logs an event (START, PAUSE, RESUME, STOP, AUTO_PAUSE) associated with a session.
    pub fn log_event(
        &self,
        session_id: i64,
        event_type: &str,
        created_at: DateTime<Utc>,
    ) -> Result<()> {
        let created_str = created_at.to_rfc3339();

        self.conn
            .execute(
                "INSERT INTO events (session_id, event_type, created_at) VALUES (?1, ?2, ?3)",
                params![session_id, event_type, created_str],
            )
            .context("Failed to insert event into database")?;

        tracing::debug!("Logged event '{}' for session: {}", event_type, session_id);
        Ok(())
    }

    /// Retrieves the last session from the database, if any.
    pub fn get_last_session(&self) -> Result<Option<DbSession>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, started_at, ended_at FROM sessions ORDER BY id DESC LIMIT 1")
            .context("Failed to prepare statement")?;

        let mut rows = stmt.query([]).context("Failed to query sessions")?;

        if let Some(row) = rows.next().context("Failed to iterate rows")? {
            let id: i64 = row.get(0)?;
            let started_str: String = row.get(1)?;
            let ended_str: Option<String> = row.get(2)?;

            let started_at = DateTime::parse_from_rfc3339(&started_str)
                .context("Failed to parse started_at ISO string")?
                .with_timezone(&Utc);

            let ended_at = match ended_str {
                Some(s) => Some(
                    DateTime::parse_from_rfc3339(&s)
                        .context("Failed to parse ended_at ISO string")?
                        .with_timezone(&Utc),
                ),
                None => None,
            };

            Ok(Some(DbSession {
                id,
                started_at,
                ended_at,
            }))
        } else {
            Ok(None)
        }
    }

    /// Retrieves the last event type for a given session.
    pub fn get_last_event_type(&self, session_id: i64) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT event_type FROM events WHERE session_id = ?1 ORDER BY id DESC LIMIT 1")
            .context("Failed to prepare statement")?;

        let mut rows = stmt
            .query(params![session_id])
            .context("Failed to query events")?;
        if let Some(row) = rows.next().context("Failed to iterate rows")? {
            let event_type: String = row.get(0)?;
            Ok(Some(event_type))
        } else {
            Ok(None)
        }
    }

    /// Computes the elapsed active duration of a session by processing its event history.
    #[allow(dead_code)]
    pub fn get_session_elapsed_time(&self, session_id: i64) -> Result<std::time::Duration> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT event_type, created_at FROM events WHERE session_id = ?1 ORDER BY id ASC",
            )
            .context("Failed to prepare statement")?;

        let mut rows = stmt
            .query(params![session_id])
            .context("Failed to query events for elapsed time")?;

        let mut total_duration = std::time::Duration::ZERO;
        let mut active_start: Option<DateTime<Utc>> = None;

        while let Some(row) = rows.next().context("Failed to iterate events")? {
            let event_type: String = row.get(0)?;
            let created_str: String = row.get(1)?;
            let created_at = DateTime::parse_from_rfc3339(&created_str)
                .context("Failed to parse event created_at")?
                .with_timezone(&Utc);

            match event_type.as_str() {
                "START" | "RESUME" => {
                    active_start = Some(created_at);
                }
                "PAUSE" | "AUTO_PAUSE" | "STOP" => {
                    if let Some(start) = active_start
                        && let Ok(duration) = (created_at - start).to_std()
                    {
                        total_duration += duration;
                    }
                    active_start = None;
                }
                _ => {}
            }
        }

        // If the session is currently active/running, add the time since the last active start
        if let Some(start) = active_start {
            let now = Utc::now();
            if now > start
                && let Ok(duration) = (now - start).to_std()
            {
                total_duration += duration;
            }
        }

        Ok(total_duration)
    }

    /// Computes the total elapsed active duration for the current local calendar day.
    pub fn get_today_elapsed_time(&self) -> Result<std::time::Duration> {
        let local_now = chrono::Local::now();
        let today_start_local = local_now.date_naive().and_hms_opt(0, 0, 0).unwrap();
        let today_start_utc = chrono::Local
            .from_local_datetime(&today_start_local)
            .single()
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|| {
                local_now.with_timezone(&Utc) - chrono::Duration::try_hours(24).unwrap()
            });

        // Query all sessions that could possibly have been active today.
        // That is, sessions that have ended after today_start_utc, or are still active (ended_at IS NULL).
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM sessions WHERE ended_at IS NULL OR ended_at >= ?1")
            .context("Failed to prepare sessions statement")?;

        let mut rows = stmt
            .query(params![today_start_utc.to_rfc3339()])
            .context("Failed to query sessions for today's elapsed time")?;

        let mut total_duration = std::time::Duration::ZERO;
        let now_utc = Utc::now();

        while let Some(row) = rows.next().context("Failed to iterate sessions")? {
            let session_id: i64 = row.get(0)?;

            // Get all events for this session
            let mut event_stmt = self
                .conn
                .prepare(
                    "SELECT event_type, created_at FROM events WHERE session_id = ?1 ORDER BY id ASC"
                )
                .context("Failed to prepare events statement")?;

            let mut event_rows = event_stmt
                .query(params![session_id])
                .context("Failed to query events for session")?;

            let mut active_start: Option<DateTime<Utc>> = None;

            while let Some(event_row) = event_rows.next().context("Failed to iterate events")? {
                let event_type: String = event_row.get(0)?;
                let created_str: String = event_row.get(1)?;
                let created_at = DateTime::parse_from_rfc3339(&created_str)
                    .context("Failed to parse event created_at")?
                    .with_timezone(&Utc);

                match event_type.as_str() {
                    "START" | "RESUME" => {
                        active_start = Some(created_at);
                    }
                    "PAUSE" | "AUTO_PAUSE" | "STOP" => {
                        if let Some(start) = active_start {
                            total_duration +=
                                overlap_duration(start, created_at, today_start_utc, now_utc);
                        }
                        active_start = None;
                    }
                    _ => {}
                }
            }

            // If session is still active
            if let Some(start) = active_start {
                total_duration += overlap_duration(start, now_utc, today_start_utc, now_utc);
            }
        }

        Ok(total_duration)
    }
}

fn overlap_duration(
    a: DateTime<Utc>,
    b: DateTime<Utc>,
    x: DateTime<Utc>,
    y: DateTime<Utc>,
) -> std::time::Duration {
    let start = a.max(x);
    let end = b.min(y);
    if start < end {
        (end - start).to_std().unwrap_or(std::time::Duration::ZERO)
    } else {
        std::time::Duration::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_session_elapsed_time() -> Result<()> {
        let db = Database::in_memory()?;
        let session_id = db.create_session(Utc::now())?;

        let start_time = Utc::now();
        db.log_event(session_id, "START", start_time)?;

        // Simulate 10 seconds of active work
        let pause_time = start_time + chrono::Duration::try_seconds(10).unwrap();
        db.log_event(session_id, "PAUSE", pause_time)?;

        // Simulated inactive time (5 seconds)
        let resume_time = pause_time + chrono::Duration::try_seconds(5).unwrap();
        db.log_event(session_id, "RESUME", resume_time)?;

        // Simulate 15 more seconds of active work
        let stop_time = resume_time + chrono::Duration::try_seconds(15).unwrap();
        db.log_event(session_id, "STOP", stop_time)?;
        db.end_session(session_id, stop_time)?;

        let elapsed = db.get_session_elapsed_time(session_id)?;
        assert_eq!(elapsed, Duration::from_secs(25)); // 10s + 15s

        Ok(())
    }

    #[test]
    fn test_today_elapsed_time() -> Result<()> {
        let db = Database::in_memory()?;

        // Get local time start of today, convert to UTC
        let local_now = chrono::Local::now();
        let today_start_local = local_now.date_naive().and_hms_opt(0, 0, 0).unwrap();
        let today_start_utc = chrono::Local
            .from_local_datetime(&today_start_local)
            .single()
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap()
            .with_timezone(&Utc);

        // 1. A session yesterday (ended before today)
        let yesterday_session_id =
            db.create_session(today_start_utc - chrono::Duration::try_hours(2).unwrap())?;
        db.log_event(
            yesterday_session_id,
            "START",
            today_start_utc - chrono::Duration::try_hours(2).unwrap(),
        )?;
        db.log_event(
            yesterday_session_id,
            "STOP",
            today_start_utc - chrono::Duration::try_hours(1).unwrap(),
        )?;
        db.end_session(
            yesterday_session_id,
            today_start_utc - chrono::Duration::try_hours(1).unwrap(),
        )?;

        // 2. A session spanning midnight (started yesterday, ended today)
        let span_session_id =
            db.create_session(today_start_utc - chrono::Duration::try_minutes(30).unwrap())?;
        db.log_event(
            span_session_id,
            "START",
            today_start_utc - chrono::Duration::try_minutes(30).unwrap(),
        )?;
        db.log_event(
            span_session_id,
            "STOP",
            today_start_utc + chrono::Duration::try_minutes(15).unwrap(),
        )?;
        db.end_session(
            span_session_id,
            today_start_utc + chrono::Duration::try_minutes(15).unwrap(),
        )?;
        // Expected time today for this session: 15 minutes = 900 seconds.

        // 3. A session today that ended today
        let session_today =
            db.create_session(today_start_utc + chrono::Duration::try_hours(1).unwrap())?;
        db.log_event(
            session_today,
            "START",
            today_start_utc + chrono::Duration::try_hours(1).unwrap(),
        )?;
        db.log_event(
            session_today,
            "PAUSE",
            today_start_utc
                + chrono::Duration::try_hours(1).unwrap()
                + chrono::Duration::try_minutes(10).unwrap(),
        )?;
        db.log_event(
            session_today,
            "RESUME",
            today_start_utc
                + chrono::Duration::try_hours(1).unwrap()
                + chrono::Duration::try_minutes(20).unwrap(),
        )?;
        db.log_event(
            session_today,
            "STOP",
            today_start_utc
                + chrono::Duration::try_hours(1).unwrap()
                + chrono::Duration::try_minutes(35).unwrap(),
        )?;
        db.end_session(
            session_today,
            today_start_utc
                + chrono::Duration::try_hours(1).unwrap()
                + chrono::Duration::try_minutes(35).unwrap(),
        )?;
        // Expected time today: 10 mins (start to pause) + 15 mins (resume to stop) = 25 minutes = 1500 seconds.

        // Total expected duration so far today: 900s + 1500s = 2400s (40 minutes)
        let elapsed = db.get_today_elapsed_time()?;
        assert_eq!(elapsed, Duration::from_secs(2400));

        // 4. An ongoing session today
        let now = Utc::now();
        let ongoing_start = now.max(today_start_utc + chrono::Duration::try_minutes(1).unwrap())
            - chrono::Duration::try_minutes(5).unwrap();

        let ongoing_session = db.create_session(ongoing_start)?;
        db.log_event(ongoing_session, "START", ongoing_start)?;

        let elapsed_after_start = db.get_today_elapsed_time()?;
        let expected_min = Duration::from_secs(2400) + (now - ongoing_start).to_std().unwrap();

        let diff = elapsed_after_start.abs_diff(expected_min);
        assert!(diff < Duration::from_secs(2));

        Ok(())
    }
}
