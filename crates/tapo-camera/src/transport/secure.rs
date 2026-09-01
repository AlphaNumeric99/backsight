//! The HTTPS control API with the "secure connection" login (`encrypt_type` 3), plus
//! the older plain login it replaced.
//!
//! Login and request encryption follow pytapo's `transport/pytapo/pytapo.py`
//! (MIT, Juraj Nyíri and contributors):
//!
//! 1. Send `login` with a random client nonce. The camera answers with its own nonce
//!    and a `device_confirm` value that proves it knows the password; checking it
//!    locally also tells us which password hash (SHA-256 or MD5) the camera uses —
//!    and catches a wrong password before the camera counts a failed attempt.
//! 2. Send `login` again with `digest_passwd`, receiving a session token (`stok`) and
//!    a starting sequence number.
//! 3. Every request is AES-128-CBC encrypted with a key and IV derived from both nonces,
//!    wrapped in `securePassthrough`, and signed with a `Tapo_tag` header.

use std::time::Duration;

use aes::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use md5::Md5;
use reqwest::header::{self, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::tls::{self, CertFingerprint, SharedPin};

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Top-level error codes after which pytapo re-establishes the session once.
const SESSION_ERROR_CODES: &[i64] = &[
    -40401, // session expired
    -1,     // e.g. "check tapo tag failed"
    9999,   // session timeout
    -40413, // invalid nonce
    1002,   // transport not available
    1112,   // HTTP transport failed
    -1001,  // unspecific error
];

/// How the camera hashes the password before mixing it with the nonces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PasswordHash {
    Md5,
    Sha256,
}

impl PasswordHash {
    /// The password hashed this way, as uppercase hex.
    pub fn apply(self, password: &str) -> String {
        match self {
            Self::Md5 => hex::encode_upper(Md5::digest(password.as_bytes())),
            Self::Sha256 => hex::encode_upper(Sha256::digest(password.as_bytes())),
        }
    }
}

fn sha256_upper(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hex::encode_upper(hasher.finalize())
}

/// The value a genuine camera sends as `device_confirm`.
pub(crate) fn expected_device_confirm(cnonce: &str, hashed_password: &str, nonce: &str) -> String {
    let proof = sha256_upper(&[
        cnonce.as_bytes(),
        hashed_password.as_bytes(),
        nonce.as_bytes(),
    ]);
    format!("{proof}{nonce}{cnonce}")
}

/// The `digest_passwd` we send in the second login step.
pub(crate) fn digest_password(cnonce: &str, hashed_password: &str, nonce: &str) -> String {
    let digest = sha256_upper(&[
        hashed_password.as_bytes(),
        cnonce.as_bytes(),
        nonce.as_bytes(),
    ]);
    format!("{digest}{cnonce}{nonce}")
}

/// Session key (`"lsk"`) or IV (`"ivb"`).
pub(crate) fn derive_key(
    label: &str,
    cnonce: &str,
    hashed_password: &str,
    nonce: &str,
) -> [u8; 16] {
    let hashed_key = sha256_upper(&[
        cnonce.as_bytes(),
        hashed_password.as_bytes(),
        nonce.as_bytes(),
    ]);
    let mut hasher = Sha256::new();
    hasher.update(label.as_bytes());
    hasher.update(cnonce.as_bytes());
    hasher.update(nonce.as_bytes());
    hasher.update(hashed_key.as_bytes());
    let digest = hasher.finalize();
    digest[..16].try_into().expect("16 of 32 bytes")
}

/// The `Tapo_tag` header for a request body sent with sequence number `seq`.
pub(crate) fn tapo_tag(hashed_password: &str, cnonce: &str, body: &[u8], seq: u64) -> String {
    let inner = sha256_upper(&[hashed_password.as_bytes(), cnonce.as_bytes()]);
    sha256_upper(&[inner.as_bytes(), body, seq.to_string().as_bytes()])
}

fn random_nonce() -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("OS random number generator");
    hex::encode_upper(bytes)
}

fn error_code(value: &Value) -> Option<i64> {
    value.get("error_code").and_then(Value::as_i64)
}

/// Detects the camera's "too many failed logins" responses.
fn check_lockout(value: &Value) -> Result<()> {
    let seconds = |v: Option<&Value>| v.and_then(Value::as_u64).filter(|s| *s > 0);
    let locked = seconds(value.pointer("/result/data/sec_left"))
        .or_else(|| {
            (value.pointer("/data/code").and_then(Value::as_i64) == Some(-40404))
                .then(|| seconds(value.pointer("/data/sec_left")))
                .flatten()
        })
        .or_else(|| (error_code(value) == Some(-40404)).then_some(1800));
    match locked {
        Some(secs) => Err(Error::Locked {
            retry_after: Duration::from_secs(secs),
        }),
        None => Ok(()),
    }
}

