//! Camera passwords, kept in the OS credential store (Windows Credential Manager,
//! macOS Keychain, Secret Service on Linux) and never in the app's database or logs.

use keyring::v1::Entry;

use crate::error::{ApiError, ApiResult};
use crate::model::CameraAccount;
use crate::qubo::Tokens;

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

/// The Qubo account, shared by every Qubo camera (there is one cloud account, not
/// one credential per camera).
const QUBO_ACCOUNT: &str = "qubo/account";
/// The Qubo session tokens, so the account survives restarts without signing in.
const QUBO_TOKENS: &str = "qubo/tokens";

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

pub fn set_qubo_account(account: &CameraAccount) -> ApiResult<()> {
    let json = serde_json::to_string(account).expect("JSON");
    entry(QUBO_ACCOUNT)?
        .set_password(&json)
        .map_err(|e| ApiError::internal(format!("could not save the Qubo account: {e}")))
}

pub fn qubo_account() -> Option<CameraAccount> {
    let json = entry(QUBO_ACCOUNT).ok()?.get_password().ok()?;
    serde_json::from_str(&json).ok()
}

pub fn set_qubo_tokens(tokens: &Tokens) -> ApiResult<()> {
    let json = serde_json::to_string(tokens).expect("JSON");
    entry(QUBO_TOKENS)?
        .set_password(&json)
        .map_err(|e| ApiError::internal(format!("could not save the Qubo session: {e}")))
}

pub fn qubo_tokens() -> Option<Tokens> {
    let json = entry(QUBO_TOKENS).ok()?.get_password().ok()?;
    serde_json::from_str(&json).ok()
}

pub fn delete_qubo_account() {
    for account in [QUBO_ACCOUNT, QUBO_TOKENS] {
        if let Ok(entry) = entry(account) {
            let _ = entry.delete_credential();
        }
    }
}

/// Removes every secret stored for a camera.
pub fn delete_all(camera_id: &str) {
    if let Ok(entry) = entry(&cloud_account(camera_id)) {
        let _ = entry.delete_credential();
    }
    delete_camera_account(camera_id);
}
