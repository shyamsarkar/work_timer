use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
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
                        && let Ok(duration) = (created_at - start).to_std() {
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
                && let Ok(duration) = (now - start).to_std() {
                    total_duration += duration;
                }
        }

        Ok(total_duration)
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
}
