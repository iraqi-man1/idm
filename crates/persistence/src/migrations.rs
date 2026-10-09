//! Schema migrations, tracked with `PRAGMA user_version`.
//!
//! Migrations are append-only: never edit a released migration, add a new
//! one instead. Each migration runs inside its own transaction together with
//! the `user_version` bump, so a crash mid-upgrade leaves the previous schema
//! intact.

use rusqlite::Connection;

use crate::DbError;

pub(crate) const MIGRATIONS: &[&str] = &[
    // 1: initial schema
    r#"
    CREATE TABLE downloads (
        id              TEXT PRIMARY KEY NOT NULL,
        url             TEXT NOT NULL,
        final_url       TEXT,
        page_url        TEXT,
        file_name       TEXT NOT NULL,
        save_dir        TEXT NOT NULL,
        temp_path       TEXT,
        kind            TEXT NOT NULL,
        status          TEXT NOT NULL,
        category        TEXT NOT NULL,
        total_size      INTEGER,
        downloaded      INTEGER NOT NULL DEFAULT 0,
        resumable       INTEGER,
        etag            TEXT,
        last_modified   TEXT,
        mime            TEXT,
        referer         TEXT,
        user_agent      TEXT,
        headers_json    TEXT NOT NULL DEFAULT '[]',
        max_connections INTEGER NOT NULL DEFAULT 8,
        queue_id        TEXT,
        priority        INTEGER NOT NULL DEFAULT 0,
        position        INTEGER NOT NULL DEFAULT 0,
        created_at      INTEGER NOT NULL,
        started_at      INTEGER,
        completed_at    INTEGER,
        scheduled_at    INTEGER,
        error           TEXT,
        error_kind      TEXT,
        retry_count     INTEGER NOT NULL DEFAULT 0,
        speed_limit     INTEGER NOT NULL DEFAULT 0,
        checksum_algo   TEXT,
        checksum_value  TEXT,
        checksum_ok     INTEGER,
        media_json      TEXT,
        elapsed_ms      INTEGER NOT NULL DEFAULT 0,
        secret_id       TEXT,
        conflict        TEXT NOT NULL DEFAULT 'rename',
        name_locked     INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX idx_downloads_status ON downloads(status);
    CREATE INDEX idx_downloads_queue ON downloads(queue_id, position);
    CREATE INDEX idx_downloads_url ON downloads(url);

    CREATE TABLE segments (
        download_id TEXT NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
        idx         INTEGER NOT NULL,
        start       INTEGER NOT NULL,
        end_        INTEGER,
        written     INTEGER NOT NULL,
        PRIMARY KEY (download_id, idx)
    );

    CREATE TABLE queues (
        id             TEXT PRIMARY KEY NOT NULL,
        name           TEXT NOT NULL,
        max_concurrent INTEGER NOT NULL DEFAULT 2,
        running        INTEGER NOT NULL DEFAULT 0,
        schedule_json  TEXT NOT NULL DEFAULT '{}',
        post_action    TEXT NOT NULL DEFAULT 'none',
        retry_failed   INTEGER NOT NULL DEFAULT 0,
        sort_order     INTEGER NOT NULL DEFAULT 0,
        built_in       INTEGER NOT NULL DEFAULT 0
    );
    INSERT INTO queues (id, name, max_concurrent, running, built_in, sort_order)
        VALUES ('main', 'Main queue', 3, 1, 1, 0);

    CREATE TABLE settings (
        key   TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL
    );

    CREATE TABLE secrets (
        id         TEXT PRIMARY KEY NOT NULL,
        nonce      BLOB NOT NULL,
        ciphertext BLOB NOT NULL,
        created_at INTEGER NOT NULL
    );

    CREATE TABLE stats_daily (
        day    TEXT PRIMARY KEY NOT NULL,
        bytes  INTEGER NOT NULL DEFAULT 0,
        files  INTEGER NOT NULL DEFAULT 0,
        failed INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE download_log (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        download_id TEXT NOT NULL REFERENCES downloads(id) ON DELETE CASCADE,
        ts          INTEGER NOT NULL,
        level       TEXT NOT NULL,
        message     TEXT NOT NULL
    );
    CREATE INDEX idx_log_download ON download_log(download_id, id);
    "#,
];

pub(crate) fn run(conn: &mut Connection) -> Result<u32, DbError> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let target = MIGRATIONS.len() as u32;
    if current > target {
        return Err(DbError::SchemaTooNew {
            found: current,
            supported: target,
        });
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let version = i as u32 + 1;
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
        tracing::info!(version, "applied database migration");
    }
    Ok(target)
}
