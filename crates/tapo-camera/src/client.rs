//! The camera handle: one logged-in connection to one camera's control API.

use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::error::{Error, Result};
use crate::tls::CertFingerprint;
use crate::transport::{PasswordHash, SecureTransport};

/// How to reach and log in to a camera.
#[derive(Debug, Clone)]
pub struct CameraConfig {
    /// IP address or host name.
    pub host: String,
    /// Control API port, 443 on every known model.
    pub port: u16,
    /// Always `admin` for the owner account.
    pub username: String,
    /// The camera owner's TP-Link account password.
    pub password: SecretString,
    /// Certificate fingerprint pinned on a previous connection. `None` trusts and
    /// records whatever the camera presents (read it back with
    /// [`Camera::certificate`] and store it).
    pub certificate: Option<CertFingerprint>,
    /// Per-request timeout.
    pub timeout: Duration,
}

impl CameraConfig {
    pub fn new(host: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 443,
            username: "admin".into(),
            password: SecretString::from(password.into()),
            certificate: None,
            timeout: Duration::from_secs(10),
        }
    }

    pub fn with_certificate(mut self, fingerprint: Option<CertFingerprint>) -> Self {
        self.certificate = fingerprint;
        self
    }
}

/// Why the client refuses to contact the camera for now.
#[derive(Debug, Clone, Copy)]
enum Blocked {
    /// The password was rejected; logging in again would count as another failure.
    BadCredentials,
    /// The camera is locked out until this instant.
    LockedUntil(Instant),
}

/// A connection to one camera's control API.
///
/// Requests are sent one at a time (cameras answer parallel requests with errors) and the
/// session is renewed transparently when it expires. A rejected password or a lockout is
/// remembered, so the client never makes the camera count extra failed logins.
pub struct Camera {
    host: String,
    transport: Mutex<SecureTransport>,
    blocked: StdMutex<Option<Blocked>>,
}

impl std::fmt::Debug for Camera {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Camera")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

impl Camera {
    /// Creates a client without contacting the camera.
    pub fn new(config: CameraConfig) -> Result<Self> {
        let transport = SecureTransport::new(
            &config.host,
            config.port,
            &config.username,
            config.password,
            config.certificate,
            config.timeout,
        )?;
        Ok(Self {
            host: config.host,
            transport: Mutex::new(transport),
            blocked: StdMutex::new(None),
        })
    }

    /// Creates a client and logs in, verifying the credentials.
    pub async fn connect(config: CameraConfig) -> Result<Self> {
        let camera = Self::new(config)?;
        camera.login().await?;
        Ok(camera)
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// The certificate fingerprint seen on the last connection; persist it and pass it
    /// back in [`CameraConfig::certificate`] next time.
    pub async fn certificate(&self) -> Option<CertFingerprint> {
        self.transport.lock().await.certificate()
    }

    /// The password hash the camera uses (needed by the media stream), known after login.
    pub async fn password_hash(&self) -> Option<PasswordHash> {
        self.transport.lock().await.password_hash()
    }

    fn check_blocked(&self) -> Result<()> {
        let mut blocked = self.blocked.lock().expect("blocked lock");
        match *blocked {
            Some(Blocked::BadCredentials) => Err(Error::BadCredentials),
            Some(Blocked::LockedUntil(until)) => {
                let now = Instant::now();
                if now < until {
                    Err(Error::Locked {
                        retry_after: until - now,
                    })
                } else {
                    *blocked = None;
                    Ok(())
                }
            }
            None => Ok(()),
        }
    }

    fn remember(&self, error: &Error) {
        let state = match error {
            Error::BadCredentials | Error::NotOwnerAccount => Blocked::BadCredentials,
            Error::Locked { retry_after } => Blocked::LockedUntil(Instant::now() + *retry_after),
            _ => return,
        };
        *self.blocked.lock().expect("blocked lock") = Some(state);
    }

    /// Logs in now (normally done lazily by the first request).
    pub async fn login(&self) -> Result<()> {
        self.check_blocked()?;
        let result = self.transport.lock().await.login().await;
        if let Err(err) = &result {
            self.remember(err);
        }
        result
    }

    /// Sends a raw request and returns the camera's full response.
    pub async fn raw_request(&self, request: &Value) -> Result<Value> {
        self.check_blocked()?;
        let result = self.transport.lock().await.send(request).await;
        if let Err(err) = &result {
            self.remember(err);
        }
        result
    }

    /// Calls one method through `multipleRequest` and returns its `result`.
    pub async fn execute(&self, method: &str, params: Value) -> Result<Value> {
        self.execute_many(&[(method, params)])
            .await?
            .pop()
            .ok_or_else(|| Error::protocol("empty multipleRequest response"))?
    }

    /// Calls several methods in one round trip. The outer error is for the whole
    /// request; each inner result is for one method.
    pub async fn execute_many(&self, calls: &[(&str, Value)]) -> Result<Vec<Result<Value>>> {
        let requests: Vec<Value> = calls
            .iter()
            .map(|(method, params)| json!({ "method": method, "params": params }))
            .collect();
        let response = self
            .raw_request(
                &json!({ "method": "multipleRequest", "params": { "requests": requests } }),
            )
            .await?;

        if let Some(code) = response.get("error_code").and_then(Value::as_i64)
            && code != 0
        {
            return Err(Error::Camera { code, method: None });
        }
        let responses = response
            .pointer("/result/responses")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::protocol(format!("missing responses: {response}")))?;

        Ok(calls
            .iter()
            .zip(responses)
            .map(|((method, _), item)| {
                match item.get("error_code").and_then(Value::as_i64).unwrap_or(0) {
                    0 => Ok(item.get("result").cloned().unwrap_or(Value::Null)),
                    code => Err(Error::Camera {
                        code,
                        method: Some((*method).to_owned()),
                    }),
                }
            })
            .collect())
    }
}
