//! Saved cameras, their control-API clients and their live status.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde_json::Value;
use tapo_camera::{CameraConfig, CertFingerprint};
use tauri::{AppHandle, Emitter};

use crate::db::{CameraRecord, Db};
use crate::error::{ApiError, ApiResult};
use crate::model::{
    AddCameraRequest, AppEvent, Camera, CameraState, CameraStatus, DiscoveredDevice, StorageInfo,
    UpdateCameraRequest,
};
use crate::secrets;

/// Name of the Tauri event carrying [`AppEvent`]s to the UI.
pub const EVENT_NAME: &str = "backsight://event";

const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// How the camera's clock relates to UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockInfo {
    /// Seconds to add to a camera-clock timestamp to get a UTC unix timestamp.
    pub correction: i64,
    /// The camera's local time offset from UTC, in minutes.
    pub utc_offset_minutes: i32,
}

#[derive(Debug, Clone, Default)]
struct LiveInfo {
    storage: Option<StorageInfo>,
    clock: Option<ClockInfo>,
}

pub struct CameraHandle {
    pub id: String,
    record: RwLock<CameraRecord>,
    client: RwLock<Arc<tapo_camera::Camera>>,
    status: RwLock<CameraStatus>,
    info: RwLock<LiveInfo>,
    user_id: tokio::sync::Mutex<Option<u64>>,
    /// Held while a media session is open: cameras allow only one.
    pub media: tokio::sync::Mutex<()>,
}

impl CameraHandle {
    fn new(record: CameraRecord, client: tapo_camera::Camera) -> Self {
        Self {
            id: record.id.clone(),
            record: RwLock::new(record),
            client: RwLock::new(Arc::new(client)),
            status: RwLock::new(CameraStatus::new(CameraState::Connecting)),
            info: RwLock::new(LiveInfo::default()),
            user_id: tokio::sync::Mutex::new(None),
            media: tokio::sync::Mutex::new(()),
        }
    }

    pub fn client(&self) -> Arc<tapo_camera::Camera> {
        self.client.read().expect("client lock").clone()
    }

    pub fn record(&self) -> CameraRecord {
        self.record.read().expect("record lock").clone()
    }

    pub fn clock(&self) -> Option<ClockInfo> {
        self.info.read().expect("info lock").clock
    }

    pub fn status(&self) -> CameraStatus {
        self.status.read().expect("status lock").clone()
    }

    /// The owner password, for the media stream.
    pub fn cloud_password(&self) -> ApiResult<String> {
        secrets::cloud_password(&self.id)
            .ok_or_else(|| ApiError::new("auth_failed", "No password is saved for this camera."))
    }

    /// The playback user id, fetched once and reused.
    pub async fn user_id(&self, refresh: bool) -> ApiResult<u64> {
        let mut cached = self.user_id.lock().await;
        if let (Some(id), false) = (*cached, refresh) {
            return Ok(id);
        }
        let id = self.client().user_id().await?;
        *cached = Some(id);
        Ok(id)
    }

    pub fn to_api(&self) -> Camera {
        let record = self.record();
        let info = self.info.read().expect("info lock").clone();
        Camera {
            id: record.id,
            name: record.name,
            host: record.host,
            model: record.model,
            firmware: record.firmware,
            mac: record.mac,
            group_ids: record.group_ids,
            favorite: record.favorite,
            has_camera_account: record.has_camera_account,
            status: self.status(),
            storage: info.storage,
            video_codec: None,
            utc_offset_minutes: info.clock.map(|c| c.utc_offset_minutes),
            time_zone: None,
            snapshot_url: None,
        }
    }
}

pub struct CameraManager {
    app: AppHandle,
    db: Arc<Db>,
    cameras: RwLock<HashMap<String, Arc<CameraHandle>>>,
}

fn client_for(record: &CameraRecord, password: String) -> ApiResult<tapo_camera::Camera> {
    let pin = record
        .cert_pin
        .as_deref()
        .and_then(CertFingerprint::from_hex);
    Ok(tapo_camera::Camera::new(
        CameraConfig::new(&record.host, password).with_certificate(pin),
    )?)
}

fn now_rfc3339() -> String {
    jiff::Timestamp::now().to_string()
}

fn random_id() -> String {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).expect("OS random number generator");
    hex::encode(bytes)
}

