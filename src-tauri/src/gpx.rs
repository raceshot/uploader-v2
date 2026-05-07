use chrono::{DateTime, Utc};
use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::{AppError, Result};

#[derive(Debug, Clone)]
pub struct TrackPoint {
    pub timestamp_ms: i64,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhotoCoord {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub source: String,   // "gpx" | "manual-fallback" | "unmatched"
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpxPreviewRow {
    pub file_name: String,
    pub capture_time_utc: Option<String>,
    pub source: String,
    pub coord: Option<String>,
    pub note: String,
}

pub struct GpxContext {
    pub track_points: Vec<TrackPoint>,
    pub time_offset_ms: i64,
    pub fallback_lat: Option<f64>,
    pub fallback_lon: Option<f64>,
    pub fallback_mode: String,
    pub max_gap_ms: i64,
}

// ── GPX 解析 ──────────────────────────────────────────────────────────────────

pub fn parse_gpx(path: &Path) -> Result<Vec<TrackPoint>> {
    let xml = std::fs::read_to_string(path)
        .map_err(|e| AppError::Gpx(format!("讀取 GPX 失敗：{e}")))?;

    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(true);

    let mut points: Vec<TrackPoint> = Vec::new();
    let mut in_trkpt = false;
    let mut current_lat: Option<f64> = None;
    let mut current_lon: Option<f64> = None;
    let mut in_time = false;
    let mut time_buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name_bytes = e.name();
                let name = std::str::from_utf8(name_bytes.as_ref()).unwrap_or("");
                if name.ends_with("trkpt") {
                    in_trkpt = true;
                    current_lat = None;
                    current_lon = None;
                    for attr in e.attributes().flatten() {
                        let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                        let val = std::str::from_utf8(&attr.value).unwrap_or("").to_string();
                        match key {
                            "lat" => current_lat = val.parse().ok(),
                            "lon" => current_lon = val.parse().ok(),
                            _ => {}
                        }
                    }
                } else if in_trkpt && name.ends_with("time") {
                    in_time = true;
                    time_buf.clear();
                }
            }
            Ok(Event::Text(e)) if in_time => {
                time_buf.push_str(e.unescape().unwrap_or_default().as_ref());
            }
            Ok(Event::End(e)) => {
                let name_bytes = e.name();
                let name = std::str::from_utf8(name_bytes.as_ref()).unwrap_or("");
                if name.ends_with("time") && in_time {
                    in_time = false;
                }
                if name.ends_with("trkpt") {
                    if let (Some(lat), Some(lon)) = (current_lat, current_lon) {
                        if !time_buf.is_empty() {
                            if let Ok(dt) = time_buf.parse::<DateTime<Utc>>() {
                                points.push(TrackPoint {
                                    timestamp_ms: dt.timestamp_millis(),
                                    lat,
                                    lon,
                                });
                            }
                        }
                    }
                    in_trkpt = false;
                    time_buf.clear();
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(AppError::Gpx(format!("XML 解析錯誤：{e}"))),
            _ => {}
        }
    }

    if points.len() < 2 {
        return Err(AppError::Gpx(
            "GPX 軌跡點不足（至少需要 2 個含時間的 trkpt）".to_string(),
        ));
    }

    points.sort_by_key(|p| p.timestamp_ms);
    Ok(points)
}

// ── 線性插值 ─────────────────────────────────────────────────────────────────

pub fn interpolate(
    points: &[TrackPoint],
    target_ms: i64,
    max_gap_ms: i64,
) -> (Option<f64>, Option<f64>, String) {
    if points.is_empty() {
        return (None, None, "GPX 無軌跡點".to_string());
    }
    if target_ms < points[0].timestamp_ms {
        return (None, None, "照片時間早於 GPX 起點".to_string());
    }
    if target_ms > points[points.len() - 1].timestamp_ms {
        return (None, None, "照片時間晚於 GPX 終點".to_string());
    }

    for i in 1..points.len() {
        let prev = &points[i - 1];
        let curr = &points[i];
        if curr.timestamp_ms == target_ms {
            return (Some(curr.lat), Some(curr.lon), "精確匹配".to_string());
        }
        if curr.timestamp_ms > target_ms {
            let gap = curr.timestamp_ms - prev.timestamp_ms;
            if gap > max_gap_ms {
                return (None, None, format!("相鄰軌跡間隔 {:.0}s 超過上限", gap as f64 / 1000.0));
            }
            let ratio = (target_ms - prev.timestamp_ms) as f64 / gap as f64;
            let lat = prev.lat + (curr.lat - prev.lat) * ratio;
            let lon = prev.lon + (curr.lon - prev.lon) * ratio;
            return (Some(lat), Some(lon), "線性插值".to_string());
        }
    }

    let last = &points[points.len() - 1];
    (Some(last.lat), Some(last.lon), "使用最後一個軌跡點".to_string())
}

// ── EXIF 時間擷取 ─────────────────────────────────────────────────────────────

pub fn extract_exif_timestamp_ms(path: &Path) -> Option<i64> {
    use exif::{In, Reader, Tag};
    use std::io::BufReader;

    let file = std::fs::File::open(path).ok()?;
    let mut bufreader = BufReader::new(&file);
    let exif = Reader::new().read_from_container(&mut bufreader).ok()?;

    let date_str = exif
        .get_field(Tag::DateTimeOriginal, In::PRIMARY)
        .or_else(|| exif.get_field(Tag::DateTimeDigitized, In::PRIMARY))
        .or_else(|| exif.get_field(Tag::DateTime, In::PRIMARY))
        .map(|f| f.display_value().to_string())?;

    let offset_str = exif
        .get_field(Tag::OffsetTimeOriginal, In::PRIMARY)
        .or_else(|| exif.get_field(Tag::OffsetTimeDigitized, In::PRIMARY))
        .or_else(|| exif.get_field(Tag::OffsetTime, In::PRIMARY))
        .map(|f| f.display_value().to_string());

    let subsec_str = exif
        .get_field(Tag::SubSecTimeOriginal, In::PRIMARY)
        .or_else(|| exif.get_field(Tag::SubSecTimeDigitized, In::PRIMARY))
        .or_else(|| exif.get_field(Tag::SubSecTime, In::PRIMARY))
        .map(|f| f.display_value().to_string());

    parse_exif_datetime(&date_str, offset_str.as_deref(), subsec_str.as_deref())
}

fn parse_exif_datetime(date_str: &str, offset: Option<&str>, subsec: Option<&str>) -> Option<i64> {
    // 格式：2024:01:15 10:30:45
    let cleaned = date_str.trim().replace('"', "");
    if cleaned.len() < 19 {
        return None;
    }

    let year: i32 = cleaned[0..4].parse().ok()?;
    let month: u32 = cleaned[5..7].parse().ok()?;
    let day: u32 = cleaned[8..10].parse().ok()?;
    let hour: u32 = cleaned[11..13].parse().ok()?;
    let min: u32 = cleaned[14..16].parse().ok()?;
    let sec: u32 = cleaned[17..19].parse().ok()?;

    let subsec_ms: i64 = subsec
        .and_then(|s| {
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                None
            } else {
                let padded = format!("{:0<3}", &digits[..digits.len().min(3)]);
                padded.parse::<i64>().ok()
            }
        })
        .unwrap_or(0);

