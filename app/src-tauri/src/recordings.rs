//! Which days have footage, and the recordings and detection events of a day.

use serde_json::Value;

use crate::cameras::{CameraHandle, ClockInfo};
use crate::db::Db;
use crate::error::{ApiError, ApiResult};
use crate::model::{DayIndex, DetectionEvent, RecordingKind, RecordingSegment};
use crate::thumbnails;

/// Past days rarely change; today's index is refreshed after this many seconds.
const TODAY_CACHE_SECONDS: i64 = 60;

fn parse_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn iso(utc_seconds: i64) -> String {
    jiff::Timestamp::from_second(utc_seconds)
        .map(|t| t.to_string())
        .unwrap_or_default()
}

fn parse_date(date: &str) -> ApiResult<jiff::civil::Date> {
    date.parse()
        .map_err(|_| ApiError::invalid(format!("invalid date {date:?}, expected YYYY-MM-DD")))
}

fn clock_of(handle: &CameraHandle) -> ClockInfo {
    handle.clock().unwrap_or(ClockInfo {
        correction: 0,
        utc_offset_minutes: 0,
    })
}

/// Camera-clock unix seconds of local midnight on `date` and of the next midnight.
fn day_bounds(date: jiff::civil::Date, clock: ClockInfo) -> (i64, i64) {
    let naive_midnight = date
        .at(0, 0, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .map(|z| z.timestamp().as_second())
        .unwrap_or(0);
    let utc_midnight = naive_midnight - i64::from(clock.utc_offset_minutes) * 60;
    let start = utc_midnight - clock.correction;
    (start, start + 86_400)
}

/// `YYYY-MM` → the dates of that month with recordings, as `YYYY-MM-DD`.
pub async fn days_with_recordings(handle: &CameraHandle, month: &str) -> ApiResult<Vec<String>> {
    let first = parse_date(&format!("{month}-01"))?;
    let last = first.last_of_month();
    let fmt = |d: jiff::civil::Date| d.strftime("%Y%m%d").to_string();
    let result = handle
        .tapo_client()?
        .days_with_recordings(&fmt(first), &fmt(last))
        .await;
    let list = match result {
        Ok(list) => list,
        // -71105: "search failed", which cameras also return for "nothing found".
        Err(err) if err.camera_code() == Some(-71105) => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    Ok(parse_days(&list))
}

/// `searchDateWithVideo` results: `[{ "search_results_1": { "date": "20260916" } }, …]`
/// (flat `{ "date": … }` objects and plain strings are accepted too).
pub fn parse_days(list: &Value) -> Vec<String> {
    fn date_of(value: &Value) -> Option<String> {
        match value {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Object(o) => match o.get("date") {
                Some(date) => date_of(date),
                None => o.values().find_map(date_of),
            },
            _ => None,
        }
    }
    let mut days: Vec<String> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(date_of)
        .filter(|d| d.len() == 8)
        .map(|d| format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..]))
        .collect();
    days.sort();
    days.dedup();
    days
}

