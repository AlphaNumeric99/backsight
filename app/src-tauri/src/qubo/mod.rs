//! Qubo cloud cameras: the account, its devices and the live stream.
//!
//! This integration uses the cloud interface of the Qubo Smart Cam 360 3MP.
//! Backsight talks to the vendor's cloud with the
//! owner's own account: it signs in, lists the account's cameras and asks the cloud for
//! a short-lived live-view URL (RTSPS, relayed through the vendor's Wowza servers).
//! SD-card playback and exports are not implemented by this integration.
//!
//! The cloud protocol was reverse-engineered from the Qubo Android app
//! (`com.hero.iot` 3.2.92); see [`cloud`] for the request shapes.
//!
//! A session is a pair of JWTs: an access token that lives one hour and a refresh
//! token that lives 180 days. Both are kept in the OS keychain (never in the
//! database or logs); the account password is stored once, when the camera is added.

pub mod cloud;
pub mod rtsp;
pub mod video;

use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use tokio::sync::Mutex;

use crate::error::{ApiError, ApiResult};
use crate::secrets;

pub use cloud::{CloudClient, Device, StreamTicket, Tokens};

/// Reuse the single camera account type for the Qubo account (e-mail and password).
pub use crate::model::CameraAccount;

/// How long before its expiry an access token is refreshed.
const REFRESH_MARGIN_SECS: i64 = 300;

/// The TLS configuration for the Qubo cloud and its streams: the ring provider with
/// the usual public roots. Shared by the REST client and the RTSP connection.
pub(crate) fn tls_config() -> ApiResult<Arc<rustls::ClientConfig>> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
            .map_err(|e| e.to_string())
            .map(|builder| Arc::new(builder.with_root_certificates(roots).with_no_client_auth()))
        })
        .clone()
        .map_err(|e| ApiError::internal(format!("TLS setup: {e}")))
}

/// The signed-in Qubo account, shared by every Qubo camera.
///
/// Tokens are cached in memory and persisted to the keychain; they are refreshed when
/// they are about to expire, and the stored account password is only used (a full
/// login) when there are no tokens or the refresh token has been rejected.
pub struct Qubo {
    cloud: CloudClient,
    tokens: Mutex<Option<Tokens>>,
    blocked: StdMutex<Option<ApiError>>,
}

impl Qubo {
    pub fn new() -> ApiResult<Self> {
        Ok(Self {
            cloud: CloudClient::new()?,
            tokens: Mutex::new(secrets::qubo_tokens()),
            blocked: StdMutex::new(None),
        })
    }

    /// Verifies the credentials, stores them as the account, and returns the
    /// account's devices.
    pub async fn sign_in(&self, account: &CameraAccount) -> ApiResult<Vec<Device>> {
        if account.username.trim().is_empty() {
            return Err(ApiError::invalid(
                "Enter the Qubo account's e-mail address.",
            ));
        }
        if account.password.is_empty() {
            return Err(ApiError::invalid("Enter the Qubo account's password."));
        }
        let mut cached = self.tokens.lock().await;
        let tokens = self
            .cloud
            .login(account.username.trim(), &account.password)
            .await?;
        let devices = self.cloud.devices(&tokens).await?;
        let known_account = secrets::qubo_account();
        if known_account
            .as_ref()
            .is_some_and(|known| !known.username.eq_ignore_ascii_case(account.username.trim()))
        {
            return Err(ApiError::invalid(
                "Remove the saved Qubo cameras before changing accounts.",
            ));
        }
        let account = CameraAccount {
            username: account.username.trim().to_owned(),
            password: account.password.clone(),
        };
        secrets::set_qubo_account(&account)?;
        secrets::set_qubo_tokens(&tokens)?;
        *cached = Some(tokens);
        *self.blocked.lock().expect("Qubo login lock") = None;
        Ok(devices)
    }

    /// The account's devices, across all units.
    pub async fn devices(&self) -> ApiResult<Vec<Device>> {
        let tokens = self.access_tokens(false).await?;
        match self.cloud.devices(&tokens).await {
            Err(err) if err.code == "auth_failed" => {
                // The cloud may revoke a token before its JWT expiry.
                self.cloud.devices(&self.access_tokens(true).await?).await
            }
            result => result,
        }
    }

