use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::{AppError, Result};

const API_BASE: &str = "https://api.raceshot.app";

pub fn photographer_upload_url() -> String {
    format!("{}/api/v1/photographer/upload", API_BASE)
}

pub fn host_upload_url() -> String {
    format!("{}/api/v1/host/albums/upload/photo", API_BASE)
}

pub fn verify_token_url() -> String {
    format!("{}/api/v1/photographer/api/verify", API_BASE)
}

pub fn list_events_url() -> String {
    format!("{}/api/v1/photographer/api/events", API_BASE)
}

// ── API 回傳型別 ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct VerifyResponse {
    pub valid: Option<bool>,
    pub user: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EventInfo {
    pub id: String,
    pub name: String,
    pub date: String,
}

#[derive(Debug, Deserialize)]
pub struct EventsResponse {
    pub events: Option<Vec<EventInfo>>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserInfo {
    pub role: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
}

// ── 上傳結果 ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UploadResult {
    pub file_name: String,
    pub abs_path: String,
    pub success: bool,
    pub photo_id: Option<String>,
    pub message: String,
    pub error: Option<String>,
    pub status_code: Option<u16>,
}

// ── 上傳參數 ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UploadParams {
    pub token: String,
    pub event_id: String,
    pub location: String,
    pub longitude: Option<f64>,
    pub latitude: Option<f64>,
    pub concurrency: u32,
    pub timeout_secs: u64,
    pub folder: String,
    pub gpx_file: Option<String>,
    pub gpx_time_offset: i32,
    pub gpx_fallback_mode: String,
    pub gpx_max_gap: u32,
    #[serde(default)]
    pub reupload_failures: bool,
}

// ── API 呼叫 ─────────────────────────────────────────────────────────────────

