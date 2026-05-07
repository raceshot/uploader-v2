use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const ALLOWED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

#[derive(Debug, Serialize, Deserialize)]
pub struct ScannedFile {
    pub abs_path: String,
    pub file_name: String,
    pub file_size: i64,
    pub mtime_ns: i64,
}

pub fn scan_folder(dir: &Path) -> Vec<ScannedFile> {
    let mut files = Vec::new();

    for entry in WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        // 忽略隱藏檔案
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with('.'))
            .unwrap_or(false)
        {
            continue;
        }

        if !path.is_file() {
            continue;
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        if !ext.as_deref().map(|e| ALLOWED_EXTENSIONS.contains(&e)).unwrap_or(false) {
            continue;
        }

        if let Ok(sf) = ScannedFile::from_path(path) {
            files.push(sf);
        }
    }

    files.sort_by(|a, b| a.abs_path.cmp(&b.abs_path));
    files
}

impl ScannedFile {
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        #[cfg(windows)]
        use std::os::windows::fs::MetadataExt;

        let meta = std::fs::metadata(path)?;

        #[cfg(unix)]
        let mtime_ns = meta.mtime_nsec();

        #[cfg(windows)]
        let mtime_ns = {
            // Windows FILETIME: 100-nanosecond intervals since 1601-01-01
            // Convert to nanoseconds since Unix epoch (approximate)
            let ft = meta.last_write_time();
            (ft as i64 - 116_444_736_000_000_000) * 100
        };

        #[cfg(not(any(unix, windows)))]
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);

        Ok(Self {
            abs_path: path.to_string_lossy().into_owned(),
            file_name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            file_size: meta.len() as i64,
            mtime_ns,
        })
    }

    pub fn path(&self) -> PathBuf {
        PathBuf::from(&self.abs_path)
    }
}
