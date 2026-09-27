use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::Result;

fn db_path() -> PathBuf {
    let base = dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".raceshot_uploader");
    std::fs::create_dir_all(&base).ok();
    base.join("history.db")
}

pub fn open_db() -> Result<Connection> {
    let conn = Connection::open(db_path())?;
    initialize_schema(&conn)?;
    Ok(conn)
}

fn initialize_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch("
        PRAGMA journal_mode=WAL;
        PRAGMA synchronous=NORMAL;
        CREATE TABLE IF NOT EXISTS uploads (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id    TEXT NOT NULL,
            photo_id    TEXT,
            abs_path    TEXT NOT NULL,
            file_size   INTEGER NOT NULL,
            mtime_ns    INTEGER NOT NULL,
            sha256_head TEXT NOT NULL,
            uploaded_at TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_event_hash
            ON uploads(event_id, sha256_head);
        CREATE INDEX IF NOT EXISTS idx_event_path
            ON uploads(event_id, abs_path);
        CREATE TABLE IF NOT EXISTS failed_uploads (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id       TEXT NOT NULL,
            abs_path       TEXT NOT NULL,
            file_size      INTEGER NOT NULL,
            mtime_ns       INTEGER NOT NULL,
            sha256_head    TEXT NOT NULL,
            location       TEXT NOT NULL,
            longitude      REAL,
            latitude       REAL,
            last_error     TEXT NOT NULL,
            attempts       INTEGER NOT NULL DEFAULT 1,
            last_failed_at TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_failed_event_path
            ON failed_uploads(event_id, abs_path);
        CREATE INDEX IF NOT EXISTS idx_failed_event
            ON failed_uploads(event_id);
    ")?;
    Ok(())
}

#[derive(Debug, Clone)]
pub enum DupCheck {
    Duplicate { photo_id: Option<String> },
    New,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedUpload {
    pub id: i64,
    pub event_id: String,
    pub abs_path: String,
    pub file_size: i64,
    pub mtime_ns: i64,
    pub sha256_head: String,
    pub location: String,
    pub longitude: Option<f64>,
    pub latitude: Option<f64>,
    pub last_error: String,
    pub attempts: i64,
    pub last_failed_at: String,
}

/// 三層去重檢查：
/// 1. path + size + mtime 完全吻合 → 最快路徑
/// 2. sha256_head 吻合（已改名/移動）→ 更新路徑並跳過
/// 3. 都不符合 → 需要上傳
pub fn check_duplicate(
    conn: &Connection,
    event_id: &str,
    abs_path: &str,
    file_size: i64,
    mtime_ns: i64,
    sha256_head: &str,
) -> Result<DupCheck> {
    // Level 1: path + size + mtime
    let mut stmt = conn.prepare_cached(
        "SELECT photo_id FROM uploads
         WHERE event_id=? AND abs_path=? AND file_size=? AND mtime_ns=?
         LIMIT 1"
    )?;
    let result = stmt.query_row(
        params![event_id, abs_path, file_size, mtime_ns],
        |row| row.get::<_, Option<String>>(0),
    );
    if let Ok(photo_id) = result {
        return Ok(DupCheck::Duplicate { photo_id });
    }

    // Level 2: content hash
    let mut stmt2 = conn.prepare_cached(
        "SELECT photo_id FROM uploads
         WHERE event_id=? AND sha256_head=?
         LIMIT 1"
    )?;
    let result2 = stmt2.query_row(
        params![event_id, sha256_head],
        |row| row.get::<_, Option<String>>(0),
    );
    if let Ok(photo_id) = result2 {
        // 路徑變了但內容相同，更新路徑紀錄
        conn.execute(
            "UPDATE uploads SET abs_path=?, file_size=?, mtime_ns=?
             WHERE event_id=? AND sha256_head=?",
            params![abs_path, file_size, mtime_ns, event_id, sha256_head],
        )?;
        return Ok(DupCheck::Duplicate { photo_id });
    }

    Ok(DupCheck::New)
}

pub fn record_upload(
    conn: &Connection,
    event_id: &str,
    photo_id: Option<&str>,
    abs_path: &str,
    file_size: i64,
    mtime_ns: i64,
    sha256_head: &str,
) -> Result<()> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    conn.execute(
        "INSERT OR IGNORE INTO uploads
            (event_id, photo_id, abs_path, file_size, mtime_ns, sha256_head, uploaded_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        params![event_id, photo_id, abs_path, file_size, mtime_ns, sha256_head, now],
    )?;
    remove_failed_upload(conn, event_id, abs_path)?;
    Ok(())
}

pub fn clear_event_history(conn: &Connection, event_id: &str) -> Result<usize> {
    let uploads = conn.execute(
        "DELETE FROM uploads WHERE event_id=?",
        params![event_id],
    )?;
    let failed = conn.execute(
        "DELETE FROM failed_uploads WHERE event_id=?",
        params![event_id],
    )?;
    Ok(uploads + failed)
}

pub fn record_failed_upload(
    conn: &Connection,
    event_id: &str,
    abs_path: &str,
    file_size: i64,
    mtime_ns: i64,
    sha256_head: &str,
    location: &str,
    longitude: Option<f64>,
    latitude: Option<f64>,
    last_error: &str,
) -> Result<()> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    conn.execute(
        "INSERT INTO failed_uploads
            (event_id, abs_path, file_size, mtime_ns, sha256_head, location,
             longitude, latitude, last_error, attempts, last_failed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?)
         ON CONFLICT(event_id, abs_path) DO UPDATE SET
            file_size=excluded.file_size,
            mtime_ns=excluded.mtime_ns,
            sha256_head=excluded.sha256_head,
            location=excluded.location,
            longitude=excluded.longitude,
            latitude=excluded.latitude,
            last_error=excluded.last_error,
            attempts=failed_uploads.attempts + 1,
            last_failed_at=excluded.last_failed_at",
        params![
            event_id,
            abs_path,
            file_size,
            mtime_ns,
            sha256_head,
            location,
            longitude,
            latitude,
            last_error,
            now,
        ],
    )?;
    Ok(())
}

pub fn remove_failed_upload(conn: &Connection, event_id: &str, abs_path: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM failed_uploads WHERE event_id=? AND abs_path=?",
        params![event_id, abs_path],
    )?;
    Ok(())
}

pub fn list_failed_uploads(conn: &Connection, event_id: &str) -> Result<Vec<FailedUpload>> {
    let mut stmt = conn.prepare(
        "SELECT id, event_id, abs_path, file_size, mtime_ns, sha256_head,
                location, longitude, latitude, last_error, attempts, last_failed_at
         FROM failed_uploads
         WHERE event_id=?
         ORDER BY id",
    )?;
    let rows = stmt.query_map(params![event_id], |row| {
        Ok(FailedUpload {
            id: row.get(0)?,
            event_id: row.get(1)?,
            abs_path: row.get(2)?,
            file_size: row.get(3)?,
            mtime_ns: row.get(4)?,
            sha256_head: row.get(5)?,
            location: row.get(6)?,
            longitude: row.get(7)?,
            latitude: row.get(8)?,
            last_error: row.get(9)?,
            attempts: row.get(10)?,
            last_failed_at: row.get(11)?,
        })
    })?;

    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn count_failed_uploads(conn: &Connection, event_id: &str) -> Result<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM failed_uploads WHERE event_id=?",
        params![event_id],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

/// 計算檔案前 512KB 的 SHA-256（速度與可靠度的平衡點）
pub fn compute_sha256_head(path: &Path) -> std::io::Result<String> {
    use sha2::{Sha256, Digest};
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 512 * 1024];
    let n = file.read(&mut buf)?;
    hasher.update(&buf[..n]);
    Ok(hex::encode(hasher.finalize()))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileInfo {
    pub abs_path: String,
    pub file_size: i64,
    pub mtime_ns: i64,
}

impl FileInfo {
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        let mtime_ns = meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        Ok(Self {
            abs_path: path.to_string_lossy().into_owned(),
            file_size: meta.len() as i64,
            mtime_ns,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn failed_upload_lifecycle_is_persistent_and_clearable() {
        let conn = Connection::open_in_memory().unwrap();
        initialize_schema(&conn).unwrap();

        record_failed_upload(
            &conn,
            "event-1",
            "/photos/a.jpg",
            100,
            200,
            "hash-a",
            "終點線",
            Some(25.0),
            Some(121.0),
            "timeout",
        )
        .unwrap();
        assert_eq!(count_failed_uploads(&conn, "event-1").unwrap(), 1);

        record_failed_upload(
            &conn,
            "event-1",
            "/photos/a.jpg",
            100,
            200,
            "hash-a",
            "終點線",
            Some(25.0),
            Some(121.0),
            "connection closed",
        )
        .unwrap();
        assert_eq!(list_failed_uploads(&conn, "event-1").unwrap()[0].attempts, 2);

        record_upload(
            &conn,
            "event-1",
            Some("photo-1"),
            "/photos/a.jpg",
            100,
            200,
            "hash-a",
        )
        .unwrap();
        assert_eq!(count_failed_uploads(&conn, "event-1").unwrap(), 0);

        record_failed_upload(
            &conn,
            "event-1",
            "/photos/b.jpg",
            100,
            200,
            "hash-b",
            "終點線",
            Some(25.0),
            Some(121.0),
            "timeout",
        )
        .unwrap();
        assert_eq!(clear_event_history(&conn, "event-1").unwrap(), 2);
        assert_eq!(count_failed_uploads(&conn, "event-1").unwrap(), 0);
    }
}
