use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;
use velox_segments::PersistedSegment;
use velox_types::{
    Category, ChecksumAlgorithm, ChecksumSpec, ConflictPolicy, DownloadId, DownloadKind,
    DownloadStatus, ErrorKind, HeaderPair, MediaRequest,
};

use crate::{from_i64, to_i64, Database, DbError, DbResult};

/// A download as stored in the database.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadRecord {
    pub id: DownloadId,
    pub url: String,
    pub final_url: Option<String>,
    pub page_url: Option<String>,
    pub file_name: String,
    pub save_dir: String,
    /// Path of the partial file while downloading.
    pub temp_path: Option<String>,
    pub kind: DownloadKind,
    pub status: DownloadStatus,
    pub category: Category,
    pub total_size: Option<u64>,
    pub downloaded: u64,
    pub resumable: Option<bool>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub mime: Option<String>,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    /// Non-sensitive custom headers. Sensitive headers live in secrets.
    pub headers: Vec<HeaderPair>,
    pub max_connections: u8,
    pub queue_id: Option<String>,
    pub priority: i32,
    pub position: i64,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub scheduled_at: Option<i64>,
    pub error: Option<String>,
    pub error_kind: Option<ErrorKind>,
    pub retry_count: u32,
    pub speed_limit: u64,
    pub checksum: Option<ChecksumSpec>,
    pub checksum_ok: Option<bool>,
    pub media: Option<MediaRequest>,
    pub elapsed_ms: u64,
    /// Id of the encrypted secrets row (cookies, credentials).
    pub secret_id: Option<String>,
    pub conflict: ConflictPolicy,
    /// The user chose the file name explicitly; do not replace it with the
    /// server-provided name.
    pub name_locked: bool,
}

impl DownloadRecord {
    /// A new record with defaults; callers fill in the rest.
    pub fn new(url: impl Into<String>, file_name: impl Into<String>, save_dir: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            url: url.into(),
            final_url: None,
            page_url: None,
            file_name: file_name.into(),
            save_dir: save_dir.into(),
            temp_path: None,
            kind: DownloadKind::Http,
            status: DownloadStatus::Paused,
            category: Category::Other,
            total_size: None,
            downloaded: 0,
            resumable: None,
            etag: None,
            last_modified: None,
            mime: None,
            referer: None,
            user_agent: None,
            headers: Vec::new(),
            max_connections: 8,
            queue_id: None,
            priority: 0,
            position: 0,
            created_at: velox_types::now_ms(),
            started_at: None,
            completed_at: None,
            scheduled_at: None,
            error: None,
            error_kind: None,
            retry_count: 0,
            speed_limit: 0,
            checksum: None,
            checksum_ok: None,
            media: None,
            elapsed_ms: 0,
            secret_id: None,
            conflict: ConflictPolicy::Rename,
            name_locked: false,
        }
    }

    fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        let id: String = r.get("id")?;
        let kind: String = r.get("kind")?;
        let status: String = r.get("status")?;
        let category: String = r.get("category")?;
        let headers_json: String = r.get("headers_json")?;
        let media_json: Option<String> = r.get("media_json")?;
        let checksum_algo: Option<String> = r.get("checksum_algo")?;
        let checksum_value: Option<String> = r.get("checksum_value")?;
        let error_kind: Option<String> = r.get("error_kind")?;
        let conflict: String = r.get("conflict")?;
        let bad = |what: &str| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("invalid {what}").into(),
            )
        };
        Ok(Self {
            id: Uuid::parse_str(&id).map_err(|_| bad("id"))?,
            url: r.get("url")?,
            final_url: r.get("final_url")?,
            page_url: r.get("page_url")?,
            file_name: r.get("file_name")?,
            save_dir: r.get("save_dir")?,
            temp_path: r.get("temp_path")?,
            kind: DownloadKind::parse(&kind).ok_or_else(|| bad("kind"))?,
            status: DownloadStatus::parse(&status).ok_or_else(|| bad("status"))?,
            category: Category::parse(&category),
            total_size: r.get::<_, Option<i64>>("total_size")?.map(from_i64),
            downloaded: from_i64(r.get("downloaded")?),
            resumable: r.get::<_, Option<bool>>("resumable")?,
            etag: r.get("etag")?,
            last_modified: r.get("last_modified")?,
            mime: r.get("mime")?,
            referer: r.get("referer")?,
            user_agent: r.get("user_agent")?,
            headers: serde_json::from_str(&headers_json).unwrap_or_default(),
            max_connections: r.get::<_, i64>("max_connections")?.clamp(1, 32) as u8,
            queue_id: r.get("queue_id")?,
            priority: r.get("priority")?,
            position: r.get("position")?,
            created_at: r.get("created_at")?,
            started_at: r.get("started_at")?,
            completed_at: r.get("completed_at")?,
            scheduled_at: r.get("scheduled_at")?,
            error: r.get("error")?,
            error_kind: error_kind.as_deref().map(ErrorKind::parse),
            retry_count: r.get::<_, i64>("retry_count")?.max(0) as u32,
            speed_limit: from_i64(r.get("speed_limit")?),
            checksum: match (checksum_algo, checksum_value) {
                (Some(a), Some(v)) => ChecksumAlgorithm::parse(&a)
                    .map(|algorithm| ChecksumSpec { algorithm, expected: v }),
                _ => None,
            },
            checksum_ok: r.get("checksum_ok")?,
            media: media_json.and_then(|j| serde_json::from_str(&j).ok()),
            elapsed_ms: from_i64(r.get("elapsed_ms")?),
            secret_id: r.get("secret_id")?,
            conflict: if conflict == "overwrite" { ConflictPolicy::Overwrite } else { ConflictPolicy::Rename },
            name_locked: r.get("name_locked")?,
        })
    }
}

