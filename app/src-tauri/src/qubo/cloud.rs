//! The Qubo cloud REST API (srvcapp.platform.quboworld.com).
//!
//! Every request carries the Android app's identity headers; a session is the
//! `Subscriber-Key` / `User-UUID` header pair. Request shapes were taken from the Qubo
//! Android app (`com.hero.iot` 3.2.92):
//!
//! - sign in: `POST /sms/api/v1/sp/{sp}/user/login?system=CS` with
//!   `{"accessToken":"", "username", "password", "deviceAttribute"}` → tokens;
//! - refresh: `POST /sms/api/v1/sp/{sp}/users/{uuid}/auth/refresh` with both tokens
//!   → a new access token (the refresh token itself is not rotated);
//! - devices: `GET /unit-entity-management/api/v1/sp/{sp}/units/{unit}/devices`;
//! - live view: `POST /stream-manager/api/v1/sp/{sp}/stream/unit/{unit}/device/{device}`
//!   with `timestamp=NOW` and the stream quality → a short-lived `rtsps://` URL.
//!
//! The subscriber and app identifiers are constants of the Qubo platform, not user
//! input.

use std::time::Duration;

use base64::Engine as _;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::error::{ApiError, ApiResult};

/// Qubo's service-provider id, as sent by the app.
const SP_ID: &str = "d10e4bfb0153496e8e8bb955f7ebe413";
/// The app id of the Qubo Android app.
const APP_ID: &str = "934488E68332E88B1E0F9AF552840184955629777525A195949C0BE97DEF6455";
/// A device name the cloud accepts; it only labels this "mobile device" in listings.
const DEVICE_ATTRIBUTE: &str = "backsight_desktop";

/// The signed-in account: both tokens plus the account's user id.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub uuid: String,
    #[serde(default)]
    pub source_device_id: String,
}

impl Tokens {
    /// Seconds until the access token expires; unreadable tokens require renewal.
    ///
    /// The signature is not verified (the cloud does that on every request); this only
    /// decides whether to refresh early.
    pub fn expires_in(&self) -> i64 {
        fn exp_of(token: &str) -> Option<i64> {
            let payload = token.split('.').nth(1)?;
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .ok()?;
            let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
            value.get("exp")?.as_i64()
        }
        exp_of(&self.access_token)
            .map(|exp| exp - now_unix())
            .unwrap_or(0)
    }

    /// The same pair with a new access token.
    pub fn with_access(mut self, access_token: String) -> Self {
        self.access_token = access_token;
        self
    }
}

/// One device of the Qubo account.
///
/// `unit_uuid` is not part of the cloud's reply; it is filled in while walking the
/// account's units, so a device can be handed straight back for streaming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    #[serde(rename = "deviceUUID")]
    pub device_uuid: String,
    /// e.g. `"ptzCamera3MP"` for the Smart Cam 360, `"mobile"` for a phone or this app.
    #[serde(rename = "deviceType")]
    pub device_type: String,
    #[serde(rename = "deviceName", default)]
    pub device_name: String,
    #[serde(rename = "macAddress", default)]
    pub mac_address: Option<String>,
    /// The unit ("home") the device belongs to; filled by [`CloudClient::devices`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit_uuid: String,
}

/// A unit ("home") of the Qubo account; cameras hang off one.
#[derive(Debug, Deserialize)]
struct Unit {
    #[serde(rename = "unitUUID")]
    unit_uuid: String,
}

/// One live-view grant: the URL to play and the session the cloud tracks it under.
#[derive(Clone, Deserialize)]
pub struct StreamTicket {
    #[serde(rename = "streamURL")]
    pub stream_url: String,
    #[serde(rename = "appSessionId", default)]
    #[allow(dead_code)] // kept for symmetry with the app's own session bookkeeping
    pub app_session_id: String,
}

/// The error body the cloud returns: `{"code": "ERR_…", "message": ["…"]}`.
#[derive(Debug, Deserialize)]
struct CloudError {
    code: String,
    #[serde(default)]
    message: Vec<String>,
}

/// HTTP client for the Qubo cloud.
pub struct CloudClient {
    http: reqwest::Client,
    /// Device id used during the initial login; saved with its token pair.
    source_device_id: String,
}

fn now_unix() -> i64 {
    jiff::Timestamp::now().as_second()
}

