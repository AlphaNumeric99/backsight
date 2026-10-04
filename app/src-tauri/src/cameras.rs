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
    AddCameraRequest, AddQuboCameraRequest, AppEvent, Brand, Camera, CameraState, CameraStatus,
    DiscoveredDevice, QuboCloudDevice, StorageInfo, UpdateCameraRequest,
};
use crate::previews::{self, Previews};
use crate::qubo::Qubo;
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
    /// The Tapo control client; cloud cameras (Qubo) have none.
    client: RwLock<Option<Arc<tapo_camera::Camera>>>,
    status: RwLock<CameraStatus>,
    info: RwLock<LiveInfo>,
    user_id: tokio::sync::Mutex<Option<u64>>,
    audio_rate: tokio::sync::OnceCell<u32>,
}

impl CameraHandle {
    fn new(record: CameraRecord, client: Option<tapo_camera::Camera>) -> Self {
        Self {
            id: record.id.clone(),
            record: RwLock::new(record),
            client: RwLock::new(client.map(Arc::new)),
            status: RwLock::new(CameraStatus::new(CameraState::Connecting)),
            info: RwLock::new(LiveInfo::default()),
            user_id: tokio::sync::Mutex::new(None),
            audio_rate: tokio::sync::OnceCell::new(),
        }
    }

    /// How this camera is reached.
    pub fn brand(&self) -> Brand {
        self.record().brand
    }

    /// The cloud device id of a Qubo camera (its `host` column).
    pub fn device_uuid(&self) -> String {
        self.record().host
    }

    /// The Tapo control client; errors for cloud cameras, whose features are gated
    /// off before this is reachable.
    pub fn tapo_client(&self) -> ApiResult<Arc<tapo_camera::Camera>> {
        self.client
            .read()
            .expect("client lock")
            .clone()
            .ok_or_else(|| {
                ApiError::new(
                    "unsupported",
                    "This camera is a Qubo cloud camera; this feature needs a camera with an \
                     SD card on your network.",
                )
            })
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
        let id = self.tapo_client()?.user_id().await?;
        *cached = Some(id);
        Ok(id)
    }

    /// The microphone sample rate (from `getAudioConfig`, in kHz there), 8 kHz if unknown.
    pub async fn audio_rate(&self) -> u32 {
        *self
            .audio_rate
            .get_or_init(|| async {
                let config = match self.tapo_client() {
                    Ok(client) => client.audio_config().await.ok(),
                    Err(_) => None,
                };
                config
                    .and_then(|config| {
                        let rate = config.pointer("/microphone/sampling_rate")?;
                        rate.as_u64().or_else(|| rate.as_str()?.parse().ok())
                    })
                    .map(|khz| (khz * 1000) as u32)
                    .filter(|hz| (8_000..=48_000).contains(hz))
                    .unwrap_or(8_000)
            })
            .await
    }

    /// `preview_at`: when the camera's preview was saved (ms since the epoch), if it has one.
    fn to_api(&self, preview_at: Option<i64>) -> Camera {
        let record = self.record();
        let info = self.info.read().expect("info lock").clone();
        Camera {
            id: record.id,
            name: record.name,
            brand: record.brand,
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
            snapshot_url: preview_at.map(|ms| previews::url(&self.id, ms)),
            snapshot_at: preview_at.map(rfc3339_ms),
        }
    }
}

pub struct CameraManager {
    app: AppHandle,
    db: Arc<Db>,
    previews: Previews,
    qubo: Arc<Qubo>,
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

fn rfc3339_ms(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.to_string())
        .unwrap_or_default()
}

fn random_id() -> String {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).expect("OS random number generator");
    hex::encode(bytes)
}

/// The Qubo device types the cloud calls cameras: anything with video that isn't a
/// phone or the app itself.
fn is_qubo_camera(device: &crate::qubo::Device) -> bool {
    let kind = device.device_type.as_str();
    !matches!(kind, "mobile")
        && (kind.to_ascii_lowercase().contains("cam") || kind == "videoDoorbell")
}