impl CameraManager {
    /// Loads saved cameras and starts polling their status.
    pub fn start(app: AppHandle, db: Arc<Db>) -> ApiResult<Arc<Self>> {
        let manager = Arc::new(Self {
            app,
            db,
            cameras: RwLock::new(HashMap::new()),
        });
        for record in manager.db.cameras()? {
            let password = secrets::cloud_password(&record.id).unwrap_or_default();
            match client_for(&record, password) {
                Ok(client) => {
                    let handle = Arc::new(CameraHandle::new(record, client));
                    manager.insert(handle);
                }
                Err(err) => tracing::warn!(camera = %record.id, %err, "could not create client"),
            }
        }
        let poller = manager.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                for handle in poller.handles() {
                    poller.refresh(&handle).await;
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        });
        Ok(manager)
    }

    fn insert(&self, handle: Arc<CameraHandle>) {
        self.cameras
            .write()
            .expect("cameras lock")
            .insert(handle.id.clone(), handle);
    }

    pub fn handles(&self) -> Vec<Arc<CameraHandle>> {
        let mut handles: Vec<_> = self
            .cameras
            .read()
            .expect("cameras lock")
            .values()
            .cloned()
            .collect();
        handles.sort_by(|a, b| a.record().created_at.cmp(&b.record().created_at));
        handles
    }

    pub fn get(&self, id: &str) -> ApiResult<Arc<CameraHandle>> {
        self.cameras
            .read()
            .expect("cameras lock")
            .get(id)
            .cloned()
            .ok_or_else(|| ApiError::not_found("camera"))
    }

    pub fn list(&self) -> Vec<Camera> {
        self.handles().iter().map(|h| h.to_api()).collect()
    }

    pub fn emit(&self, event: AppEvent) {
        if let Err(err) = self.app.emit(EVENT_NAME, event) {
            tracing::warn!(%err, "could not emit event");
        }
    }

    fn set_status(&self, handle: &CameraHandle, status: CameraStatus) {
        let changed = {
            let mut current = handle.status.write().expect("status lock");
            let changed = current.state != status.state
                || current.message != status.message
                || current.locked_until != status.locked_until;
            *current = status.clone();
            changed
        };
        if changed {
            self.emit(AppEvent::CameraStatus {
                camera_id: handle.id.clone(),
                status,
            });
        }
    }

    /// Polls clock, SD card and privacy mode in one request and updates the status.
    pub async fn refresh(&self, handle: &CameraHandle) {
        let client = handle.client();
        let calls = [
            (
                "getClockStatus",
                serde_json::json!({ "system": { "name": "clock_status" } }),
            ),
            (
                "getSdCardStatus",
                serde_json::json!({ "harddisk_manage": { "table": ["hd_info"] } }),
            ),
            (
                "getLensMaskConfig",
                serde_json::json!({ "lens_mask": { "name": ["lens_mask_info"] } }),
            ),
        ];
        match client.execute_many(&calls).await {
            Ok(results) => {
                let mut info = handle.info.read().expect("info lock").clone();
                if let Some(Ok(clock)) = results.first() {
                    info.clock =
                        parse_clock(clock, jiff::Timestamp::now().as_second()).or(info.clock);
                }
                if let Some(Ok(sd)) = results.get(1) {
                    info.storage = parse_storage(sd);
                }
                let privacy = results
                    .get(2)
                    .and_then(|r| r.as_ref().ok())
                    .and_then(|v| v.pointer("/lens_mask/lens_mask_info/enabled"))
                    .and_then(Value::as_str)
                    == Some("on");
                *handle.info.write().expect("info lock") = info;
                let mut status = CameraStatus::new(if privacy {
                    CameraState::Privacy
                } else {
                    CameraState::Online
                });
                status.last_seen = Some(now_rfc3339());
                self.set_status(handle, status);
            }
            Err(err) => {
                let api: ApiError = err.into();
                let mut status = CameraStatus::new(match api.code {
                    "auth_failed" => CameraState::AuthFailed,
                    "camera_locked" => CameraState::Locked,
                    "unsupported" => CameraState::Unsupported,
                    _ => CameraState::Offline,
                });
                status.message = Some(api.message);
                status.locked_until = api.retry_at;
                status.last_seen = handle.status().last_seen;
                self.set_status(handle, status);
            }
        }
    }

    pub async fn discover(&self, timeout: Duration) -> ApiResult<Vec<DiscoveredDevice>> {
        let found = tapo_camera::discovery::discover(timeout)
            .await
            .map_err(|e| ApiError::internal(format!("discovery failed: {e}")))?;
        let known: Vec<CameraRecord> = self.handles().iter().map(|h| h.record()).collect();
        Ok(found
            .into_iter()
            .filter_map(|device| {
                use tapo_camera::discovery::{DeviceKind, LoginScheme};
                let kind = match device.kind {
                    DeviceKind::Camera => "camera",
                    DeviceKind::Doorbell => "doorbell",
                    DeviceKind::Hub => "hub",
                    DeviceKind::Other(_) => return None,
                };
                let host = device.ip.to_string();
                let already_added = known.iter().any(|r| {
                    r.host == host || (r.mac.is_some() && r.mac.as_deref() == device.mac.as_deref())
                });
                Some(DiscoveredDevice {
                    host,
                    mac: device.mac.clone(),
                    model: device.model.clone(),
                    name: device
                        .raw
                        .get("device_name")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    firmware: device
                        .raw
                        .get("firmware_version")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    kind: kind.into(),
                    login_scheme: match device.login_scheme() {
                        LoginScheme::Secure => "secure",
                        LoginScheme::Tpap => "tpap",
                        LoginScheme::Unknown => "unknown",
                    }
                    .into(),
                    already_added,
                })
            })
            .collect())
    }

    pub async fn add(&self, request: AddCameraRequest) -> ApiResult<Camera> {
        let host = request.host.trim().to_owned();
        if host.is_empty() {
            return Err(ApiError::invalid("Enter the camera's IP address."));
        }
        if request.cloud_password.is_empty() {
            return Err(ApiError::invalid("Enter the TP-Link account password."));
        }
        if self.handles().iter().any(|h| h.record().host == host) {
            return Err(ApiError::invalid("This camera is already added."));
        }

        let client =
            tapo_camera::Camera::new(CameraConfig::new(&host, request.cloud_password.clone()))?;
        client.login().await?;
        let info = client.device_info().await?;
        let text = |key: &str| info.get(key).and_then(Value::as_str).map(str::to_owned);

        let record = CameraRecord {
            id: random_id(),
            name: request
                .name
                .filter(|n| !n.trim().is_empty())
                .or_else(|| text("device_alias"))
                .unwrap_or_else(|| host.clone()),
            host,
            model: text("device_model"),
            firmware: text("sw_version"),
            mac: text("mac"),
            group_ids: request.group_ids.unwrap_or_default(),
            favorite: false,
            has_camera_account: request.camera_account.is_some(),
            cert_pin: client.certificate().await.map(|f| f.to_hex()),
            created_at: now_rfc3339(),
        };

        secrets::set_cloud_password(&record.id, &request.cloud_password)?;
        if let Some(account) = &request.camera_account {
            secrets::set_camera_account(&record.id, account)?;
        }
        self.db.upsert_camera(&record)?;

        let handle = Arc::new(CameraHandle::new(record, client));
        self.insert(handle.clone());
        self.refresh(&handle).await;
        self.emit(AppEvent::CamerasChanged);
        Ok(handle.to_api())
    }

    pub async fn update(&self, id: &str, request: UpdateCameraRequest) -> ApiResult<Camera> {
        let handle = self.get(id)?;
        let mut record = handle.record();

        if let Some(password) = request.cloud_password.filter(|p| !p.is_empty()) {
            // Verify before replacing the working password.
            let client = client_for(&record, password.clone())?;
            client.login().await?;
            secrets::set_cloud_password(id, &password)?;
            *handle.client.write().expect("client lock") = Arc::new(client);
            *handle.user_id.lock().await = None;
        }
        if let Some(name) = request.name.filter(|n| !n.trim().is_empty()) {
            record.name = name.trim().to_owned();
        }
        if let Some(groups) = request.group_ids {
            record.group_ids = groups;
        }
        if let Some(favorite) = request.favorite {
            record.favorite = favorite;
        }
        match request.camera_account {
            Some(Some(account)) => {
                secrets::set_camera_account(id, &account)?;
                record.has_camera_account = true;
            }
            Some(None) => {
                secrets::delete_camera_account(id);
                record.has_camera_account = false;
            }
            None => {}
        }

        self.db.upsert_camera(&record)?;
        *handle.record.write().expect("record lock") = record;
        self.refresh(&handle).await;
        self.emit(AppEvent::CamerasChanged);
        Ok(handle.to_api())
    }

    pub fn remove(&self, id: &str) -> ApiResult<()> {
        self.get(id)?;
        self.db.delete_camera(id)?;
        secrets::delete_all(id);
        self.cameras.write().expect("cameras lock").remove(id);
        self.emit(AppEvent::CamerasChanged);
        Ok(())
    }
}