impl CloudClient {
    pub fn new() -> ApiResult<Self> {
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(super::tls_config()?.as_ref().clone())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| ApiError::internal(format!("HTTP client: {e}")))?;
        let mut id = [0u8; 8];
        getrandom::fill(&mut id).map_err(|e| ApiError::internal(format!("random: {e}")))?;
        Ok(Self {
            http,
            source_device_id: hex::encode(id),
        })
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
        tokens: Option<&Tokens>,
    ) -> ApiResult<reqwest::Response> {
        let mut request = self
            .http
            .request(
                method,
                format!("https://srvcapp.platform.quboworld.com{path}"),
            )
            .header("Content-Type", "application/json")
            .header("App-Id", APP_ID)
            .header("Source", "ANDROID")
            .header(
                "Source-Device-Id",
                tokens
                    .filter(|t| !t.source_device_id.is_empty())
                    .map(|t| t.source_device_id.as_str())
                    .unwrap_or(&self.source_device_id),
            );
        if let Some(tokens) = tokens {
            request = request
                .header("Subscriber-Key", &tokens.access_token)
                .header("User-UUID", &tokens.uuid)
                .header("Token-Type", "USER");
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                ApiError::new(
                    "offline",
                    "The Qubo cloud didn't answer. Check the internet connection.",
                )
            } else {
                ApiError::new("offline", "The Qubo cloud request failed.")
            }
        })?;
        Ok(response)
    }

    /// Turns a non-2xx response into the UI's error shape.
    async fn failure(response: reqwest::Response) -> ApiError {
        let status = response.status();
        let error: Option<CloudError> = response.json().await.ok();
        let code = error
            .as_ref()
            .map(|e| e.code.as_str())
            .unwrap_or("")
            .to_owned();
        let message = error
            .and_then(|e| e.message.into_iter().next())
            .unwrap_or_else(|| format!("the Qubo cloud returned {status}"));
        if status.as_u16() == 401 || code.starts_with("ERR_AUTH") {
            ApiError::new(
                "auth_failed",
                "The Qubo cloud rejected the account. Sign in again.",
            )
        } else if status.is_client_error() {
            ApiError::invalid(message)
        } else {
            ApiError::internal(message)
        }
    }

    /// Signs in with the account's e-mail and password.
    pub async fn login(&self, username: &str, password: &str) -> ApiResult<Tokens> {
        let body = json!({
            "accessToken": "",
            "username": username,
            "password": password,
            "deviceAttribute": DEVICE_ATTRIBUTE,
        });
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/sms/api/v1/sp/{SP_ID}/user/login?system=CS"),
                Some(body),
                None,
            )
            .await?;
        if !response.status().is_success() {
            return Err(Self::failure(response).await);
        }
        let mut tokens: Tokens = response
            .json()
            .await
            .map_err(|e| ApiError::internal(format!("unexpected sign-in reply: {e}")))?;
        tokens.source_device_id = self.source_device_id.clone();
        Ok(tokens)
    }

    /// Exchanges the token pair for a new access token.
    pub async fn refresh(&self, tokens: &Tokens) -> ApiResult<String> {
        let body = json!({
            "accessToken": tokens.access_token,
            "refreshToken": tokens.refresh_token,
        });
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/sms/api/v1/sp/{SP_ID}/users/{}/auth/refresh", tokens.uuid),
                Some(body),
                Some(tokens),
            )
            .await?;
        if !response.status().is_success() {
            return Err(Self::failure(response).await);
        }
        #[derive(Deserialize)]
        struct Refreshed {
            #[serde(rename = "accessToken")]
            access_token: String,
        }
        let refreshed: Refreshed = response
            .json()
            .await
            .map_err(|e| ApiError::internal(format!("unexpected refresh reply: {e}")))?;
        Ok(refreshed.access_token)
    }

    /// The account's devices, across all units, each carrying its unit.
    pub async fn devices(&self, tokens: &Tokens) -> ApiResult<Vec<Device>> {
        let units: Vec<Unit> = self
            .get_json(
                &format!("/unit-entity-management/api/v1/sp/{SP_ID}/units"),
                tokens,
            )
            .await?;
        let mut devices = Vec::new();
        for unit in units {
            let mut list: Vec<Device> = self
                .get_json(
                    &format!(
                        "/unit-entity-management/api/v1/sp/{SP_ID}/units/{}/devices",
                        unit.unit_uuid
                    ),
                    tokens,
                )
                .await?;
            for device in &mut list {
                device.unit_uuid = unit.unit_uuid.clone();
            }
            devices.extend(list);
        }
        Ok(devices)
    }

    /// A short-lived live-stream URL for one camera.
    ///
    /// `quality` is one of the cloud's stream qualities (`"high"`, `"medium"`, `"low"`,
    /// `"auto"`); `self_device` is the account's mobile device that "watches".
    pub async fn stream_url(
        &self,
        tokens: &Tokens,
        device: &Device,
        self_device: &str,
        quality: &str,
    ) -> ApiResult<StreamTicket> {
        let path = format!(
            "/stream-manager/api/v1/sp/{SP_ID}/stream/unit/{}/device/{}\
             ?timestamp=NOW&type={quality}&streamSourceCamera=primary&deviceType={}",
            device.unit_uuid, device.device_uuid, device.device_type
        );
        let body = json!({ "userAppID": self_device });
        let response = self
            .request(reqwest::Method::POST, &path, Some(body), Some(tokens))
            .await?;
        if !response.status().is_success() {
            return Err(Self::failure(response).await);
        }
        response
            .json()
            .await
            .map_err(|e| ApiError::internal(format!("unexpected stream reply: {e}")))
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        tokens: &Tokens,
    ) -> ApiResult<T> {
        let response = self
            .request(reqwest::Method::GET, path, None, Some(tokens))
            .await?;
        if !response.status().is_success() {
            return Err(Self::failure(response).await);
        }
        response
            .json()
            .await
            .map_err(|e| ApiError::internal(format!("unexpected qubo reply: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> Tokens {
        Tokens {
            access_token: "header.eyJleHAiOjE3OTExMTU4MzF9.sig".into(),
            refresh_token: "refresh".into(),
            uuid: "u".into(),
            source_device_id: "viewer".into(),
        }
    }

    #[test]
    fn expires_in_reads_the_jwt_payload() {
        // exp is 2026-10-04T17:40:31Z; frozen tests are avoided, so only sanity-check.
        let tokens = tokens();
        assert!((tokens.expires_in() - (1_791_115_831 - now_unix())).abs() <= 1);
    }

    #[test]
    fn expires_in_survives_garbage() {
        let mut tokens = tokens();
        tokens.access_token = "not-a-jwt".into();
        assert_eq!(tokens.expires_in(), 0);
    }

    #[test]
    fn with_access_keeps_the_rest() {
        let tokens = tokens().with_access("new-access".into());
        assert_eq!(tokens.access_token, "new-access");
        assert_eq!(tokens.refresh_token, "refresh");
        assert_eq!(tokens.uuid, "u");
    }

    #[test]
    fn devices_parse_the_cloud_casing() {
        let json = r#"[{
            "deviceUUID": "camera-1",
            "deviceType": "ptzCamera3MP",
            "deviceName": "Cam 360 3MP",
            "macAddress": "00:11:22:33:44:55"
        }, {
            "deviceUUID": "mobile-1",
            "deviceType": "mobile",
            "deviceName": "MobileApp_EFAEFF74"
        }]"#;
        let devices: Vec<Device> = serde_json::from_str(json).unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].device_type, "ptzCamera3MP");
        assert_eq!(devices[0].mac_address.as_deref(), Some("00:11:22:33:44:55"));
        assert_eq!(devices[1].mac_address, None);
        assert_eq!(devices[1].device_name, "MobileApp_EFAEFF74");
        assert_eq!(devices[1].unit_uuid, "");
    }

    #[test]
    fn stream_ticket_parses() {
        let ticket: StreamTicket = serde_json::from_str(
            r#"{"streamURL": "rtsps://wowza/x?token=1", "appSessionId": "s"}"#,
        )
        .unwrap();
        assert_eq!(ticket.stream_url, "rtsps://wowza/x?token=1");
    }

    #[test]
    fn cloud_error_parses() {
        let err: CloudError = serde_json::from_str(
            r#"{"code":"ERR_AUTH_001","message":["Invalid User, Access denied"]}"#,
        )
        .unwrap();
        assert_eq!(err.code, "ERR_AUTH_001");
        assert_eq!(err.message, vec!["Invalid User, Access denied".to_string()]);
    }
}
