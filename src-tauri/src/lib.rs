mod auth;
mod config;
mod error;
mod gpx;
mod history;
mod scanner;
mod uploader;

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use config::AppConfig;
use error::AppError;
use gpx::GpxContext;
use history::{check_duplicate, compute_sha256_head, open_db, record_upload, DupCheck};
use scanner::scan_folder;
use uploader::{upload_single, verify_token, list_events, choose_endpoint, UploadParams};

// ── App 狀態 ─────────────────────────────────────────────────────────────────

pub struct AppState {
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub upload_running: Arc<AtomicBool>,
    pub http_client: reqwest::Client,
}

// ── 事件 payload ─────────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
struct LogEvent {
    message: String,
    level: String,
}

#[derive(Clone, Serialize)]
struct ProgressEvent {
    current: usize,
    total: usize,
    file_name: String,
}

#[derive(Clone, Serialize)]
struct FinishedEvent {
    success: usize,
    failed: usize,
}

// ── Commands ─────────────────────────────────────────────────────────────────

#[tauri::command]
async fn cmd_get_config() -> AppConfig {
    config::load_config()
}

#[tauri::command]
async fn cmd_save_config(cfg: AppConfig) -> Result<(), AppError> {
    config::save_config(&cfg)
}

#[tauri::command]
async fn cmd_verify_token(
    state: State<'_, AppState>,
    token: String,
) -> Result<uploader::UserInfo, AppError> {
    let (valid, user) = verify_token(&state.http_client, &token).await?;
    if valid {
        user.ok_or(AppError::Other("驗證成功但無使用者資訊".to_string()))
    } else {
        Err(AppError::Other("Token 無效".to_string()))
    }
}

#[tauri::command]
async fn cmd_list_events(
    state: State<'_, AppState>,
    token: String,
) -> Result<Vec<uploader::EventInfo>, AppError> {
    list_events(&state.http_client, &token).await
}

#[tauri::command]
async fn cmd_start_auth_server(app: AppHandle) -> Result<(), AppError> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url("https://raceshot.app/uploader-auth", None::<&str>)
        .map_err(|e| AppError::Other(e.to_string()))?;

    let app_clone = app.clone();
    tokio::spawn(async move {
        let handle = auth::start_auth_server().await;
        if let Ok(Some(token)) = handle.await {
            let _ = app_clone.emit("auth://token", token);
        }
    });

    Ok(())
}

#[derive(Serialize)]
struct ScanSummary {
    total: usize,
    to_upload: usize,
    skipped: usize,
}

#[tauri::command]
async fn cmd_scan_folder(
    state: State<'_, AppState>,
    folder: String,
    event_id: String,
) -> Result<ScanSummary, AppError> {
    let path = std::path::Path::new(&folder);
    let files = scan_folder(path);
    let total = files.len();

    let db = state.db.lock().map_err(|_| AppError::Other("DB 鎖定失敗".to_string()))?;
    let mut skipped = 0usize;

    for f in &files {
        let hash = compute_sha256_head(std::path::Path::new(&f.abs_path)).unwrap_or_default();
        if let Ok(DupCheck::Duplicate { .. }) =
            check_duplicate(&db, &event_id, &f.abs_path, f.file_size, f.mtime_ns, &hash)
        {
            skipped += 1;
        }
    }

    Ok(ScanSummary { total, to_upload: total - skipped, skipped })
}

#[derive(Deserialize)]
struct GpxPreviewParams {
    folder: String,
    gpx_file: String,
    time_offset_secs: i32,
    fallback_lat: Option<f64>,
    fallback_lon: Option<f64>,
    fallback_mode: String,
    max_gap_secs: u32,
    sample_count: usize,
}

