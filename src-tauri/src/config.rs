use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub token: String,
    pub folder: String,
    pub event_id: String,
    pub location: String,
    pub longitude: f64,
    pub latitude: f64,
    pub gpx_file: String,
    pub gpx_time_offset: i32,
    pub gpx_fallback_mode: String,
    pub gpx_max_gap: u32,
    pub gpx_preview_count: u32,
    pub concurrency: u32,
    pub timeout: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            token: String::new(),
            folder: String::new(),
            event_id: String::new(),
            location: String::new(),
            longitude: 121.0,
            latitude: 25.0,
            gpx_file: String::new(),
            gpx_time_offset: 0,
            gpx_fallback_mode: "manual".to_string(),
            gpx_max_gap: 300,
            gpx_preview_count: 50,
            concurrency: 20,
            timeout: 120,
        }
    }
}

fn config_path() -> PathBuf {
    let base = dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".raceshot_uploader");
    std::fs::create_dir_all(&base).ok();
    base.join("config.json")
}

pub fn load_config() -> AppConfig {
    let path = config_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(config: &AppConfig) -> crate::error::Result<()> {
    let path = config_path();
    let json = serde_json::to_string_pretty(config)?;
    std::fs::write(path, json)?;
    Ok(())
}