struct Session {
    stok: String,
    /// `None` for the old unencrypted login.
    cipher: Option<SessionCipher>,
}

struct SessionCipher {
    lsk: [u8; 16],
    ivb: [u8; 16],
    seq: u64,
    cnonce: String,
    hashed_password: String,
}

enum Reply {
    Json(Value),
    /// The response couldn't be decrypted: the camera dropped our session.
    SessionLost,
}

/// Connection to one camera's control API.
pub(crate) struct SecureTransport {
    http: reqwest::Client,
    origin: String,
    username: String,
    password: SecretString,
    pin: SharedPin,
    session: Option<Session>,
    password_hash: Option<PasswordHash>,
}

impl SecureTransport {
    pub(crate) fn new(
        host: &str,
        port: u16,
        username: &str,
        password: SecretString,
        expected_certificate: Option<CertFingerprint>,
        timeout: Duration,
    ) -> Result<Self> {
        let origin = format!("https://{host}:{port}");
        let pin = tls::new_pin(expected_certificate);
        let tls = tls::client_config(pin.clone()).map_err(|e| Error::Tls(e.to_string()))?;

        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=UTF-8"),
        );
        headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static("Tapo CameraClient Android"),
        );
        headers.insert("requestByApp", HeaderValue::from_static("true"));
        headers.insert(
            header::REFERER,
            HeaderValue::from_str(&origin).map_err(|e| Error::protocol(e.to_string()))?,
        );

        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .default_headers(headers)
            .http1_only()
            .http1_title_case_headers()
            .pool_max_idle_per_host(1)
            .connect_timeout(Duration::from_secs(5))
            .timeout(timeout)
            .build()?;

        Ok(Self {
            http,
            origin,
            username: username.to_owned(),
            password,
            pin,
            session: None,
            password_hash: None,
        })
    }

    /// Certificate fingerprint seen in the most recent handshake.
    pub(crate) fn certificate(&self) -> Option<CertFingerprint> {
        tls::seen(&self.pin)
    }

    /// The password hash the camera uses, known after the first successful login.
    pub(crate) fn password_hash(&self) -> Option<PasswordHash> {
        self.password_hash
    }

    async fn post(&self, url: &str, body: Vec<u8>, extra: Option<HeaderMap>) -> Result<Value> {
        let mut request = self.http.post(url).body(body);
        if let Some(extra) = extra {
            request = request.headers(extra);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(err) => {
                if let Some((expected, actual)) = tls::mismatch(&self.pin) {
                    return Err(Error::CertificateMismatch {
                        expected: expected.to_hex(),
                        actual: actual.to_hex(),
                    });
                }
                return Err(err.into());
            }
        };
        // Secure sessions report expiry with HTTP 500 and a JSON body, so read the
        // body regardless of the status code.
        let bytes = response.bytes().await?;
        serde_json::from_slice(&bytes).map_err(|e| {
            Error::protocol(format!(
                "invalid JSON ({e}): {}",
                String::from_utf8_lossy(&bytes)
            ))
        })
    }

    async fn post_json(&self, url: &str, value: &Value) -> Result<Value> {
        self.post(url, serde_json::to_vec(value).expect("JSON"), None)
            .await
    }

    /// Logs in, replacing any existing session.
    pub(crate) async fn login(&mut self) -> Result<()> {
        self.session = None;
        let cnonce = random_nonce();
        let first = self
            .post_json(
                &self.origin,
                &json!({
                    "method": "login",
                    "params": { "cnonce": cnonce, "encrypt_type": "3", "username": self.username },
                }),
            )
            .await?;
        check_lockout(&first)?;

        let data = first.pointer("/result/data");
        let field = |key: &str| data.and_then(|d| d.get(key)).and_then(Value::as_str);
        let secure = error_code(&first) == Some(-40413)
            && data
                .and_then(|d| d.get("encrypt_type"))
                .is_some_and(|t| t.to_string().contains('3'));

        match (field("nonce"), field("device_confirm")) {
            (Some(nonce), Some(confirm)) if secure => {
                let (nonce, confirm) = (nonce.to_owned(), confirm.to_owned());
                self.login_secure(&cnonce, &nonce, &confirm).await
            }
            _ => match error_code(&first) {
                Some(-40211) => Err(Error::UnsupportedLogin(
                    "the camera requires the newer TPAP login (encrypt_type 4)".into(),
                )),
                // No nonce offered: firmware from before the secure login.
                _ if !secure => self.login_plain().await,
                code => Err(Error::Camera {
                    code: code.unwrap_or(0),
                    method: Some("login".into()),
                }),
            },
        }
    }

    async fn login_secure(&mut self, cnonce: &str, nonce: &str, confirm: &str) -> Result<()> {
        let password = self.password.expose_secret();
        let hash = [PasswordHash::Sha256, PasswordHash::Md5]
            .into_iter()
            .find(|h| expected_device_confirm(cnonce, &h.apply(password), nonce) == confirm)
            .ok_or(Error::BadCredentials)?;
        let hashed_password = hash.apply(password);

        let second = self
            .post_json(
                &self.origin,
                &json!({
                    "method": "login",
                    "params": {
                        "cnonce": cnonce,
                        "encrypt_type": "3",
                        "digest_passwd": digest_password(cnonce, &hashed_password, nonce),
                        "username": self.username,
                    },
                }),
            )
            .await?;
        check_lockout(&second)?;

        let result = second.get("result");
        let stok = result.and_then(|r| r.get("stok")).and_then(Value::as_str);
        let start_seq = result
            .and_then(|r| r.get("start_seq"))
            .and_then(Value::as_u64);
        let (Some(stok), Some(start_seq)) = (stok, start_seq) else {
            return Err(match error_code(&second) {
                Some(-40411 | -40209 | -40401) => Error::BadCredentials,
                code => Error::Camera {
                    code: code.unwrap_or(0),
                    method: Some("login".into()),
                },
            });
        };
        if let Some(group) = result
            .and_then(|r| r.get("user_group"))
            .and_then(Value::as_str)
            && group != "root"
        {
            return Err(Error::NotOwnerAccount);
        }

        self.password_hash = Some(hash);
        self.session = Some(Session {
            stok: stok.to_owned(),
            cipher: Some(SessionCipher {
                lsk: derive_key("lsk", cnonce, &hashed_password, nonce),
                ivb: derive_key("ivb", cnonce, &hashed_password, nonce),
                seq: start_seq,
                cnonce: cnonce.to_owned(),
                hashed_password,
            }),
        });
        Ok(())
    }

    async fn login_plain(&mut self) -> Result<()> {
        let hashed = PasswordHash::Md5.apply(self.password.expose_secret());
        let reply = self
            .post_json(
                &self.origin,
                &json!({
                    "method": "login",
                    "params": { "hashed": true, "password": hashed, "username": self.username },
                }),
            )
            .await?;
        check_lockout(&reply)?;
        let Some(stok) = reply.pointer("/result/stok").and_then(Value::as_str) else {
            return Err(match error_code(&reply) {
                Some(-40401 | -40411 | -40209) => Error::BadCredentials,
                code => Error::Camera {
                    code: code.unwrap_or(0),
                    method: Some("login".into()),
                },
            });
        };
        self.password_hash = Some(PasswordHash::Md5);
        self.session = Some(Session {
            stok: stok.to_owned(),
            cipher: None,
        });
        Ok(())
    }

    async fn send_once(&mut self, request: &Value) -> Result<Reply> {
        let session = self.session.as_mut().expect("logged in");
        let url = format!("{}/stok={}/ds", self.origin, session.stok);
        let plain = serde_json::to_vec(request).expect("JSON");

        let Some(cipher) = session.cipher.as_mut() else {
            return self.post(&url, plain, None).await.map(Reply::Json);
        };

        let encrypted = Aes128CbcEnc::new(&cipher.lsk.into(), &cipher.ivb.into())
            .encrypt_padded_vec::<Pkcs7>(&plain);
        let body = serde_json::to_vec(&json!({
            "method": "securePassthrough",
            "params": { "request": BASE64.encode(encrypted) },
        }))
        .expect("JSON");

        let seq = cipher.seq;
        cipher.seq += 1;
        let mut headers = HeaderMap::new();
        headers.insert("Seq", HeaderValue::from(seq));
        headers.insert(
            "Tapo_tag",
            HeaderValue::from_str(&tapo_tag(
                &cipher.hashed_password,
                &cipher.cnonce,
                &body,
                seq,
            ))
            .expect("hex is a valid header value"),
        );
        let (lsk, ivb) = (cipher.lsk, cipher.ivb);

        let reply = self.post(&url, body, Some(headers)).await?;
        let Some(encoded) = reply.pointer("/result/response").and_then(Value::as_str) else {
            // Errors such as an expired session come back unencrypted.
            return Ok(Reply::Json(reply));
        };
        let ciphertext = BASE64
            .decode(encoded)
            .map_err(|e| Error::protocol(format!("bad base64 in response: {e}")))?;
        match Aes128CbcDec::new(&lsk.into(), &ivb.into()).decrypt_padded_vec::<Pkcs7>(&ciphertext) {
            Ok(plain) => serde_json::from_slice(&plain)
                .map(Reply::Json)
                .map_err(|e| Error::protocol(format!("invalid decrypted JSON: {e}"))),
            Err(_) => Ok(Reply::SessionLost),
        }
    }

    /// Sends one request, logging in first if needed. Re-establishes the session at
    /// most once when the camera reports it expired; never retries a rejected password.
    pub(crate) async fn send(&mut self, request: &Value) -> Result<Value> {
        let mut renewed = false;
        loop {
            if self.session.is_none() {
                self.login().await?;
            }
            let outcome = self.send_once(request).await;
            let retryable = match &outcome {
                Ok(Reply::Json(value)) => {
                    error_code(value).is_some_and(|c| SESSION_ERROR_CODES.contains(&c))
                }
                Ok(Reply::SessionLost) => true,
                Err(Error::Http(err)) => err.is_connect() || err.is_request(),
                Err(_) => false,
            };
            if retryable && !renewed {
                tracing::debug!("control session lost; logging in again");
                renewed = true;
                self.session = None;
                continue;
            }
            return match outcome? {
                Reply::Json(value) => Ok(value),
                Reply::SessionLost => {
                    self.session = None;
                    Err(Error::protocol(
                        "the camera dropped the session twice in a row",
                    ))
                }
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CNONCE: &str = "0123456789ABCDEF";
    const NONCE: &str = "FEDCBA9876543210";

    #[test]
    fn password_hashes_are_uppercase_hex() {
        assert_eq!(
            PasswordHash::Md5.apply("password"),
            "5F4DCC3B5AA765D61D8327DEB882CF99"
        );
        assert_eq!(
            PasswordHash::Sha256.apply("password"),
            "5E884898DA28047151D0E56F8DC6292773603D0D6AABBDD62A11EF721D1542D8"
        );
    }

    #[test]
    fn device_confirm_identifies_the_hash() {
        let md5 = PasswordHash::Md5.apply("secret");
        let confirm = expected_device_confirm(CNONCE, &md5, NONCE);
        assert!(confirm.ends_with(&format!("{NONCE}{CNONCE}")));
        assert_eq!(confirm.len(), 64 + 32);
        let sha = PasswordHash::Sha256.apply("secret");
        assert_ne!(confirm, expected_device_confirm(CNONCE, &sha, NONCE));
    }

    #[test]
    fn digest_password_layout() {
        let hashed = PasswordHash::Sha256.apply("secret");
        let digest = digest_password(CNONCE, &hashed, NONCE);
        assert_eq!(digest.len(), 64 + 32);
        assert!(digest.ends_with(&format!("{CNONCE}{NONCE}")));
    }

    #[test]
    fn keys_differ_by_label_and_round_trip() {
        let hashed = PasswordHash::Sha256.apply("secret");
        let lsk = derive_key("lsk", CNONCE, &hashed, NONCE);
        let ivb = derive_key("ivb", CNONCE, &hashed, NONCE);
        assert_ne!(lsk, ivb);

        let message = br#"{"method":"multipleRequest"}"#;
        let ct = Aes128CbcEnc::new(&lsk.into(), &ivb.into()).encrypt_padded_vec::<Pkcs7>(message);
        assert_eq!(ct.len() % 16, 0);
        let pt = Aes128CbcDec::new(&lsk.into(), &ivb.into())
            .decrypt_padded_vec::<Pkcs7>(&ct)
            .unwrap();
        assert_eq!(pt, message);
    }

    #[test]
    fn tag_depends_on_body_and_seq() {
        let hashed = PasswordHash::Sha256.apply("secret");
        let a = tapo_tag(&hashed, CNONCE, b"{}", 1);
        assert_eq!(a.len(), 64);
        assert_ne!(a, tapo_tag(&hashed, CNONCE, b"{}", 2));
        assert_ne!(a, tapo_tag(&hashed, CNONCE, b"[]", 1));
    }

    #[test]
    fn lockout_forms() {
        assert!(check_lockout(&json!({"error_code": 0})).is_ok());
        let locked = check_lockout(
            &json!({"result": {"data": {"sec_left": 1200, "time": 10, "max_time": 10}}}),
        );
        assert!(
            matches!(locked, Err(Error::Locked { retry_after }) if retry_after.as_secs() == 1200)
        );
        let locked = check_lockout(&json!({"data": {"code": -40404, "sec_left": 30}}));
        assert!(matches!(locked, Err(Error::Locked { .. })));
        assert!(matches!(
            check_lockout(&json!({"error_code": -40404})),
            Err(Error::Locked { .. })
        ));
    }
}