#[tauri::command]
async fn cmd_preview_gpx(params: GpxPreviewParams) -> Result<Vec<gpx::GpxPreviewRow>, AppError> {
    let gpx_path = std::path::Path::new(&params.gpx_file);
    let track_points = gpx::parse_gpx(gpx_path)?;

    let ctx = GpxContext {
        track_points,
        time_offset_ms: params.time_offset_secs as i64 * 1000,
        fallback_lat: params.fallback_lat,
        fallback_lon: params.fallback_lon,
        fallback_mode: params.fallback_mode.clone(),
        max_gap_ms: params.max_gap_secs as i64 * 1000,
    };

    let folder = std::path::Path::new(&params.folder);
    let all_files = scan_folder(folder);
    let sample_count = params.sample_count.min(all_files.len());
    let samples = &all_files[..sample_count];

    let rows = samples
        .iter()
        .map(|f| {
            let path = std::path::Path::new(&f.abs_path);
            let ts_ms = gpx::extract_exif_timestamp_ms(path);
            let coord = gpx::match_photo(path, &ctx);

            let capture_time_utc = ts_ms.map(|ms| {
                let secs = ms / 1000;
                chrono::DateTime::from_timestamp(secs, 0)
                    .unwrap_or_default()
                    .format("%Y-%m-%d %H:%M:%S UTC")
                    .to_string()
            });

            let coord_str = match (coord.lat, coord.lon) {
                (Some(lat), Some(lon)) => Some(format!("{:.6}, {:.6}", lat, lon)),
                _ => None,
            };

            gpx::GpxPreviewRow {
                file_name: f.file_name.clone(),
                capture_time_utc,
                source: coord.source,
                coord: coord_str,
                note: coord.note,
            }
        })
        .collect();

    Ok(rows)
}

#[tauri::command]
async fn cmd_start_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    params: UploadParams,
) -> Result<(), AppError> {
    if state.upload_running.swap(true, Ordering::SeqCst) {
        return Err(AppError::Other("上傳已在執行中".to_string()));
    }

    let running = Arc::clone(&state.upload_running);
    let db_arc = Arc::clone(&state.db);
    let client = state.http_client.clone();

    tokio::spawn(async move {
        let result = run_upload(&app, &client, &db_arc, &running, &params).await;
        running.store(false, Ordering::SeqCst);
        if let Err(e) = result {
            let _ = app.emit("upload://log", LogEvent {
                message: format!("上傳錯誤：{e}"),
                level: "error".to_string(),
            });
        }
    });

    Ok(())
}