/// A line in a download's activity log.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LogEntry {
    pub ts: i64,
    pub level: String,
    pub message: String,
}

const MAX_LOG_PER_DOWNLOAD: i64 = 300;

fn upsert(conn: &Connection, d: &DownloadRecord) -> DbResult<()> {
    let headers_json = serde_json::to_string(&d.headers)?;
    let media_json = d.media.as_ref().map(serde_json::to_string).transpose()?;
    conn.prepare_cached(
        "INSERT INTO downloads (
            id, url, final_url, page_url, file_name, save_dir, temp_path, kind, status, category,
            total_size, downloaded, resumable, etag, last_modified, mime, referer, user_agent,
            headers_json, max_connections, queue_id, priority, position, created_at, started_at,
            completed_at, scheduled_at, error, error_kind, retry_count, speed_limit, checksum_algo,
            checksum_value, checksum_ok, media_json, elapsed_ms, secret_id, conflict, name_locked)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,
                 ?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33,?34,?35,?36,?37,?38,?39)
         ON CONFLICT(id) DO UPDATE SET
            url=excluded.url, final_url=excluded.final_url, page_url=excluded.page_url,
            file_name=excluded.file_name, save_dir=excluded.save_dir, temp_path=excluded.temp_path,
            kind=excluded.kind, status=excluded.status, category=excluded.category,
            total_size=excluded.total_size, downloaded=excluded.downloaded,
            resumable=excluded.resumable, etag=excluded.etag, last_modified=excluded.last_modified,
            mime=excluded.mime, referer=excluded.referer, user_agent=excluded.user_agent,
            headers_json=excluded.headers_json, max_connections=excluded.max_connections,
            queue_id=excluded.queue_id, priority=excluded.priority, position=excluded.position,
            started_at=excluded.started_at, completed_at=excluded.completed_at,
            scheduled_at=excluded.scheduled_at, error=excluded.error, error_kind=excluded.error_kind,
            retry_count=excluded.retry_count, speed_limit=excluded.speed_limit,
            checksum_algo=excluded.checksum_algo, checksum_value=excluded.checksum_value,
            checksum_ok=excluded.checksum_ok, media_json=excluded.media_json,
            elapsed_ms=excluded.elapsed_ms, secret_id=excluded.secret_id,
            conflict=excluded.conflict, name_locked=excluded.name_locked",
    )?
    .execute(params![
        d.id.to_string(),
        d.url,
        d.final_url,
        d.page_url,
        d.file_name,
        d.save_dir,
        d.temp_path,
        d.kind.as_str(),
        d.status.as_str(),
        d.category.as_str(),
        d.total_size.map(to_i64),
        to_i64(d.downloaded),
        d.resumable,
        d.etag,
        d.last_modified,
        d.mime,
        d.referer,
        d.user_agent,
        headers_json,
        d.max_connections as i64,
        d.queue_id,
        d.priority,
        d.position,
        d.created_at,
        d.started_at,
        d.completed_at,
        d.scheduled_at,
        d.error,
        d.error_kind.map(|k| k.as_str()),
        d.retry_count as i64,
        to_i64(d.speed_limit),
        d.checksum.as_ref().map(|c| c.algorithm.as_str()),
        d.checksum.as_ref().map(|c| c.expected.clone()),
        d.checksum_ok,
        media_json,
        to_i64(d.elapsed_ms),
        d.secret_id,
        match d.conflict {
            ConflictPolicy::Rename => "rename",
            ConflictPolicy::Overwrite => "overwrite",
        },
        d.name_locked,
    ])?;
    Ok(())
}

