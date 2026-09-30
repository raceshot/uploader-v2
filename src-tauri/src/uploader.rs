use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::{AppError, Result};

const API_BASE: &str = "https://api.raceshot.app";

fn default_auto_mode() -> bool {
    true
}

pub fn photographer_upload_url() -> String {
    format!("{}/api/v1/photographer/upload", API_BASE)
}

pub fn host_upload_url() -> String {
    format!("{}/api/v1/host/albums/upload/photo", API_BASE)
}

pub fn photographer_featured_url(photo_id: &str) -> String {
    format!(
        "{}/api/v1/photographer/photos/{}/featured",
        API_BASE,
        urlencoding::encode(photo_id)
    )
}

pub fn host_photo_url(event_id: &str, photo_id: &str) -> String {
    format!(
        "{}/api/v1/host/albums/{}/photos/{}",
        API_BASE,
        urlencoding::encode(strip_org_prefix(event_id).as_str()),
        urlencoding::encode(photo_id)
    )
}

pub fn host_photos_url(event_id: &str, page: usize, limit: usize) -> String {
    format!(
        "{}/api/v1/host/albums/{}/photos?page={}&limit={}&featured_only=true",
        API_BASE,
        urlencoding::encode(strip_org_prefix(event_id).as_str()),
        page,
        limit
    )
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
    #[serde(default = "default_auto_mode")]
    pub auto_mode: bool,
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

pub async fn list_featured_photo_ids(
    client: &Client,
    token: &str,
    event_id: &str,
    timeout_secs: u64,
) -> Result<Vec<String>> {
    let timeout = std::time::Duration::from_secs(timeout_secs.max(10));

    if event_id.starts_with("org_") {
        let mut page = 1usize;
        let limit = 100usize;
        let mut photo_ids = Vec::new();

        loop {
            let resp = client
                .get(host_photos_url(event_id, page, limit))
                .bearer_auth(token)
                .timeout(timeout)
                .send()
                .await?;
            let status = resp.status();
            let payload: serde_json::Value = resp.json().await?;

            if !status.is_success() {
                return Err(AppError::Other(api_error_message(&payload, "取得主辦相簿精選失敗")));
            }

            let rows = payload
                .get("photos")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let row_count = rows.len();
            photo_ids.extend(rows.into_iter().filter_map(|photo| {
                photo
                    .get("photo_id")
                    .and_then(|value| value.as_str())
                    .map(String::from)
            }));

            let total = payload
                .get("total")
                .and_then(|value| value.as_u64())
                .unwrap_or(photo_ids.len() as u64);
            if row_count == 0 || photo_ids.len() as u64 >= total {
                break;
            }
            page += 1;
        }

        return Ok(photo_ids);
    }

    let resp = client
        .get(format!(
            "{}/api/v1/photographer/photos/featured?event_id={}",
            API_BASE,
            urlencoding::encode(event_id)
        ))
        .bearer_auth(token)
        .timeout(timeout)
        .send()
        .await?;
    let status = resp.status();
    let payload: serde_json::Value = resp.json().await?;

    if !status.is_success() {
        return Err(AppError::Other(api_error_message(&payload, "取得攝影師精選失敗")));
    }

    Ok(payload
        .get("photos")
        .and_then(|value| value.as_array())
        .map(|photos| {
            photos
                .iter()
                .filter_map(|photo| {
                    photo
                        .get("photo_id")
                        .and_then(|value| value.as_str())
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default())
}

pub async fn set_photo_featured(
    client: &Client,
    token: &str,
    event_id: &str,
    photo_id: &str,
    is_featured: bool,
    timeout_secs: u64,
) -> Result<()> {
    let (url, body) = if event_id.starts_with("org_") {
        (
            host_photo_url(event_id, photo_id),
            serde_json::json!({ "is_featured": is_featured }),
        )
    } else {
        (
            photographer_featured_url(photo_id),
            serde_json::json!({ "event_id": event_id, "is_featured": is_featured }),
        )
    };

    let resp = client
        .put(url)
        .bearer_auth(token)
        .timeout(std::time::Duration::from_secs(timeout_secs.max(10)))
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let payload: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);

    if !status.is_success() {
        return Err(AppError::Other(api_error_message(&payload, "更新精選狀態失敗")));
    }

    Ok(())
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
        .or_else(|| payload.get("photoId").and_then(|v| v.as_str()))
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

fn api_error_message(payload: &serde_json::Value, fallback: &str) -> String {
    payload
        .get("error")
        .or_else(|| payload.get("message"))
        .or_else(|| payload.get("details"))
        .and_then(|value| value.as_str())
        .unwrap_or(fallback)
        .to_string()
}

pub fn choose_endpoint(event_id: &str) -> String {
    if event_id.starts_with("org_") {
        host_upload_url()
    } else {
        photographer_upload_url()
    }
}

#[cfg(test)]
mod tests {
    use super::{choose_endpoint, parse_upload_response, photographer_upload_url};

    #[test]
    fn upload_response_accepts_photographer_camel_case_photo_id() {
        let result = parse_upload_response(
            &serde_json::json!({ "success": true, "photoId": "photo-1" }),
            "photo.jpg",
            "/photos/photo.jpg",
            200,
        )
        .expect("成功回應應該可解析");

        assert_eq!(result.photo_id.as_deref(), Some("photo-1"));
    }

    #[test]
    fn organization_event_uses_host_upload_endpoint() {
        assert_ne!(choose_endpoint("org_123"), photographer_upload_url());
        assert!(choose_endpoint("org_123").ends_with("/api/v1/host/albums/upload/photo"));
    }
}
