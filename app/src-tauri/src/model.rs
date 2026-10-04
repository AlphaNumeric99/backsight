//! Types exchanged with the UI. They mirror `app/src/ipc/api.ts` exactly (camelCase
//! JSON); change both together.

use serde::{Deserialize, Deserializer, Serialize};

pub type CameraId = String;

/// Which protocol a camera speaks, and therefore how it is reached: Tapo cameras
/// are on the LAN, Qubo cameras live behind the vendor's cloud.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Brand {
    #[default]
    Tapo,
    Qubo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraState {
    Online,
    Connecting,
    Offline,
    Privacy,
    Locked,
    AuthFailed,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraStatus {
    pub state: CameraState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
}

impl CameraStatus {
    pub fn new(state: CameraState) -> Self {
        Self {
            state,
            message: None,
            locked_until: None,
            last_seen: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    pub present: bool,
    /// "normal" | "unformatted" | "full" | "error" | "none"
    pub status: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_mode: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub loop_recording: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Camera {
    pub id: CameraId,
    pub name: String,
    pub brand: Brand,
    /// The camera's address on the LAN, or the cloud device id for Qubo cameras.
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    pub group_ids: Vec<String>,
    pub favorite: bool,
    pub has_camera_account: bool,
    pub status: CameraStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<StorageInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utc_offset_minutes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_url: Option<String>,
    /// When the preview at `snapshot_url` was captured (RFC 3339, UTC).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraGroup {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredDevice {
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware: Option<String>,
    /// "camera" | "doorbell" | "hub" | "other"
    pub kind: String,
    /// "legacy" | "secure" | "tpap" | "unknown"
    pub login_scheme: String,
    pub already_added: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraAccount {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddCameraRequest {
    pub host: String,
    pub name: Option<String>,
    pub cloud_password: String,
    pub camera_account: Option<CameraAccount>,
    pub group_ids: Option<Vec<String>>,
}

/// A camera of the signed-in Qubo account, as offered when adding one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuboCloudDevice {
    pub device_uuid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The cloud's device type, e.g. `"ptzCamera3MP"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub already_added: bool,
}

/// Adds a camera of the already-signed-in Qubo account.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddQuboCameraRequest {
    pub device_uuid: String,
    pub name: Option<String>,
    pub group_ids: Option<Vec<String>>,
}

/// Distinguishes a missing field (`None`) from an explicit `null` (`Some(None)`).
fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCameraRequest {
    pub name: Option<String>,
    pub group_ids: Option<Vec<String>>,
    pub favorite: Option<bool>,
    pub cloud_password: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub camera_account: Option<Option<CameraAccount>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingKind {
    Continuous,
    Detection,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecordingSegment {
    pub start: String,
    pub end: String,
    pub kind: RecordingKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionEvent {
    pub id: String,
    pub start: String,
    pub end: String,
    pub types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayIndex {
    pub camera_id: CameraId,
    pub date: String,
    pub segments: Vec<RecordingSegment>,
    pub events: Vec<DetectionEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamQuality {
    Hd,
    Sd,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StreamRequest {
    Live {
        camera_id: CameraId,
        quality: StreamQuality,
    },
    Playback {
        camera_id: CameraId,
        start: String,
        speed: f64,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub camera_id: CameraId,
    pub start: String,
    pub end: String,
    pub output_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportState {
    Queued,
    Running,
    Paused,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportJob {
    pub id: String,
    pub camera_id: CameraId,
    pub camera_name: String,
    pub start: String,
    pub end: String,
    pub state: ExportState,
    pub progress: f64,
    pub bytes_written: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// "system" | "light" | "dark"
    pub theme: String,
    pub export_dir: String,
    pub export_name_template: String,
    pub cache_limit_mb: u64,
    pub default_live_quality: StreamQuality,
    /// "1" | "2" | "4" | "1+5" | "9" | "16"
    pub multiview_layout: String,
    pub multiview_order: Vec<CameraId>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum AppEvent {
    CameraStatus {
        camera_id: CameraId,
        status: CameraStatus,
    },
    CamerasChanged,
    /// What the status poll reads besides the state: SD card and clock.
    CameraInfo {
        camera_id: CameraId,
        #[serde(skip_serializing_if = "Option::is_none")]
        storage: Option<StorageInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        utc_offset_minutes: Option<i32>,
    },
    /// A new preview was saved for the camera.
    CameraPreview {
        camera_id: CameraId,
        snapshot_url: String,
        snapshot_at: String,
    },
    ExportProgress {
        job: ExportJob,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn camera_state_and_events_match_the_ui_contract() {
        let event = AppEvent::CameraStatus {
            camera_id: "c1".into(),
            status: CameraStatus::new(CameraState::AuthFailed),
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({ "type": "camera-status", "cameraId": "c1", "status": { "state": "auth_failed" } })
        );
        assert_eq!(
            serde_json::to_value(AppEvent::CamerasChanged).unwrap(),
            json!({ "type": "cameras-changed" })
        );
        let info = AppEvent::CameraInfo {
            camera_id: "c1".into(),
            storage: None,
            utc_offset_minutes: Some(330),
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            json!({ "type": "camera-info", "cameraId": "c1", "utcOffsetMinutes": 330 })
        );
        let preview = AppEvent::CameraPreview {
            camera_id: "c1".into(),
            snapshot_url: "thumb://localhost/preview/c1/1".into(),
            snapshot_at: "2026-09-29T06:00:00Z".into(),
        };
        assert_eq!(
            serde_json::to_value(&preview).unwrap(),
            json!({
                "type": "camera-preview",
                "cameraId": "c1",
                "snapshotUrl": "thumb://localhost/preview/c1/1",
                "snapshotAt": "2026-09-29T06:00:00Z"
            })
        );
    }

    #[test]
    fn cameras_carry_their_brand() {
        // The brand decides what the UI hides (playback, exports) and which
        // secondary line the cards show.
        let camera = Camera {
            id: "c1".into(),
            name: "Gate".into(),
            brand: Brand::Qubo,
            host: "48ae90fc".into(),
            model: None,
            firmware: None,
            mac: None,
            group_ids: vec![],
            favorite: false,
            has_camera_account: false,
            status: CameraStatus::new(CameraState::Online),
            storage: None,
            video_codec: None,
            utc_offset_minutes: None,
            time_zone: None,
            snapshot_url: None,
            snapshot_at: None,
        };
        assert_eq!(
            serde_json::to_value(&camera).unwrap().get("brand").unwrap(),
            &json!("qubo")
        );
        assert_eq!(serde_json::to_value(Brand::Tapo).unwrap(), json!("tapo"));
    }

    #[test]
    fn add_qubo_request_parses() {
        let request: AddQuboCameraRequest =
            serde_json::from_value(json!({ "deviceUuid": "d", "name": "Living room" })).unwrap();
        assert_eq!(request.device_uuid, "d");
        assert_eq!(request.name.as_deref(), Some("Living room"));
        assert_eq!(request.group_ids, None);
    }

    #[test]
    fn stream_request_parses() {
        let live: StreamRequest =
            serde_json::from_value(json!({ "kind": "live", "cameraId": "c1", "quality": "sd" }))
                .unwrap();
        assert!(matches!(
            live,
            StreamRequest::Live {
                quality: StreamQuality::Sd,
                ..
            }
        ));
        let playback: StreamRequest = serde_json::from_value(
            json!({ "kind": "playback", "cameraId": "c1", "start": "2026-09-29T08:00:00Z", "speed": 2 }),
        )
        .unwrap();
        assert!(matches!(playback, StreamRequest::Playback { speed, .. } if speed == 2.0));
    }

    #[test]
    fn update_request_distinguishes_null_from_missing() {
        let missing: UpdateCameraRequest =
            serde_json::from_value(json!({ "name": "Gate" })).unwrap();
        assert!(missing.camera_account.is_none());
        let null: UpdateCameraRequest =
            serde_json::from_value(json!({ "cameraAccount": null })).unwrap();
        assert!(matches!(null.camera_account, Some(None)));
    }
}