    // 解析時區偏移，預設 UTC+8
    let offset_secs: i64 = offset
        .and_then(|o| {
            let o = o.trim().replace('"', "");
            let sign: i64 = if o.starts_with('-') { -1 } else { 1 };
            let parts: Vec<&str> = o.trim_start_matches(['+', '-']).split(':').collect();
            if parts.len() == 2 {
                let h: i64 = parts[0].parse().ok()?;
                let m: i64 = parts[1].parse().ok()?;
                Some(sign * (h * 3600 + m * 60))
            } else {
                None
            }
        })
        .unwrap_or(8 * 3600); // 預設 UTC+8

    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    let naive = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(year, month, day)?,
        NaiveTime::from_hms_opt(hour, min, sec)?,
    );
    let utc_secs = naive.and_utc().timestamp() - offset_secs;
    Some(utc_secs * 1000 + subsec_ms)
}

// ── 對單張照片做 GPX 匹配 ────────────────────────────────────────────────────

pub fn match_photo(path: &Path, ctx: &GpxContext) -> PhotoCoord {
    let ts = extract_exif_timestamp_ms(path);

    let Some(ts) = ts else {
        return match ctx.fallback_mode.as_str() {
            "manual" if ctx.fallback_lat.is_some() => PhotoCoord {
                lat: ctx.fallback_lat,
                lon: ctx.fallback_lon,
                source: "manual-fallback".to_string(),
                note: "缺少 EXIF 拍攝時間，改用手動座標".to_string(),
            },
            _ => PhotoCoord {
                lat: None, lon: None,
                source: "unmatched".to_string(),
                note: "缺少 EXIF 拍攝時間".to_string(),
            },
        };
    };

    let adjusted = ts + ctx.time_offset_ms;
    let (lat, lon, note) = interpolate(&ctx.track_points, adjusted, ctx.max_gap_ms);

    if lat.is_some() {
        return PhotoCoord { lat, lon, source: "gpx".to_string(), note };
    }

    match ctx.fallback_mode.as_str() {
        "manual" if ctx.fallback_lat.is_some() => PhotoCoord {
            lat: ctx.fallback_lat,
            lon: ctx.fallback_lon,
            source: "manual-fallback".to_string(),
            note: format!("{note}，改用手動座標"),
        },
        _ => PhotoCoord { lat: None, lon: None, source: "unmatched".to_string(), note },
    }
}
