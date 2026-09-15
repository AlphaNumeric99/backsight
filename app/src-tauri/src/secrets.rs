//! Camera passwords, kept in the OS credential store (Windows Credential Manager,
//! macOS Keychain, Secret Service on Linux) and never in the app's database or logs.

use keyring::v1::Entry;

use crate::error::{ApiError, ApiResult};
use crate::model::CameraAccount;

const SERVICE: &str = "Backsight";

fn entry(account: &str) -> ApiResult<Entry> {
    Entry::new(SERVICE, account)
        .map_err(|e| ApiError::internal(format!("credential store unavailable: {e}")))
}

fn cloud_account(camera_id: &str) -> String {
    format!("{camera_id}/owner")
}

fn rtsp_account(camera_id: &str) -> String {
    format!("{camera_id}/camera-account")
}

pub fn set_cloud_password(camera_id: &str, password: &str) -> ApiResult<()> {
    entry(&cloud_account(camera_id))?
        .set_password(password)
        .map_err(|e| ApiError::internal(format!("could not save the password: {e}")))
}

pub fn cloud_password(camera_id: &str) -> Option<String> {
    entry(&cloud_account(camera_id)).ok()?.get_password().ok()
}

pub fn set_camera_account(camera_id: &str, account: &CameraAccount) -> ApiResult<()> {
    let json = serde_json::to_string(account).expect("JSON");
    entry(&rtsp_account(camera_id))?
        .set_password(&json)
        .map_err(|e| ApiError::internal(format!("could not save the camera account: {e}")))
}

#[allow(dead_code)] // for the RTSP live source (v1.x)
pub fn camera_account(camera_id: &str) -> Option<CameraAccount> {
    let json = entry(&rtsp_account(camera_id)).ok()?.get_password().ok()?;
    serde_json::from_str(&json).ok()
}

pub fn delete_camera_account(camera_id: &str) {
    if let Ok(entry) = entry(&rtsp_account(camera_id)) {
        let _ = entry.delete_credential();
    }
}

/// Removes every secret stored for a camera.
pub fn delete_all(camera_id: &str) {
    if let Ok(entry) = entry(&cloud_account(camera_id)) {
        let _ = entry.delete_credential();
    }
    delete_camera_account(camera_id);
}
