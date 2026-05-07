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
    ")?;
    Ok(conn)
}

#[derive(Debug, Clone)]
pub enum DupCheck {
    Duplicate { photo_id: Option<String> },
    New,
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
    Ok(())
}

pub fn clear_event_history(conn: &Connection, event_id: &str) -> Result<usize> {
    let n = conn.execute(
        "DELETE FROM uploads WHERE event_id=?",
        params![event_id],
    )?;
    Ok(n)
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