impl CameraManager {
    /// Loads saved cameras and starts polling their status.
    pub fn start(
        app: AppHandle,
        db: Arc<Db>,
        previews: Previews,
        qubo: Arc<Qubo>,
    ) -> ApiResult<Arc<Self>> {
        let manager = Arc::new(Self {
            app,
            db,
            previews,
            qubo,
            cameras: RwLock::new(HashMap::new()),
        });
        for record in manager.db.cameras()? {
            match record.brand {
                Brand::Tapo => {
                    let password = secrets::cloud_password(&record.id).unwrap_or_default();
                    match client_for(&record, password) {
                        Ok(client) => {
                            let handle = Arc::new(CameraHandle::new(record, Some(client)));
                            manager.insert(handle);
                        }
                        Err(err) => {
                            tracing::warn!(camera = %record.id, %err, "could not create client")
                        }
                    }
                }
                Brand::Qubo => {
                    let handle = Arc::new(CameraHandle::new(record, None));
                    manager.insert(handle);
                }
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
        self.handles().iter().map(|h| self.api(h)).collect()
    }

    pub fn camera(&self, id: &str) -> ApiResult<Camera> {
        let handle = self.get(id)?;
        Ok(self.api(&handle))
    }

    fn api(&self, handle: &CameraHandle) -> Camera {
        handle.to_api(self.previews.saved_at(&handle.id))
    }

    /// Stores the camera's latest picture (a JPEG from the player) as its preview.
    pub fn save_preview(&self, id: &str, jpeg: &[u8]) -> ApiResult<()> {
        self.get(id)?;
        let saved_at = self.previews.save(id, jpeg)?;
        self.emit(AppEvent::CameraPreview {
            camera_id: id.to_owned(),
            snapshot_url: previews::url(id, saved_at),
            snapshot_at: rfc3339_ms(saved_at),
        });
        Ok(())
    }

    pub fn preview(&self, id: &str) -> Option<Vec<u8>> {
        self.previews.read(id)
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
        match handle.brand() {
            Brand::Tapo => self.refresh_tapo(handle).await,
            Brand::Qubo => self.refresh_qubo(handle).await,
        }
    }

    async fn refresh_qubo(&self, handle: &CameraHandle) {
        let status = match self.qubo.devices().await {
            // The account answers and the camera is still on it: that is all a
            // cloud poll can know. Storage and clock live on the camera's SD card,
            // which the cloud doesn't expose.
            Ok(devices)
                if devices
                    .iter()
                    .any(|d| d.device_uuid == handle.device_uuid()) =>
            {
                CameraStatus {
                    state: CameraState::Online,
                    last_seen: Some(now_rfc3339()),
                    ..CameraStatus::new(CameraState::Online)
                }
            }
            Ok(_) => CameraStatus {
                state: CameraState::Offline,
                message: Some("The camera is no longer on the Qubo account.".into()),
                ..CameraStatus::new(CameraState::Offline)
            },
            Err(err) => CameraStatus {
                state: match err.code {
                    "auth_failed" => CameraState::AuthFailed,
                    _ => CameraState::Offline,
                },
                message: Some(err.message),
                ..CameraStatus::new(CameraState::Offline)
            },
        };
        let last_seen = handle.status().last_seen;
        let mut status = status;
        if status.state != CameraState::Online {
            status.last_seen = last_seen;
        }
        self.set_status(handle, status);
    }

    async fn refresh_tapo(&self, handle: &CameraHandle) {
        // Dispatched by `refresh`, which only sends Tapo cameras here.
        let client = handle
            .tapo_client()
            .expect("refresh_tapo is only called for Tapo cameras");
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
            (
                "getRecordPlan",
                serde_json::json!({ "record_plan": { "name": ["chn1_channel"] } }),
            ),
        ];
        match client.execute_many(&calls).await {
            Ok(results) => {
                let mut info = handle.info.read().expect("info lock").clone();
                let shown = |info: &LiveInfo| {
                    (
                        info.storage.clone(),
                        info.clock.map(|c| c.utc_offset_minutes),
                    )
                };
                let before = shown(&info);
                if let Some(Ok(clock)) = results.first() {
                    info.clock =
                        parse_clock(clock, jiff::Timestamp::now().as_second()).or(info.clock);
                }
                if let Some(Ok(sd)) = results.get(1) {
                    info.storage = parse_storage(sd);
                }
                if let (Some(storage), Some(Ok(plan))) = (info.storage.as_mut(), results.get(3)) {
                    storage.recording_mode = parse_record_plan(plan);
                }
                let privacy = results
                    .get(2)
                    .and_then(|r| r.as_ref().ok())
                    .and_then(|v| v.pointer("/lens_mask/lens_mask_info/enabled"))
                    .and_then(Value::as_str)
                    == Some("on");
                let after = shown(&info);
                *handle.info.write().expect("info lock") = info;
                let mut status = CameraStatus::new(if privacy {
                    CameraState::Privacy
                } else {
                    CameraState::Online
                });
                status.last_seen = Some(now_rfc3339());
                self.set_status(handle, status);
                if after != before {
                    let (storage, utc_offset_minutes) = after;
                    self.emit(AppEvent::CameraInfo {
                        camera_id: handle.id.clone(),
                        storage,
                        utc_offset_minutes,
                    });
                }
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
            brand: Brand::Tapo,
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

        let handle = Arc::new(CameraHandle::new(record, Some(client)));
        self.insert(handle.clone());
        self.refresh(&handle).await;
        self.emit(AppEvent::CamerasChanged);
        Ok(self.api(&handle))
    }

    /// Signs in to the Qubo account and lists its cameras. Storing the account here
    /// is what "adding" a Qubo camera means: the cameras themselves are only
    /// references into the account.
    pub async fn qubo_devices(
        &self,
        account: &crate::model::CameraAccount,
    ) -> ApiResult<Vec<QuboCloudDevice>> {
        let devices = self.qubo.sign_in(account).await?;
        let known: Vec<CameraRecord> = self.handles().iter().map(|h| h.record()).collect();
        Ok(devices
            .into_iter()
            .filter(is_qubo_camera)
            .map(|d| QuboCloudDevice {
                already_added: known.iter().any(|r| r.host == d.device_uuid),
                device_uuid: d.device_uuid,
                name: (!d.device_name.is_empty()).then_some(d.device_name),
                model: Some(d.device_type),
            })
            .collect())
    }

    pub async fn add_qubo(&self, request: AddQuboCameraRequest) -> ApiResult<Camera> {
        let device_uuid = request.device_uuid.trim().to_owned();
        if device_uuid.is_empty() {
            return Err(ApiError::invalid("Pick a camera from the account."));
        }
        if self
            .handles()
            .iter()
            .any(|h| h.brand() == Brand::Qubo && h.device_uuid() == device_uuid)
        {
            return Err(ApiError::invalid("This camera is already added."));
        }
        let Some(device) = self
            .qubo
            .devices()
            .await?
            .into_iter()
            .find(|d| d.device_uuid == device_uuid)
        else {
            return Err(ApiError::not_found("qubo camera"));
        };
        if !is_qubo_camera(&device) {
            return Err(ApiError::invalid("This Qubo device is not a camera."));
        }

        let record = CameraRecord {
            id: random_id(),
            name: request
                .name
                .filter(|n| !n.trim().is_empty())
                .or_else(|| (!device.device_name.is_empty()).then_some(device.device_name.clone()))
                .unwrap_or_else(|| device.device_type.clone()),
            brand: Brand::Qubo,
            host: device.device_uuid.clone(),
            model: Some(device.device_type),
            firmware: None,
            mac: device.mac_address,
            group_ids: request.group_ids.unwrap_or_default(),
            favorite: false,
            has_camera_account: false,
            cert_pin: None,
            created_at: now_rfc3339(),
        };
        self.db.upsert_camera(&record)?;

        let handle = Arc::new(CameraHandle::new(record, None));
        self.insert(handle.clone());
        self.refresh(&handle).await;
        self.emit(AppEvent::CamerasChanged);
        Ok(self.api(&handle))
    }

    pub async fn update(&self, id: &str, request: UpdateCameraRequest) -> ApiResult<Camera> {
        let handle = self.get(id)?;
        let mut record = handle.record();

        if let Some(password) = request.cloud_password.filter(|p| !p.is_empty()) {
            match record.brand {
                Brand::Tapo => {
                    // Verify before replacing the working password.
                    let client = client_for(&record, password.clone())?;
                    client.login().await?;
                    secrets::set_cloud_password(id, &password)?;
                    *handle.client.write().expect("client lock") = Some(Arc::new(client));
                    *handle.user_id.lock().await = None;
                }
                Brand::Qubo => {
                    // The Qubo account is shared; verify it before replacing it.
                    let account = crate::model::CameraAccount {
                        username: secrets::qubo_account()
                            .map(|a| a.username)
                            .unwrap_or_default(),
                        password,
                    };
                    self.qubo.sign_in(&account).await?;
                }
            }
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
        match (record.brand, request.camera_account) {
            (Brand::Tapo, Some(Some(account))) => {
                secrets::set_camera_account(id, &account)?;
                record.has_camera_account = true;
            }
            (Brand::Tapo, Some(None)) => {
                secrets::delete_camera_account(id);
                record.has_camera_account = false;
            }
            _ => {}
        }

        self.db.upsert_camera(&record)?;
        *handle.record.write().expect("record lock") = record;
        self.refresh(&handle).await;
        self.emit(AppEvent::CamerasChanged);
        Ok(self.api(&handle))
    }

    pub async fn remove(&self, id: &str) -> ApiResult<()> {
        let qubo = self.get(id)?.brand() == Brand::Qubo;
        self.db.delete_camera(id)?;
        secrets::delete_all(id);
        self.previews.remove(id);
        self.cameras.write().expect("cameras lock").remove(id);
        if qubo && !self.handles().iter().any(|h| h.brand() == Brand::Qubo) {
            self.qubo.forget().await;
        }
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

/// Reads `getRecordPlan`: a weekly schedule of `"HHMM-HHMM:type"` slots per day, where
/// type 1 is continuous ("timed") recording and 2 is recording on detection.
pub fn parse_record_plan(value: &Value) -> Option<String> {
    let plan = value.pointer("/record_plan/chn1_channel")?;
    if plan.get("enabled").and_then(Value::as_str) != Some("on") {
        return Some("off".into());
    }
    let days = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ];
    let slots: String = days
        .iter()
        .filter_map(|day| plan.get(*day).and_then(Value::as_str))
        .collect();
    Some(if slots.contains(":1") {
        "continuous".into()
    } else if slots.contains(":2") {
        "detection".into()
    } else {
        "off".into()
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
    } else if let Some(n) = text.strip_suffix('B') {
        (n, 1)
    } else {
        (text.as_str(), 1u64 << 20)
    };
    number
        .trim()
        .parse::<f64>()
        .ok()
        .map(|n| (n * multiplier as f64) as u64)
}

/// Reads `getSdCardStatus` → `harddisk_manage.hd_info`, a list of single-key objects. `None`
/// means the reply didn't say; an empty list means there is no card.
pub fn parse_storage(value: &Value) -> Option<StorageInfo> {
    let list = value.pointer("/harddisk_manage/hd_info")?.as_array()?;
    let Some(disk) = list
        .iter()
        .find_map(|item| item.as_object()?.values().next())
    else {
        return Some(StorageInfo {
            present: false,
            status: "none".into(),
            total_bytes: 0,
            free_bytes: 0,
            recording_mode: None,
            loop_recording: false,
        });
    };
    let text = |key: &str| disk.get(key).and_then(Value::as_str).unwrap_or("");
    let status = match text("status") {
        "insufficient" | "full" => "full",
        "unformatted" => "unformatted",
        "abnormal" | "error" => "error",
        "" | "offline" => "none",
        _ => "normal",
    };
    // Prefer the exact byte counts ("244007829504B") over the rounded ones ("227.3GB").
    let size = |key: &str| {
        disk.get(format!("{key}_accurate"))
            .and_then(parse_size)
            .or_else(|| disk.get(key).and_then(parse_size))
            .unwrap_or(0)
    };
    Some(StorageInfo {
        present: status != "none",
        status: status.into(),
        total_bytes: size("total_space"),
        free_bytes: size("free_space"),
        recording_mode: None,
        loop_recording: text("loop_record_status") == "1",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn qubo_device(kind: &str) -> crate::qubo::Device {
        crate::qubo::Device {
            device_uuid: "d".into(),
            device_type: kind.into(),
            device_name: String::new(),
            mac_address: None,
            unit_uuid: String::new(),
        }
    }

    #[test]
    fn qubo_camera_kinds() {
        // The Smart Cam 360 3MP reports as a ptz camera.
        assert!(is_qubo_camera(&qubo_device("ptzCamera3MP")));
        assert!(is_qubo_camera(&qubo_device("cam3602K4MP")));
        assert!(is_qubo_camera(&qubo_device("videoDoorbell")));
        // Phones and this app are not cameras.
        assert!(!is_qubo_camera(&qubo_device("mobile")));
        assert!(!is_qubo_camera(&qubo_device("smartPlugWifi10A")));
    }

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
    fn record_plan_modes() {
        // A C325WB set to record continuously all week.
        let plan = json!({ "record_plan": { "chn1_channel": {
            "enabled": "on", "monday": "[\"0000-2400:1\"]", "sunday": "[\"0000-2400:1\"]"
        } } });
        assert_eq!(parse_record_plan(&plan).as_deref(), Some("continuous"));
        let plan = json!({ "record_plan": { "chn1_channel": { "enabled": "on", "monday": "[\"0000-2400:2\"]" } } });
        assert_eq!(parse_record_plan(&plan).as_deref(), Some("detection"));
        let plan = json!({ "record_plan": { "chn1_channel": { "enabled": "off" } } });
        assert_eq!(parse_record_plan(&plan).as_deref(), Some("off"));
    }

    #[test]
    fn storage_prefers_exact_sizes() {
        // Trimmed from a C325WB's getSdCardStatus.
        let sd = json!({ "harddisk_manage": { "hd_info": [ { "hd_info_1": {
            "status": "normal", "detect_status": "normal", "loop_record_status": "1",
            "total_space": "227.3GB", "total_space_accurate": "244007829504B",
            "free_space": "37.2MB", "free_space_accurate": "38987620B"
        } } ] } });
        let info = parse_storage(&sd).unwrap();
        assert_eq!(info.total_bytes, 244_007_829_504);
        assert_eq!(info.free_bytes, 38_987_620);
        assert_eq!(info.status, "normal");
        assert!(info.loop_recording);
    }

    #[test]
    fn storage_empty_slot_and_unknown() {
        let empty = json!({ "harddisk_manage": { "hd_info": [] } });
        let info = parse_storage(&empty).unwrap();
        assert!(!info.present);
        assert_eq!(info.status, "none");
        assert!(parse_storage(&json!({ "harddisk_manage": {} })).is_none());
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