async fn run_upload(
    app: &AppHandle,
    client: &reqwest::Client,
    db_arc: &Arc<Mutex<rusqlite::Connection>>,
    running: &Arc<AtomicBool>,
    params: &UploadParams,
) -> error::Result<()> {
    let folder = std::path::Path::new(&params.folder);
    let all_files = scan_folder(folder);

    let _ = app.emit("upload://log", LogEvent {
        message: format!("掃描完成：共 {} 張圖片", all_files.len()),
        level: "info".to_string(),
    });

    let gpx_ctx: Option<GpxContext> = if let Some(ref gpx_path) = params.gpx_file {
        match gpx::parse_gpx(std::path::Path::new(gpx_path)) {
            Ok(pts) => Some(GpxContext {
                track_points: pts,
                time_offset_ms: params.gpx_time_offset as i64 * 1000,
                fallback_lat: params.latitude,
                fallback_lon: params.longitude,
                fallback_mode: params.gpx_fallback_mode.clone(),
                max_gap_ms: params.gpx_max_gap as i64 * 1000,
            }),
            Err(e) => {
                let _ = app.emit("upload://log", LogEvent {
                    message: format!("GPX 載入失敗：{e}，將使用手動座標"),
                    level: "warn".to_string(),
                });
                None
            }
        }
    } else {
        None
    };

    let endpoint = choose_endpoint(&params.event_id);
    let total = all_files.len();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(params.concurrency as usize));
    let mut handles: Vec<tokio::task::JoinHandle<bool>> = Vec::new();
    let mut current = 0usize;

    for f in all_files {
        if !running.load(Ordering::Relaxed) {
            let _ = app.emit("upload://log", LogEvent {
                message: "上傳已停止".to_string(),
                level: "warn".to_string(),
            });
            break;
        }

        let hash = compute_sha256_head(std::path::Path::new(&f.abs_path)).unwrap_or_default();

        let dup = {
            let db = db_arc.lock().map_err(|_| AppError::Other("DB lock 失敗".to_string()))?;
            check_duplicate(&db, &params.event_id, &f.abs_path, f.file_size, f.mtime_ns, &hash)?
        };

        current += 1;
        let _ = app.emit("upload://progress", ProgressEvent {
            current,
            total,
            file_name: f.file_name.clone(),
        });

        if let DupCheck::Duplicate { .. } = dup {
            let _ = app.emit("upload://log", LogEvent {
                message: format!("⏭ 跳過（已上傳）：{}", f.file_name),
                level: "info".to_string(),
            });
            handles.push(tokio::spawn(async move { true }));
            continue;
        }

        let (lon, lat) = if let Some(ref ctx) = gpx_ctx {
            let coord = gpx::match_photo(std::path::Path::new(&f.abs_path), ctx);
            (coord.lon, coord.lat)
        } else {
            (params.longitude, params.latitude)
        };

        let permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();
        let client_clone = client.clone();
        let token = params.token.clone();
        let event_id = params.event_id.clone();
        let location = params.location.clone();
        let endpoint_clone = endpoint.clone();
        let timeout = params.timeout_secs;
        let abs_path = f.abs_path.clone();
        let file_name = f.file_name.clone();
        let db_arc_clone = Arc::clone(db_arc);
        let hash_clone = hash.clone();
        let file_size = f.file_size;
        let mtime_ns = f.mtime_ns;
        let app_clone = app.clone();

        let handle = tokio::spawn(async move {
            let _permit = permit;
            let path = std::path::Path::new(&abs_path);

            let result = upload_single(
                &client_clone, &token, path, &event_id, &location,
                lon, lat, &endpoint_clone, timeout, 3,
            ).await;

            if result.success {
                let _ = db_arc_clone.lock().map(|db| {
                    record_upload(&db, &event_id, result.photo_id.as_deref(),
                        &abs_path, file_size, mtime_ns, &hash_clone).ok()
                });
                let _ = app_clone.emit("upload://log", LogEvent {
                    message: format!("✅ 成功：{}", file_name),
                    level: "success".to_string(),
                });
            } else {
                let _ = app_clone.emit("upload://log", LogEvent {
                    message: format!("❌ 失敗：{} - {}", file_name,
                        result.error.as_deref().unwrap_or("")),
                    level: "error".to_string(),
                });
            }

            result.success
        });

        handles.push(handle);
    }

    let mut success = 0usize;
    let mut failed = 0usize;
    for handle in handles {
        match handle.await {
            Ok(true) => success += 1,
            _ => failed += 1,
        }
    }

    let _ = app.emit("upload://log", LogEvent {
        message: format!("完成｜成功 {}，失敗 {}", success, failed),
        level: "info".to_string(),
    });
    let _ = app.emit("upload://finished", FinishedEvent { success, failed });

    Ok(())
}

#[tauri::command]
async fn cmd_stop_upload(state: State<'_, AppState>) -> Result<(), AppError> {
    state.upload_running.store(false, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
async fn cmd_clear_history(
    state: State<'_, AppState>,
    event_id: String,
) -> Result<usize, AppError> {
    let db = state.db.lock().map_err(|_| AppError::Other("DB lock 失敗".to_string()))?;
    history::clear_event_history(&db, &event_id)
}

// ── Tauri setup ──────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = open_db().expect("無法開啟 SQLite 資料庫");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            db: Arc::new(Mutex::new(db)),
            upload_running: Arc::new(AtomicBool::new(false)),
            http_client: reqwest::Client::new(),
        })
        .invoke_handler(tauri::generate_handler![
            cmd_get_config,
            cmd_save_config,
            cmd_verify_token,
            cmd_list_events,
            cmd_start_auth_server,
            cmd_scan_folder,
            cmd_preview_gpx,
            cmd_start_upload,
            cmd_stop_upload,
            cmd_clear_history,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 啟動失敗");
}