fn write_segments(conn: &Connection, id: &str, segments: &[PersistedSegment]) -> DbResult<()> {
    conn.prepare_cached("DELETE FROM segments WHERE download_id = ?1")?.execute([id])?;
    let mut stmt = conn.prepare_cached(
        "INSERT INTO segments (download_id, idx, start, end_, written) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for (i, s) in segments.iter().enumerate() {
        stmt.execute(params![id, i as i64, to_i64(s.start), s.end.map(to_i64), to_i64(s.written)])?;
    }
    Ok(())
}

impl Database {
    /// Insert or replace a download record.
    pub fn save_download(&self, d: &DownloadRecord) -> DbResult<()> {
        self.with(|c| upsert(c, d))
    }

    pub fn get_download(&self, id: DownloadId) -> DbResult<Option<DownloadRecord>> {
        self.with(|c| {
            Ok(c.prepare_cached("SELECT * FROM downloads WHERE id = ?1")?
                .query_row([id.to_string()], DownloadRecord::from_row)
                .optional()?)
        })
    }

    /// All downloads, newest first.
    pub fn list_downloads(&self) -> DbResult<Vec<DownloadRecord>> {
        self.with(|c| {
            let mut stmt = c.prepare_cached("SELECT * FROM downloads ORDER BY created_at DESC")?;
            let rows = stmt.query_map([], DownloadRecord::from_row)?;
            let mut out = Vec::new();
            for row in rows {
                match row {
                    Ok(r) => out.push(r),
                    Err(e) => tracing::error!(error = %e, "skipping corrupt download row"),
                }
            }
            Ok(out)
        })
    }