/// Recordings and detection events of one camera-local day, cached in the database.
pub async fn day_index(handle: &CameraHandle, db: &Db, date: &str) -> ApiResult<DayIndex> {
    let client = handle.tapo_client()?;
    let day = parse_date(date)?;
    let clock = clock_of(handle);
    let now = jiff::Timestamp::now().as_second();
    let (start, end) = day_bounds(day, clock);
    let is_past_day = end + clock.correction < now;

    if let Some((json, fetched_at)) = db.cached_day(&handle.id, date)?
        && (is_past_day || now - fetched_at < TODAY_CACHE_SECONDS)
        && let Ok(cached) = serde_json::from_str::<CachedDay>(&json)
        && cached.version == CACHE_VERSION
    {
        return Ok(cached.into_index(&handle.id, date, clock));
    }

    let compact = day.strftime("%Y%m%d").to_string();
    let recordings = match fetch_recordings(handle, &compact).await {
        Ok(list) => list,
        Err(err) if err.camera_code() == Some(-71105) => Value::Array(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let events = match client.detection_events(start, end).await {
        Ok(list) => list,
        Err(err) if err.camera_code() == Some(-71105) => Value::Array(Vec::new()),
        // Some models don't support the detection list; recordings still work.
        Err(err) if err.camera_code() == Some(-40106) || err.camera_code() == Some(-40210) => {
            Value::Array(Vec::new())
        }
        Err(err) => return Err(err.into()),
    };

    let cached = CachedDay {
        version: CACHE_VERSION,
        segments: parse_segments(&recordings),
        events: parse_events(&events),
    };
    db.cache_day(
        &handle.id,
        date,
        &serde_json::to_string(&cached).expect("JSON"),
        now,
    )?;
    Ok(cached.into_index(&handle.id, date, clock))
}

/// `searchVideoOfDay`, refreshing the playback user id once if the camera rejects it.
async fn fetch_recordings(handle: &CameraHandle, date: &str) -> tapo_camera::Result<Value> {
    let client = handle
        .tapo_client()
        .map_err(|err| tapo_camera::Error::Protocol(err.message))?;
    let user_id = match handle.user_id(false).await {
        Ok(id) => id,
        Err(_) => return client.recordings_of_day(date, 0).await,
    };
    match client.recordings_of_day(date, user_id).await {
        Err(err) if err.camera_code() == Some(-71103) => {
            let fresh = handle.user_id(true).await.unwrap_or(user_id);
            client.recordings_of_day(date, fresh).await
        }
        other => other,
    }
}

/// Bump when the cached layout or meaning changes.
const CACHE_VERSION: u32 = 2;

/// What the camera reported for a day, in camera-clock seconds (converted to UTC when
/// read, with the current clock correction).
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedDay {
    #[serde(default)]
    version: u32,
    segments: Vec<CachedSegment>,
    events: Vec<CachedEvent>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
struct CachedSegment {
    start: i64,
    end: i64,
    detection: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
struct CachedEvent {
    start: i64,
    end: i64,
    types: Vec<String>,
}

impl CachedDay {
    fn into_index(self, camera_id: &str, date: &str, clock: ClockInfo) -> DayIndex {
        let utc = |camera_seconds: i64| iso(camera_seconds + clock.correction);
        DayIndex {
            camera_id: camera_id.to_owned(),
            date: date.to_owned(),
            segments: self
                .segments
                .into_iter()
                .map(|s| RecordingSegment {
                    start: utc(s.start),
                    end: utc(s.end),
                    kind: if s.detection {
                        RecordingKind::Detection
                    } else {
                        RecordingKind::Continuous
                    },
                })
                .collect(),
            events: self
                .events
                .into_iter()
                .map(|e| DetectionEvent {
                    id: format!("{camera_id}-{}", e.start),
                    start: utc(e.start),
                    end: utc(e.end),
                    types: e.types,
                    thumbnail_url: Some(thumbnails::url(camera_id, e.start)),
                })
                .collect(),
        }
    }
}

/// `searchVideoOfDay` results: single-key objects `{ "…": { startTime, endTime, vedio_type } }`.
fn parse_segments(list: &Value) -> Vec<CachedSegment> {
    let mut segments: Vec<CachedSegment> = list
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|item| match item.as_object() {
            Some(obj) if obj.contains_key("startTime") => vec![item],
            Some(obj) => obj.values().collect(),
            None => Vec::new(),
        })
        .filter_map(|rec| {
            let start = parse_i64(rec.get("startTime")?)?;
            let end = parse_i64(rec.get("endTime")?)?;
            // The firmware spells it "vedio_type"; 1 = timed (continuous) recording.
            let kind = rec
                .get("vedio_type")
                .or_else(|| rec.get("video_type"))
                .and_then(parse_i64)
                .unwrap_or(1);
            (end > start).then_some(CachedSegment {
                start,
                end,
                detection: kind != 1,
            })
        })
        .collect();
    segments.sort_by_key(|s| s.start);
    segments
}

/// Maps the camera's numeric detection types to the UI's event type names.
fn event_type_name(code: i64) -> &'static str {
    match code {
        2 => "motion",
        3 => "tamper",
        4 => "line_crossing",
        5 => "area_intrusion",
        6 => "person",
        7 => "baby_cry",
        8 => "vehicle",
        9 | 33 => "pet",
        10 | 17 | 18 => "doorbell",
        11..=14 => "sound",
        _ => "other",
    }
}

fn collect_codes(value: &Value, out: &mut Vec<i64>) {
    match value {
        Value::Array(items) => items.iter().for_each(|v| collect_codes(v, out)),
        other => {
            if let Some(code) = parse_i64(other) {
                out.push(code);
            }
        }
    }
}

/// Detection types in an `events_N` bitmask: bit `b` of `events_N` stands for type
/// `(N - 1) * 32 + b + 1` (e.g. `events_1 = 162` = motion + person + vehicle).
fn codes_from_bitmask(key: &str, mask: i64, out: &mut Vec<i64>) {
    let Some(word) = key
        .strip_prefix("events_")
        .and_then(|n| n.parse::<i64>().ok())
        .filter(|n| *n >= 1)
    else {
        return;
    };
    for bit in 0..32 {
        if mask & (1 << bit) != 0 {
            out.push((word - 1) * 32 + bit + 1);
        }
    }
}

/// `searchDetectionList` results, e.g.
/// `{ "start_time": …, "end_time": …, "alarm_type": 6, "events_1": 162 }`: `alarm_type`
/// is the main detection, `events_N` a bitmask of everything detected in the clip.
fn parse_events(list: &Value) -> Vec<CachedEvent> {
    let mut events: Vec<CachedEvent> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|event| {
            let start = parse_i64(event.get("start_time")?)?;
            let end = event
                .get("end_time")
                .and_then(parse_i64)
                .filter(|e| *e >= start)
                .unwrap_or(start);

            let mut primary = Vec::new();
            for key in ["alarm_type", "event_type", "video_type", "type"] {
                if let Some(v) = event.get(key) {
                    collect_codes(v, &mut primary);
                }
            }
            let mut detected = Vec::new();
            if let Some(obj) = event.as_object() {
                for (key, value) in obj {
                    if let Some(mask) = parse_i64(value) {
                        codes_from_bitmask(key, mask, &mut detected);
                    }
                }
            }

            // Main type first (the UI colours by it), then the rest in a stable order.
            let mut types: Vec<String> = Vec::new();
            for code in primary.into_iter().chain({
                detected.sort_unstable();
                detected
            }) {
                let name = event_type_name(code).to_owned();
                if code != 1 && !types.contains(&name) {
                    types.push(name);
                }
            }
            // Smart detections (person, vehicle…) come with motion too; keep motion
            // only when it's all there is.
            if types.len() > 1 {
                types.retain(|t| t != "motion");
            }
            if types.is_empty() {
                types.push("motion".into());
            }
            Some(CachedEvent { start, end, types })
        })
        .collect();
    events.sort_by_key(|e| e.start);
    events.dedup_by_key(|e| e.start);
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CLOCK: ClockInfo = ClockInfo {
        correction: 10,
        utc_offset_minutes: 330,
    };

    #[test]
    fn days_from_real_search_results() {
        // Shape returned by a C325WB (firmware 1.4.4).
        let list = json!([
            { "search_results_1": { "date": "20260916" } },
            { "search_results_2": { "date": "20260917" } }
        ]);
        assert_eq!(parse_days(&list), vec!["2026-09-16", "2026-09-17"]);
    }

    #[test]
    fn events_decode_alarm_type_and_bitmask() {
        // Real combinations seen on a C325WB: motion only, person, vehicle, all three.
        let list = json!([
            { "alarm_type": 2, "events_1": 2, "start_time": 100, "end_time": 110 },
            { "alarm_type": 6, "events_1": 34, "start_time": 200, "end_time": 210 },
            { "alarm_type": 8, "events_1": 130, "start_time": 300, "end_time": 310 },
            { "alarm_type": 6, "events_1": 162, "start_time": 400, "end_time": 410 }
        ]);
        let events = parse_events(&list);
        let types: Vec<_> = events.iter().map(|e| e.types.clone()).collect();
        assert_eq!(
            types,
            vec![
                vec!["motion".to_string()],
                vec!["person".to_string()],
                vec!["vehicle".to_string()],
                vec!["person".to_string(), "vehicle".to_string()],
            ]
        );
    }

    #[test]
    fn days_from_search_results() {
        let list = json!([{ "date": "20260929" }, { "date": "20260901" }, "20260915", { "date": 20260920 }]);
        assert_eq!(
            parse_days(&list),
            vec!["2026-09-01", "2026-09-15", "2026-09-20", "2026-09-29"]
        );
    }

    #[test]
    fn segments_from_single_key_objects() {
        let list = json!([
            { "search_video_results_2": { "startTime": "200", "endTime": 260, "vedio_type": 2 } },
            { "search_video_results_1": { "startTime": 100, "endTime": 150, "vedio_type": 1 } },
            { "bad": { "startTime": 300, "endTime": 300 } }
        ]);
        let segments = parse_segments(&list);
        assert_eq!(
            segments,
            vec![
                CachedSegment {
                    start: 100,
                    end: 150,
                    detection: false
                },
                CachedSegment {
                    start: 200,
                    end: 260,
                    detection: true
                },
            ]
        );
    }

    #[test]
    fn events_map_types() {
        let list = json!([
            { "start_time": 500, "end_time": 520, "event_type": [6, 2] },
            { "start_time": 400, "end_time": 410 },
            { "start_time": 500, "end_time": 530, "event_type": 8 }
        ]);
        let events = parse_events(&list);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].types, vec!["motion"]);
        assert_eq!(events[1].types, vec!["person"]);
        assert_eq!(events[1].start, 500);

        // Converted to UTC with the clock correction when read, with thumbnails.
        let index = CachedDay {
            version: CACHE_VERSION,
            segments: Vec::new(),
            events,
        }
        .into_index("cam", "1970-01-01", CLOCK);
        assert_eq!(index.events[1].start, "1970-01-01T00:08:30Z");
        assert!(
            index.events[1]
                .thumbnail_url
                .as_deref()
                .unwrap()
                .ends_with("/cam/500")
        );
    }

    #[test]
    fn day_bounds_follow_local_midnight() {
        let date: jiff::civil::Date = "2026-09-29".parse().unwrap();
        let (start, end) = day_bounds(date, CLOCK);
        // 2026-09-29T00:00+05:30 = 2026-09-28T18:30Z, minus the 10 s correction.
        let expected: jiff::Timestamp = "2026-09-28T18:30:00Z".parse().unwrap();
        assert_eq!(start, expected.as_second() - 10);
        assert_eq!(end - start, 86_400);
    }
}