/// Reads `getClockStatus`: the camera clock and its local wall time.
pub fn parse_clock(value: &Value, host_now: i64) -> Option<ClockInfo> {
    let clock = value.pointer("/system/clock_status")?;
    let camera_seconds = clock
        .get("seconds_from_1970")
        .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))?;
    let correction = host_now - camera_seconds;

    // Local wall time as if it were UTC, minus real UTC now = the zone offset.
    let utc_offset_minutes = clock
        .get("local_time")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<jiff::civil::DateTime>().ok())
        .and_then(|local| local.to_zoned(jiff::tz::TimeZone::UTC).ok())
        .map(|zoned| {
            let offset = zoned.timestamp().as_second() - host_now;
            // Round to the nearest quarter hour; clocks drift by seconds.
            ((offset as f64 / 900.0).round() * 15.0) as i32
        })
        .unwrap_or(0);

    Some(ClockInfo {
        correction,
        utc_offset_minutes,
    })
}

/// Parses sizes such as `"59.5GB"`, `"512MB"` or plain numbers (MB).
fn parse_size(value: &Value) -> Option<u64> {
    if let Some(n) = value.as_f64() {
        return Some((n * 1024.0 * 1024.0) as u64);
    }
    let text = value.as_str()?.trim().to_ascii_uppercase();
    let (number, multiplier) = if let Some(n) = text.strip_suffix("TB") {
        (n, 1u64 << 40)
    } else if let Some(n) = text.strip_suffix("GB") {
        (n, 1u64 << 30)
    } else if let Some(n) = text.strip_suffix("MB") {
        (n, 1u64 << 20)
    } else if let Some(n) = text.strip_suffix("KB") {
        (n, 1u64 << 10)
    } else {
        (text.as_str(), 1u64 << 20)
    };
    number
        .trim()
        .parse::<f64>()
        .ok()
        .map(|n| (n * multiplier as f64) as u64)
}