    /// Downloads with the given URL (duplicate detection).
    pub fn find_by_url(&self, url: &str) -> DbResult<Vec<DownloadId>> {
        self.with(|c| {
            let mut stmt =
                c.prepare_cached("SELECT id FROM downloads WHERE url = ?1 OR final_url = ?1")?;
            let ids = stmt
                .query_map([url], |r| r.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .filter_map(|s| Uuid::parse_str(&s).ok())
                .collect();
            Ok(ids)
        })
    }

    /// Delete a download, its segments, log and secrets.
    pub fn delete_download(&self, id: DownloadId) -> DbResult<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            let secret: Option<String> = tx
                .query_row("SELECT secret_id FROM downloads WHERE id = ?1", [id.to_string()], |r| {
                    r.get(0)
                })
                .optional()?
                .flatten();
            if let Some(s) = secret {
                tx.execute("DELETE FROM secrets WHERE id = ?1", [s])?;
            }
            tx.execute("DELETE FROM downloads WHERE id = ?1", [id.to_string()])?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Next queue position (appends to the end of a queue).
    pub fn next_position(&self) -> DbResult<i64> {
        self.with(|c| {
            Ok(c.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM downloads", [], |r| r.get(0))?)
        })
    }

    pub fn load_segments(&self, id: DownloadId) -> DbResult<Vec<PersistedSegment>> {
        self.with(|c| {
            let mut stmt = c.prepare_cached(
                "SELECT start, end_, written FROM segments WHERE download_id = ?1 ORDER BY start",
            )?;
            let rows = stmt.query_map([id.to_string()], |r| {
                Ok(PersistedSegment {
                    start: from_i64(r.get(0)?),
                    end: r.get::<_, Option<i64>>(1)?.map(from_i64),
                    written: from_i64(r.get(2)?),
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// Atomically persist a progress checkpoint: the record (with updated
    /// `downloaded` / `elapsed_ms`) and its segment table.
    ///
    /// Callers must fsync the data file *before* calling this.
    pub fn checkpoint(&self, record: &DownloadRecord, segments: &[PersistedSegment]) -> DbResult<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            upsert(&tx, record)?;
            write_segments(&tx, &record.id.to_string(), segments)?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn clear_segments(&self, id: DownloadId) -> DbResult<()> {
        self.with(|c| {
            c.execute("DELETE FROM segments WHERE download_id = ?1", [id.to_string()])?;
            Ok(())
        })
    }

    /// Append to a download's activity log, trimming old entries.
    pub fn append_log(&self, id: DownloadId, level: &str, message: &str) -> DbResult<()> {
        self.with(|c| {
            let ids = id.to_string();
            c.prepare_cached(
                "INSERT INTO download_log (download_id, ts, level, message) VALUES (?1, ?2, ?3, ?4)",
            )?
            .execute(params![ids, velox_types::now_ms(), level, message])?;
            c.prepare_cached(
                "DELETE FROM download_log WHERE download_id = ?1 AND id <= (
                    SELECT id FROM download_log WHERE download_id = ?1
                    ORDER BY id DESC LIMIT 1 OFFSET ?2)",
            )?
            .execute(params![ids, MAX_LOG_PER_DOWNLOAD])?;
            Ok(())
        })
    }

    pub fn read_log(&self, id: DownloadId) -> DbResult<Vec<LogEntry>> {
        self.with(|c| {
            let mut stmt = c.prepare_cached(
                "SELECT ts, level, message FROM download_log WHERE download_id = ?1 ORDER BY id",
            )?;
            let rows = stmt.query_map([id.to_string()], |r| {
                Ok(LogEntry { ts: r.get(0)?, level: r.get(1)?, message: r.get(2)? })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }
}

impl From<rusqlite::types::FromSqlError> for DbError {
    fn from(e: rusqlite::types::FromSqlError) -> Self {
        DbError::Corrupt(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DownloadRecord {
        let mut r = DownloadRecord::new("https://example.com/f.zip", "f.zip", "/tmp");
        r.total_size = Some(5_000_000_000);
        r.headers = vec![HeaderPair { name: "X-Test".into(), value: "1".into() }];
        r.checksum = Some(ChecksumSpec { algorithm: ChecksumAlgorithm::Sha256, expected: "ab".into() });
        r.error_kind = Some(ErrorKind::Network);
        r.media = Some(MediaRequest { url: "https://e.com/m.m3u8".into(), ..Default::default() });
        r
    }

    #[test]
    fn save_load_roundtrip() {
        let db = Database::open_in_memory().unwrap();
        let mut r = sample();
        db.save_download(&r).unwrap();
        assert_eq!(db.get_download(r.id).unwrap().unwrap(), r);
        r.status = DownloadStatus::Completed;
        r.downloaded = 5_000_000_000;
        db.save_download(&r).unwrap();
        assert_eq!(db.get_download(r.id).unwrap().unwrap(), r);
        assert_eq!(db.list_downloads().unwrap().len(), 1);
        assert_eq!(db.find_by_url("https://example.com/f.zip").unwrap(), vec![r.id]);
    }

    #[test]
    fn checkpoint_replaces_segments_atomically() {
        let db = Database::open_in_memory().unwrap();
        let mut r = sample();
        db.save_download(&r).unwrap();
        let segs = vec![
            PersistedSegment { start: 0, end: Some(100), written: 50 },
            PersistedSegment { start: 100, end: Some(5_000_000_000), written: 4_000_000_000 },
        ];
        r.downloaded = 50 + 4_000_000_000 - 100;
        db.checkpoint(&r, &segs).unwrap();
        assert_eq!(db.load_segments(r.id).unwrap(), segs);
        db.checkpoint(&r, &segs[..1]).unwrap();
        assert_eq!(db.load_segments(r.id).unwrap().len(), 1);
        db.delete_download(r.id).unwrap();
        assert!(db.load_segments(r.id).unwrap().is_empty());
        assert!(db.get_download(r.id).unwrap().is_none());
    }

    #[test]
    fn log_is_trimmed() {
        let db = Database::open_in_memory().unwrap();
        let r = sample();
        db.save_download(&r).unwrap();
        for i in 0..(MAX_LOG_PER_DOWNLOAD + 25) {
            db.append_log(r.id, "info", &format!("line {i}")).unwrap();
        }
        let log = db.read_log(r.id).unwrap();
        assert_eq!(log.len() as i64, MAX_LOG_PER_DOWNLOAD);
        assert_eq!(log.last().unwrap().message, format!("line {}", MAX_LOG_PER_DOWNLOAD + 24));
    }
}
