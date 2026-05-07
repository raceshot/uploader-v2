use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("IO 錯誤：{0}")]
    Io(#[from] std::io::Error),

    #[error("資料庫錯誤：{0}")]
    Db(#[from] rusqlite::Error),

    #[error("HTTP 錯誤：{0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON 錯誤：{0}")]
    Json(#[from] serde_json::Error),

    #[error("GPX 解析錯誤：{0}")]
    Gpx(String),

    #[error("{0}")]
    Other(String),
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