    /// A fresh live-stream ticket for one camera. The URL is short-lived, so it is
    /// requested when a stream starts, never cached.
    pub async fn stream_ticket(&self, device_uuid: &str, quality: &str) -> ApiResult<StreamTicket> {
        let devices = self.devices().await?;
        let tokens = self.access_tokens(false).await?;
        let device = devices
            .iter()
            .find(|d| d.device_uuid == device_uuid)
            .ok_or_else(|| ApiError::not_found("qubo camera"))?;
        let self_device = devices
            .iter()
            .find(|d| d.device_type == "mobile" && d.unit_uuid == device.unit_uuid)
            .map(|d| d.device_uuid.as_str())
            .ok_or_else(|| {
                ApiError::internal("the Qubo account has no mobile device registered")
            })?;
        self.cloud
            .stream_url(&tokens, device, self_device, quality)
            .await
    }

    /// Access tokens good for at least [`REFRESH_MARGIN_SECS`]: the cached pair, a
    /// refreshed pair, or a full login with the stored account password.
    async fn access_tokens(&self, force_refresh: bool) -> ApiResult<Tokens> {
        if let Some(err) = self.blocked.lock().expect("Qubo login lock").clone() {
            return Err(err);
        }
        let mut cached = self.tokens.lock().await;
        let refresh = |tokens: &Tokens| force_refresh || tokens.expires_in() < REFRESH_MARGIN_SECS;
        let current = match cached.clone() {
            Some(tokens) if !refresh(&tokens) => Some(tokens),
            Some(tokens) => match self.cloud.refresh(&tokens).await {
                Ok(access) => {
                    let tokens = tokens.with_access(access);
                    secrets::set_qubo_tokens(&tokens)?;
                    Some(tokens)
                }
                Err(err) if err.code == "auth_failed" || err.code == "invalid_input" => None,
                Err(err) => return Err(err),
            },
            None => None,
        };
        if let Some(tokens) = current {
            *cached = Some(tokens.clone());
            return Ok(tokens);
        }

        let Some(account) = secrets::qubo_account() else {
            return Err(ApiError::new(
                "auth_failed",
                "No Qubo account is signed in. Add the camera again.",
            ));
        };
        let tokens = self.cloud.login(&account.username, &account.password).await;
        let tokens = match tokens {
            Ok(tokens) => tokens,
            Err(err) => {
                if matches!(err.code, "auth_failed" | "invalid_input") {
                    *self.blocked.lock().expect("Qubo login lock") = Some(err.clone());
                }
                return Err(err);
            }
        };
        secrets::set_qubo_tokens(&tokens)?;
        *cached = Some(tokens.clone());
        Ok(tokens)
    }

    /// Removing the last Qubo camera also removes its shared account secrets.
    pub async fn forget(&self) {
        *self.tokens.lock().await = None;
        *self.blocked.lock().expect("Qubo login lock") = None;
        secrets::delete_qubo_account();
    }
}

impl std::fmt::Debug for Qubo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Qubo").finish_non_exhaustive()
    }
}