pub async fn verify_token(client: &Client, token: &str) -> Result<(bool, Option<UserInfo>)> {
    let resp = client
        .get(verify_token_url())
        .bearer_auth(token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;

    let payload: VerifyResponse = resp.json().await?;
    if payload.valid.unwrap_or(false) {
        let user = payload.user.and_then(|v| {
            Some(UserInfo {
                role: v.get("role").and_then(|r| r.as_str()).map(String::from),
                name: v.get("name").and_then(|r| r.as_str()).map(String::from),
                email: v.get("email").and_then(|r| r.as_str()).map(String::from),
            })
        });
        Ok((true, user))
    } else {
        Err(AppError::Other(
            payload.error.unwrap_or_else(|| "驗證失敗".to_string()),
        ))
    }
}

pub async fn list_events(client: &Client, token: &str) -> Result<Vec<EventInfo>> {
    let resp = client
        .get(list_events_url())
        .bearer_auth(token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;

    let payload: EventsResponse = resp.json().await?;
    Ok(payload.events.unwrap_or_default())
}

/// 上傳單張圖片，含重試邏輯
pub async fn upload_single(
    client: &Client,
    token: &str,
    file_path: &Path,
    event_id: &str,
    location: &str,
    longitude: Option<f64>,
    latitude: Option<f64>,
    endpoint: &str,
    timeout_secs: u64,
    max_retries: u32,
) -> UploadResult {
    let file_name = file_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let abs_path = file_path.to_string_lossy().into_owned();

    // 讀取檔案
    let bytes = match std::fs::read(file_path) {
        Ok(b) => b,
        Err(e) => {
            return UploadResult {
                file_name,
                abs_path,
                success: false,
                photo_id: None,
                message: "讀取失敗".to_string(),
                error: Some(e.to_string()),
                status_code: None,
            };
        }
    };

    let mime = infer_mime(&file_name);
    let clean_id = strip_org_prefix(event_id);

    let mut last_error = String::new();
    let mut last_status: Option<u16> = None;

    for attempt in 0..=max_retries {
        if attempt > 0 {
            let wait = 1.5f64.powi(attempt as i32 - 1);
            tokio::time::sleep(std::time::Duration::from_secs_f64(wait)).await;
        }

        let mut form = reqwest::multipart::Form::new()
            .text("eventId", clean_id.clone())
            .text("album_id", clean_id.clone())
            .text("location", location.to_string())
            .text("price", "169");

        if let Some(lon) = longitude {
            form = form.text("longitude", lon.to_string());
        }
        if let Some(lat) = latitude {
            form = form.text("latitude", lat.to_string());
        }

        let part = reqwest::multipart::Part::bytes(bytes.clone())
            .file_name(file_name.clone())
            .mime_str(mime)
            .unwrap_or_else(|_| {
                reqwest::multipart::Part::bytes(bytes.clone()).file_name(file_name.clone())
            });
        form = form.part("image", part);

        let send_result = client
            .post(endpoint)
            .bearer_auth(token)
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .multipart(form)
            .send()
            .await;

        match send_result {
            Err(e) => {
                last_error = e.to_string();
                continue; // 網路錯誤重試
            }
            Ok(resp) => {
                last_status = Some(resp.status().as_u16());
                let status = resp.status();

                match resp.json::<serde_json::Value>().await {
                    Ok(payload) => {
                        if let Some(r) = parse_upload_response(&payload, &file_name, &abs_path, status.as_u16()) {
                            return r;
                        }
                        // 5xx / 429 重試
                        last_error = payload
                            .get("error")
                            .or(payload.get("message"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("未知錯誤")
                            .to_string();
                        if !should_retry(status.as_u16()) {
                            break;
                        }
                    }
                    Err(e) => {
                        last_error = format!("HTTP {}: JSON 解析失敗 - {}", status.as_u16(), e);
                        if !should_retry(status.as_u16()) {
                            break;
                        }
                    }
                }
            }
        }
    }

    UploadResult {
        file_name,
        abs_path,
        success: false,
        photo_id: None,
        message: "上傳失敗".to_string(),
        error: Some(last_error),
        status_code: last_status,
    }
}

fn parse_upload_response(
    payload: &serde_json::Value,
    file_name: &str,
    abs_path: &str,
    status: u16,
) -> Option<UploadResult> {
    let success = payload.get("success").and_then(|v| v.as_bool()).unwrap_or(false)
        || payload.get("status").and_then(|v| v.as_str()) == Some("success");

    let message = payload
        .get("message")
        .or(payload.get("error"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let photo_id = payload
        .get("photoIds")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("photo_id").and_then(|v| v.as_str()))
        .map(String::from);

    if success {
        return Some(UploadResult {
            file_name: file_name.to_string(),
            abs_path: abs_path.to_string(),
            success: true,
            photo_id,
            message: if message.is_empty() { "上傳成功".to_string() } else { message },
            error: None,
            status_code: Some(status),
        });
    }

    // 重複上傳視為成功
    if status == 409 || is_duplicate_message(&message) {
        return Some(UploadResult {
            file_name: file_name.to_string(),
            abs_path: abs_path.to_string(),
            success: true,
            photo_id,
            message: "已上傳（視為成功）".to_string(),
            error: None,
            status_code: Some(status),
        });
    }

    // 4xx 非重複且非 429 → 不重試，直接回傳 None 讓外層決定
    None
}

fn is_duplicate_message(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    ["already upload", "duplicate", "已上傳", "已存在"]
        .iter()
        .any(|k| lower.contains(k))
}

fn should_retry(status: u16) -> bool {
    status >= 500 || status == 429
}

fn strip_org_prefix(id: &str) -> String {
    id.strip_prefix("org_").unwrap_or(id).to_string()
}

fn infer_mime(file_name: &str) -> &'static str {
    let lower = file_name.to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    }
}

pub fn choose_endpoint(event_id: &str) -> String {
    if event_id.starts_with("org_") {
        host_upload_url()
    } else {
        photographer_upload_url()
    }
}
