//! Errors returned by the camera client.

use std::time::Duration;

/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong talking to a camera.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Network or HTTP failure.
    #[error("could not reach the camera: {0}")]
    Http(#[from] reqwest::Error),

    /// The TLS configuration could not be built.
    #[error("TLS setup failed: {0}")]
    Tls(String),

    /// The camera presented a different certificate than the one pinned on first use.
    #[error(
        "the camera's certificate changed (pinned {expected}, got {actual}); refusing to connect"
    )]
    CertificateMismatch { expected: String, actual: String },

    /// The camera rejected the password. Never retried automatically: repeated failures
    /// lock the camera for about 30 minutes.
    #[error("the camera rejected the password")]
    BadCredentials,

    /// The account is not the camera owner's; the encrypted local API only accepts the
    /// owner account (user `admin` with the TP-Link account password).
    #[error("this account can't use the camera's local API; use the owner's TP-Link account")]
    NotOwnerAccount,

    /// Too many failed logins; the camera refuses logins for a while.
    #[error("the camera is refusing logins for another {} s after too many failed attempts", retry_after.as_secs())]
    Locked { retry_after: Duration },

    /// The camera speaks a login scheme this crate doesn't implement yet.
    #[error("unsupported login scheme: {0}")]
    UnsupportedLogin(String),

    /// The camera answered a request with an error code.
    #[error("camera error {code} ({}){}", describe_code(*code).unwrap_or("unknown"), method.as_deref().map(|m| format!(" in {m}")).unwrap_or_default())]
    Camera { code: i64, method: Option<String> },

    /// The response didn't have the expected shape.
    #[error("unexpected response from the camera: {0}")]
    Protocol(String),
}

impl Error {
    pub(crate) fn protocol(msg: impl Into<String>) -> Self {
        Self::Protocol(msg.into())
    }

    /// The camera error code, if this is a camera error.
    pub fn camera_code(&self) -> Option<i64> {
        match self {
            Self::Camera { code, .. } => Some(*code),
            _ => None,
        }
    }
}

/// Human-readable names for the error codes Tapo cameras are known to return.
/// Collected by the pytapo project.
pub fn describe_code(code: i64) -> Option<&'static str> {
    Some(match code {
        -1 => "common failure (e.g. tag check failed)",
        -40101 => "parameter does not exist",
        -40105 => "method does not exist",
        -40106 => "unsupported method",
        -40109 => "repeated request within one second",
        -40209 => "invalid login credentials",
        -40210 => "method does not exist / protocol format error",
        -40211 => "missing parameters (newer login scheme required)",
        -40401 => "session expired",
        -40404 => "device blocked after failed logins",
        -40405 => "device in factory state",
        -40406 => "out of limit",
        -40408 => "system blocked",
        -40409 => "nonce expired",
        -40411 => "invalid authentication data",
        -40412 => "HomeKit login failed",
        -40413 => "invalid nonce",
        -40414 => "must log in with the local password",
        -40418 => "TPAP authentication failed",
        -40421 => "TPAP session token invalid",
        -52402 => "invalid playback request",
        -52405 => "too many requests / device in use",
        -52407 => "too many clients",
        -52409 => "SD card unplugged",
        -52411 => "two-way talk already in use",
        -52417 => "playback session occupied",
        -52419 => "too many HTTPS clients",
        -52422 => "SD card unusable",
        -52435 => "playback sessions full",
        -64302 => "preset not found",
        -64303 => "motor busy",
        -64324 => "privacy mode is on",
        -71101 => "playback user slots full",
        -71102 => "playback user id already in use",
        -71103 => "playback user id invalid",
        -71105 => "playback search failed",
        -71114 => "storage does not exist",
        -71115 => "SD card still loading",
        _ => return None,
    })
}