/// The keychain entries the Qubo account uses; for tests that shouldn't touch the
/// real credential store.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip_through_the_keychain_format() {
        let tokens = Tokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            uuid: "u".into(),
            source_device_id: "viewer".into(),
        };
        let json = serde_json::to_string(&tokens).unwrap();
        let back: Tokens = serde_json::from_str(&json).unwrap();
        assert_eq!(back.access_token, "a");
        assert_eq!(back.uuid, "u");
    }

    /// Opt-in check against the owner's camera. Tokens and output stay local; this
    /// exercises the same REST, TLS, RTP and depacketizer code the player uses.
    #[tokio::test]
    #[ignore = "requires QUBO_TOKEN_FILE and a camera on that account"]
    async fn live_relay_produces_decodable_video() {
        use std::time::Duration;
        use tapo_camera::media::MediaEvent;

        let path = std::env::var("QUBO_TOKEN_FILE").expect("QUBO_TOKEN_FILE");
        let mut tokens: Tokens = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let cloud = CloudClient::new().unwrap();
        tokens.access_token = cloud.refresh(&tokens).await.unwrap();
        let devices = cloud.devices(&tokens).await.unwrap();
        let device = devices
            .iter()
            .find(|d| d.device_type == "ptzCamera3MP")
            .expect("3MP camera");
        let mobile = devices
            .iter()
            .find(|d| d.device_type == "mobile" && d.unit_uuid == device.unit_uuid)
            .expect("mobile device");
        let ticket = cloud
            .stream_url(&tokens, device, &mobile.device_uuid, "high")
            .await
            .unwrap();
        let mut session = tokio::time::timeout(
            Duration::from_secs(20),
            rtsp::RtspSession::connect(&ticket.stream_url, Duration::from_secs(10)),
        )
        .await
        .unwrap()
        .unwrap();
        let mut depacketizer = video::VideoDepacketizer::default()
            .with_codec(if session.codec == "H265" {
                tapo_camera::media::VideoCodec::H265
            } else {
                tapo_camera::media::VideoCodec::H264
            })
            .with_parameter_sets(&session.parameter_sets);
        let mut events = Vec::new();
        let mut frames = 0;
        let wanted = std::env::var("QUBO_FRAME_COUNT")
            .ok()
            .and_then(|n| n.parse::<usize>().ok())
            .unwrap_or(50)
            .clamp(1, 600);
        let mut elementary = Vec::new();
        let mut dimensions = None;
        let received = tokio::time::timeout(Duration::from_secs(90), async {
            while frames < wanted {
                let packet = session.next_packet().await.unwrap().expect("relay ended");
                depacketizer.push(&packet, &mut events);
                for event in events.drain(..) {
                    match event {
                        MediaEvent::VideoConfig(config) => {
                            dimensions = Some((config.width, config.height));
                            // Parameter sets in avcC are length-prefixed (2 bytes).
                            let data = &config.description;
                            if config.codec == tapo_camera::media::VideoCodec::H265 {
                                let mut at = 23;
                                for _ in 0..data[22] {
                                    at += 1;
                                    let count = u16::from_be_bytes([data[at], data[at + 1]]);
                                    at += 2;
                                    for _ in 0..count {
                                        let len =
                                            u16::from_be_bytes([data[at], data[at + 1]]) as usize;
                                        at += 2;
                                        elementary.extend_from_slice(&[0, 0, 0, 1]);
                                        elementary.extend_from_slice(&data[at..at + len]);
                                        at += len;
                                    }
                                }
                                continue;
                            }
                            let mut at = 6;
                            let sps_count = data[5] & 31;
                            for _ in 0..sps_count {
                                let len = u16::from_be_bytes([data[at], data[at + 1]]) as usize;
                                at += 2;
                                elementary.extend_from_slice(&[0, 0, 0, 1]);
                                elementary.extend_from_slice(&data[at..at + len]);
                                at += len;
                            }
                            let pps_count = data[at];
                            at += 1;
                            for _ in 0..pps_count {
                                let len = u16::from_be_bytes([data[at], data[at + 1]]) as usize;
                                at += 2;
                                elementary.extend_from_slice(&[0, 0, 0, 1]);
                                elementary.extend_from_slice(&data[at..at + len]);
                                at += len;
                            }
                        }
                        MediaEvent::Video(frame) => {
                            assert!(
                                dimensions.is_some(),
                                "video arrived without decoder configuration"
                            );
                            for nal in frame.nal_units() {
                                elementary.extend_from_slice(&[0, 0, 0, 1]);
                                elementary.extend_from_slice(nal);
                            }
                            frames += 1;
                        }
                        _ => {}
                    }
                }
            }
        })
        .await;
        session.teardown().await;
        received.expect("live video stalled");
        if let Ok(output) = std::env::var("QUBO_VIDEO_OUTPUT") {
            std::fs::write(output, elementary).unwrap();
        }
        println!("Received {frames} frames; dimensions {dimensions:?}.");
    }
}
