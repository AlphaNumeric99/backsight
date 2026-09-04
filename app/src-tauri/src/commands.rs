//! Tauri commands behind `app/src/ipc/tauri.ts`. Argument names are camelCase on the JS
//! side (Tauri converts them).

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tauri::{Manager, State};

use crate::cameras::CameraManager;
use crate::db::Db;
use crate::error::{ApiError, ApiResult};
use crate::model::{
    AddCameraRequest, Camera, CameraGroup, DayIndex, DiscoveredDevice, ExportJob, Settings,
    StreamQuality, UpdateCameraRequest,
};
use crate::recordings;

pub struct AppState {
    pub db: Arc<Db>,
    pub cameras: Arc<CameraManager>,
    pub thumbnails: Arc<crate::thumbnails::Thumbnails>,
    pub default_export_dir: String,
}

const SETTINGS_KEY: &str = "settings";

impl AppState {
    pub fn settings(&self) -> ApiResult<Settings> {
        Ok(self.db.setting(SETTINGS_KEY)?.unwrap_or_else(|| Settings {
            theme: "system".into(),
            export_dir: self.default_export_dir.clone(),
            export_name_template: "{camera} {date} {start}-{end}".into(),
            cache_limit_mb: 2048,
            default_live_quality: StreamQuality::Hd,
            multiview_layout: "4".into(),
            multiview_order: Vec::new(),
        }))
    }
}

pub fn default_export_dir(app: &tauri::AppHandle) -> String {
    app.path()
        .video_dir()
        .or_else(|_| app.path().download_dir())
        .map(|dir| dir.join("Backsight").to_string_lossy().into_owned())
        .unwrap_or_else(|_| "Backsight".into())
}

#[tauri::command]
pub async fn list_cameras(state: State<'_, AppState>) -> ApiResult<Vec<Camera>> {
    Ok(state.cameras.list())
}

#[tauri::command]
pub async fn get_camera(state: State<'_, AppState>, id: String) -> ApiResult<Camera> {
    Ok(state.cameras.get(&id)?.to_api())
}

#[tauri::command]
pub async fn discover(
    state: State<'_, AppState>,
    timeout_ms: Option<u64>,
) -> ApiResult<Vec<DiscoveredDevice>> {
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(4000).clamp(500, 15_000));
    state.cameras.discover(timeout).await
}

#[tauri::command]
pub async fn add_camera(state: State<'_, AppState>, req: AddCameraRequest) -> ApiResult<Camera> {
    state.cameras.add(req).await
}

#[tauri::command]
pub async fn update_camera(
    state: State<'_, AppState>,
    id: String,
    req: UpdateCameraRequest,
) -> ApiResult<Camera> {
    state.cameras.update(&id, req).await
}

#[tauri::command]
pub async fn remove_camera(state: State<'_, AppState>, id: String) -> ApiResult<()> {
    state.cameras.remove(&id)
}

#[tauri::command]
pub async fn list_groups(state: State<'_, AppState>) -> ApiResult<Vec<CameraGroup>> {
    Ok(state.db.groups()?)
}

#[tauri::command]
pub async fn save_groups(state: State<'_, AppState>, groups: Vec<CameraGroup>) -> ApiResult<()> {
    Ok(state.db.save_groups(&groups)?)
}

#[tauri::command]
pub async fn get_days_with_recordings(
    state: State<'_, AppState>,
    camera_id: String,
    month: String,
) -> ApiResult<Vec<String>> {
    let handle = state.cameras.get(&camera_id)?;
    recordings::days_with_recordings(&handle, &month).await
}

#[tauri::command]
pub async fn get_day_index(
    state: State<'_, AppState>,
    camera_id: String,
    date: String,
) -> ApiResult<DayIndex> {
    let handle = state.cameras.get(&camera_id)?;
    recordings::day_index(&handle, &state.db, &date).await
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> ApiResult<Settings> {
    state.settings()
}

#[tauri::command]
pub async fn update_settings(state: State<'_, AppState>, patch: Value) -> ApiResult<Settings> {
    let mut current = serde_json::to_value(state.settings()?).expect("JSON");
    let (Value::Object(current_map), Value::Object(patch_map)) = (&mut current, patch) else {
        return Err(ApiError::invalid("settings patch must be an object"));
    };
    for (key, value) in patch_map {
        if current_map.contains_key(&key) {
            current_map.insert(key, value);
        }
    }
    let settings: Settings = serde_json::from_value(current)
        .map_err(|e| ApiError::invalid(format!("invalid settings: {e}")))?;
    state.db.set_setting(SETTINGS_KEY, &settings)?;
    Ok(settings)
}

#[tauri::command]
pub async fn list_exports(state: State<'_, AppState>) -> ApiResult<Vec<ExportJob>> {
    Ok(state.db.exports()?)
}

/// Makes a string safe to use as a file name on every platform.
pub fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').to_owned();
    if trimmed.is_empty() {
        "camera".into()
    } else {
        trimmed
    }
}

/// Saves a PNG snapshot (raw request body; camera id in the `x-camera-id` header) to
/// `Pictures/Backsight` and returns the file path.
#[tauri::command]
pub async fn save_snapshot(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> ApiResult<String> {
    let tauri::ipc::InvokeBody::Raw(png) = request.body() else {
        return Err(ApiError::invalid("expected the PNG as the request body"));
    };
    if !png.starts_with(b"\x89PNG") {
        return Err(ApiError::invalid("the snapshot is not a PNG image"));
    }
    let camera_id = request
        .headers()
        .get("x-camera-id")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::invalid("missing x-camera-id header"))?;
    let handle = state.cameras.get(camera_id)?;

    let dir = app
        .path()
        .picture_dir()
        .map_err(ApiError::internal)?
        .join("Backsight");
    std::fs::create_dir_all(&dir).map_err(ApiError::internal)?;
    let stamp = jiff::Zoned::now().strftime("%Y-%m-%d %H-%M-%S").to_string();
    let path = dir.join(format!(
        "{} {stamp}.png",
        sanitize_file_name(&handle.record().name)
    ));
    std::fs::write(&path, png).map_err(ApiError::internal)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Shows an exported file in the OS file manager.
#[tauri::command]
pub async fn reveal_export(state: State<'_, AppState>, id: String) -> ApiResult<()> {
    let job = state
        .db
        .exports()?
        .into_iter()
        .find(|job| job.id == id)
        .ok_or_else(|| ApiError::not_found("export"))?;
    let path = job
        .output_path
        .ok_or_else(|| ApiError::invalid("this export has no file yet"))?;
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(ApiError::internal)
}