/// Reads `getSdCardStatus` → `harddisk_manage.hd_info`, a list of single-key objects.
pub fn parse_storage(value: &Value) -> Option<StorageInfo> {
    let list = value.pointer("/harddisk_manage/hd_info")?.as_array()?;
    let disk = list
        .iter()
        .find_map(|item| item.as_object()?.values().next())?;
    let text = |key: &str| disk.get(key).and_then(Value::as_str).unwrap_or("");
    let status = match text("status") {
        "insufficient" | "full" => "full",
        "unformatted" => "unformatted",
        "abnormal" | "error" => "error",
        "" | "offline" => "none",
        _ => "normal",
    };
    Some(StorageInfo {
        present: status != "none",
        status: status.into(),
        total_bytes: disk.get("total_space").and_then(parse_size).unwrap_or(0),
        free_bytes: disk.get("free_space").and_then(parse_size).unwrap_or(0),
        recording_mode: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn clock_offset_and_correction() {
        // Camera runs on UTC+5:30 and its seconds_from_1970 is 5 s behind the host.
        let host_now = 1_790_000_000; // 2026-09-21T14:13:20Z
        let clock = json!({ "system": { "clock_status": {
            "seconds_from_1970": host_now - 5,
            "local_time": "2026-09-21 19:43:15"
        } } });
        let info = parse_clock(&clock, host_now).unwrap();
        assert_eq!(info.correction, 5);
        assert_eq!(info.utc_offset_minutes, 330);
    }

    #[test]
    fn storage_sizes() {
        let sd = json!({ "harddisk_manage": { "hd_info": [ { "hd_info_1": {
            "status": "normal", "total_space": "59.5GB", "free_space": "12.25GB"
        } } ] } });
        let info = parse_storage(&sd).unwrap();
        assert!(info.present);
        assert_eq!(info.status, "normal");
        assert_eq!(info.total_bytes, (59.5 * (1u64 << 30) as f64) as u64);
        assert!(info.free_bytes < info.total_bytes);
    }
}
