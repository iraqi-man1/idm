//! SQLite persistence for Velox Download Manager.
//!
//! * WAL journal with `synchronous=FULL`: a committed checkpoint survives
//!   power loss, and an interrupted write never corrupts the database.
//! * Segment progress is written by the engine only *after* the data file
//!   has been fsynced, so persisted progress never runs ahead of the data.
//! * Sensitive values (cookies, passwords, auth headers) are encrypted with
//!   a key held in the OS keychain; see [`secrets`].
//!
//! All methods are synchronous and fast; async callers should run larger
//! operations through `spawn_blocking`.

mod downloads;
mod migrations;
mod queues;
pub mod secrets;
mod settings;
mod stats;

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::Connection;

pub use downloads::{DownloadRecord, LogEntry};
pub use secrets::{DownloadSecrets, SecretBox};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("database schema version {found} is newer than this application supports ({supported}); please update the application")]
    SchemaTooNew { found: u32, supported: u32 },
    #[error("record not found")]
    NotFound,
    #[error("corrupt record: {0}")]
    Corrupt(String),
    #[error("secret storage error: {0}")]
    Secret(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type DbResult<T> = Result<T, DbError>;

/// Handle to the application database. Cheap to clone.
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Open (or create) the database at `path` and apply migrations.
    pub fn open(path: &Path) -> DbResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn, true)
    }

    /// In-memory database for tests.
    pub fn open_in_memory() -> DbResult<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn, false)
    }

    fn init(mut conn: Connection, wal: bool) -> DbResult<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        if wal {
            let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
            if !mode.eq_ignore_ascii_case("wal") {
                tracing::warn!(%mode, "WAL journal mode unavailable");
            }
        }
        conn.execute_batch(
            "PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA temp_store=MEMORY;",
        )?;
        migrations::run(&mut conn)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// Run a closure with exclusive access to the connection.
    pub fn with<T>(&self, f: impl FnOnce(&mut Connection) -> DbResult<T>) -> DbResult<T> {
        let mut conn = self.conn.lock();
        f(&mut conn)
    }

    /// Schema version after migrations.
    pub fn schema_version(&self) -> DbResult<u32> {
        self.with(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))
    }

    /// Verify database integrity (used on startup after an unclean exit).
    pub fn quick_check(&self) -> DbResult<bool> {
        self.with(|c| {
            let res: String = c.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
            Ok(res == "ok")
        })
    }
}

pub(crate) fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

pub(crate) fn from_i64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_and_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db").join("velox.sqlite");
        let db = Database::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), migrations::MIGRATIONS.len() as u32);
        assert!(db.quick_check().unwrap());
        drop(db);
        let db = Database::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), migrations::MIGRATIONS.len() as u32);
    }

    #[test]
    fn refuses_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("velox.sqlite");
        {
            let c = Connection::open(&path).unwrap();
            c.pragma_update(None, "user_version", 999).unwrap();
        }
        assert!(matches!(Database::open(&path), Err(DbError::SchemaTooNew { .. })));
    }
}
