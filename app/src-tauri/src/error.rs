//! Errors as the UI sees them: `{ code, message, retryAt? }` (see `ApiError` in
//! `app/src/ipc/api.ts`).

use serde::Serialize;

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<String>,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retry_at: None,
        }
    }

    pub fn not_found(what: &str) -> Self {
        Self::new("not_found", format!("{what} not found"))
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_input", message)
    }

    pub fn internal(message: impl std::fmt::Display) -> Self {
        Self::new("internal", message.to_string())
    }
}

impl From<tapo_camera::Error> for ApiError {
    fn from(error: tapo_camera::Error) -> Self {
        use tapo_camera::Error as E;
        let message = error.to_string();
        match &error {
            E::BadCredentials => Self::new(
                "auth_failed",
                "The camera rejected the password. Use the TP-Link account password of the \
                 camera's owner, and check that Third-Party Compatibility is on in the Tapo app.",
            ),
            E::NotOwnerAccount => Self::new("auth_failed", message),
            E::Locked { retry_after } => {
                let retry_at = jiff::Timestamp::now()
                    .checked_add(jiff::SignedDuration::try_from(*retry_after).unwrap_or_default())
                    .unwrap_or_else(|_| jiff::Timestamp::now());
                Self {
                    code: "camera_locked",
                    message,
                    retry_at: Some(retry_at.to_string()),
                }
            }
            E::UnsupportedLogin(_) => Self::new("unsupported", message),
            E::Http(http) if http.is_connect() || http.is_timeout() => Self::new(
                "offline",
                "The camera didn't answer. Check that it is powered on and on the same network.",
            ),
            E::Http(_) => Self::new("offline", message),
            E::Camera { code, .. } => match code {
                -64324 => Self::new("privacy_mode", "Privacy mode is on for this camera."),
                -71101 | -71102 | -52417 | -52435 => Self::new(
                    "playback_busy",
                    "Another app or phone is already watching this camera's recordings.",
                ),
                -52405 | -52407 | -52419 => Self::new(
                    "stream_limit",
                    "The camera has too many viewers right now (NVR software counts too).",
                ),
                -40404 => Self::new("camera_locked", message),
                _ => Self::new("internal", message),
            },
            _ => Self::new("internal", message),
        }
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        Self::internal(format!("database error: {error}"))
    }
}
