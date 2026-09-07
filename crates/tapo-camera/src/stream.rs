//! The media stream on TCP port 8800: live view, SD-card playback and downloads.
//!
//! The camera speaks an HTTP-like protocol: an HTTP Digest-authenticated `POST /stream`
//! that turns into a long-lived, two-way `multipart/mixed` exchange. The client sends JSON
//! requests as parts; the camera answers with JSON parts and AES-128-CBC encrypted
//! MPEG-TS parts. Keys come from the `Key-Exchange` response header and the owner's
//! password.
//!
//! Ported from pytapo's `media_stream/session.py` and `crypto.py` (MIT, Juraj Nyíri and
//! contributors), with details from go2rtc's `pkg/tapo` (MIT, Alexey Khit).

use std::collections::HashMap;
use std::time::Duration;

use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use bytes::Bytes;
use md5::{Digest, Md5};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::error::{Error, Result};

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

const CLIENT_BOUNDARY: &str = "--client-stream-boundary--";
const DEFAULT_DEVICE_BOUNDARY: &str = "--device-stream-boundary--";
/// Largest part we accept; real parts are a few KB to a few hundred KB.
const MAX_PART_SIZE: usize = 16 * 1024 * 1024;
/// Key used by cameras with media encryption turned off (`username="none"` in the key
/// exchange); publicly documented as CVE-2022-37255 and used the same way by go2rtc.
const UNENCRYPTED_MEDIA_KEY: &str = "TPL075526460603";

fn md5_hex(data: &[u8]) -> String {
    hex::encode(Md5::digest(data))
}

/// Parses `key="value"` pairs separated by commas or spaces (Digest challenges and the
/// `Key-Exchange` header).
fn parse_params(input: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    let mut rest = input.trim();
    while !rest.is_empty() {
        rest = rest.trim_start_matches([',', ' ']);
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().to_owned();
        rest = &rest[eq + 1..];
        let value;
        if let Some(stripped) = rest.strip_prefix('"') {
            let end = stripped.find('"').unwrap_or(stripped.len());
            value = stripped[..end].to_owned();
            rest = stripped.get(end + 1..).unwrap_or("");
        } else {
            let end = rest.find([',', ' ']).unwrap_or(rest.len());
            value = rest[..end].to_owned();
            rest = &rest[end..];
        }
        params.insert(key, value);
    }
    params
}

/// RFC 2617 Digest `response` with `qop=auth`.
#[allow(clippy::too_many_arguments)]
fn digest_response(
    username: &str,
    realm: &str,
    password: &str,
    method: &str,
    uri: &str,
    nonce: &str,
    nc: &str,
    cnonce: &str,
    qop: &str,
) -> String {
    let ha1 = md5_hex(format!("{username}:{realm}:{password}").as_bytes());
    let ha2 = md5_hex(format!("{method}:{uri}").as_bytes());
    md5_hex(format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}").as_bytes())
}

/// The password as the media server wants it hashed: SHA-256 when the challenge says
/// `encrypt_type="3"`, MD5 otherwise; uppercase hex either way.
fn hash_media_password(password: &str, challenge: &HashMap<String, String>) -> String {
    if challenge.get("encrypt_type").map(String::as_str) == Some("3") {
        hex::encode_upper(Sha256::digest(password.as_bytes()))
    } else {
        hex::encode_upper(Md5::digest(password.as_bytes()))
    }
}

/// AES key and IV for the stream, derived from the `Key-Exchange` header.
#[derive(Clone)]
struct StreamCipher {
    key: [u8; 16],
    iv: [u8; 16],
}

impl StreamCipher {
    fn from_key_exchange(
        exchange: &HashMap<String, String>,
        hashed_password: &str,
    ) -> Result<Self> {
        let nonce = exchange
            .get("nonce")
            .ok_or_else(|| Error::protocol("Key-Exchange without nonce"))?;
        let username = exchange
            .get("username")
            .map(String::as_str)
            .unwrap_or("admin");
        let secret = if username == "none" {
            UNENCRYPTED_MEDIA_KEY
        } else {
            hashed_password
        };
        Ok(Self {
            key: Md5::digest(format!("{nonce}:{secret}").as_bytes()).into(),
            iv: Md5::digest(format!("{username}:{nonce}").as_bytes()).into(),
        })
    }

    /// Each part is encrypted independently with the same IV.
    fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        Aes128CbcDec::new(&self.key.into(), &self.iv.into())
            .decrypt_padded_vec::<Pkcs7>(data)
            .map_err(|_| {
                Error::protocol("could not decrypt a media part (wrong password for the stream?)")
            })
    }
}

/// Where to reach a camera's media server and how to authenticate.
#[derive(Debug, Clone)]
pub struct MediaConfig {
    pub host: String,
    /// 8800 on every known model.
    pub port: u16,
    /// Always `admin` for the owner account.
    pub username: String,
    /// The camera owner's TP-Link account password.
    pub password: SecretString,
    pub connect_timeout: Duration,
    /// Acknowledge downloads every this many parts; bigger is faster, but some cameras
    /// stall above ~500. `None` sends no window size (live view).
    pub window_size: Option<u32>,
}

impl MediaConfig {
    pub fn new(host: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 8800,
            username: "admin".into(),
            password: SecretString::from(password.into()),
            connect_timeout: Duration::from_secs(10),
            window_size: None,
        }
    }
}

/// Live stream quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// Main stream (full resolution).
    High,
    /// Sub stream (VGA-ish), for thumbnails and grids.
    Low,
}

/// What to ask the media server for.
#[derive(Debug, Clone)]
pub enum StreamRequest {
    /// Live view.
    Live { quality: Quality, channel: u32 },
    /// SD-card playback paced at real time, starting at `start` (camera clock, unix
    /// seconds). Doesn't stop by itself at `end`.
    Playback {
        client_id: u64,
        start: i64,
        end: i64,
    },
    /// SD-card download of `[start, end]` as fast as the link allows (~10× real time);
    /// ends with a "finished" notification.
    Download {
        client_id: u64,
        start: i64,
        end: i64,
        player_id: String,
    },
    /// The JPEG thumbnail the camera keeps for the detection recording starting at
    /// `start`: one `image/jpeg` part, then "finished". Continuous recordings have none
    /// (the camera then stays silent). A media session keeps the media type of its first
    /// request, so fetch thumbnails on a session of their own.
    Thumbnail {
        client_id: u64,
        start: i64,
        player_id: String,
    },
}

impl StreamRequest {
    fn to_json(&self, seq: u32) -> Value {
        let params = match self {
            Self::Live { quality, channel } => json!({
                "preview": {
                    "audio": ["default"],
                    "channels": [channel],
                    "resolutions": [match quality { Quality::High => "HD", Quality::Low => "VGA" }],
                },
                "method": "get",
            }),
            Self::Playback {
                client_id,
                start,
                end,
            } => json!({
                "playback": {
                    "client_id": client_id,
                    "channels": [0, 1],
                    "scale": "1/1",
                    "start_time": start.to_string(),
                    "end_time": end.to_string(),
                    "event_type": [1, 2],
                },
                "method": "get",
            }),
            Self::Download {
                client_id,
                start,
                end,
                player_id,
            } => json!({
                "download": {
                    "client_id": client_id,
                    "channels": [0],
                    "media_type": 0,
                    "start_time": start.to_string(),
                    "end_time": end.to_string(),
                    "player_id": player_id,
                },
                "method": "get",
            }),
            Self::Thumbnail {
                client_id,
                start,
                player_id,
            } => json!({
                "download": {
                    "client_id": client_id,
                    "channels": [0],
                    "media_type": 2,
                    "start_time": start.to_string(),
                    "player_id": player_id,
                },
                "method": "get",
            }),
        };
        json!({ "type": "request", "seq": seq, "params": params })
    }
}

/// One part received from the camera.
#[derive(Debug, Clone)]
pub enum StreamPart {
    /// Decrypted MPEG-TS bytes.
    Media {
        data: Bytes,
        /// Part headers (`X-Session-Id`, `X-Data-Sequence`, …), lowercased names.
        headers: HashMap<String, String>,
    },
    /// A JSON message, e.g. the response to our request or a status notification.
    Json(Value),
    /// Anything else (e.g. `image/jpeg`).
    Other { content_type: String, data: Bytes },
}

impl StreamPart {
    /// True for the camera's "stream finished" notification (end of a download).
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Json(v)
            if v.pointer("/params/event_type").and_then(Value::as_str) == Some("stream_status")
            && v.pointer("/params/status").and_then(Value::as_str) == Some("finished"))
    }
}

/// An authenticated connection to a camera's media server.
pub struct MediaSession<S = TcpStream> {
    stream: BufReader<S>,
    cipher: StreamCipher,
    device_boundary: Vec<u8>,
    window_size: Option<u32>,
    session_id: Option<String>,
}

impl MediaSession<TcpStream> {
    /// Connects and authenticates.
    ///
    /// The camera answers the unauthenticated request with an HTTP/1.0 challenge and
    /// takes several seconds to handle a second request on that same connection, so
    /// the authenticated request goes out on a fresh one.
    pub async fn connect(config: &MediaConfig) -> Result<Self> {
        Self::connect_and_start(config, None).await
    }

    /// Connects, authenticates and sends `request` in the same round trip. Cameras hold
    /// back their answer to the authenticated request until the first request part
    /// arrives (or a ~4 s timeout passes), so sending it right away saves seconds.
    pub async fn connect_and_start(
        config: &MediaConfig,
        request: Option<&StreamRequest>,
    ) -> Result<Self> {
        let started = std::time::Instant::now();
        let mut first = BufReader::new(Self::dial(config).await?);
        let challenge = request_challenge(&mut first).await?;
        drop(first);
        tracing::debug!(elapsed = ?started.elapsed(), "media: got the Digest challenge");
        let session = Self::authenticate(
            BufReader::new(Self::dial(config).await?),
            config,
            &challenge,
            request,
        )
        .await?;
        tracing::debug!(elapsed = ?started.elapsed(), "media: authenticated");
        Ok(session)
    }

    async fn dial(config: &MediaConfig) -> Result<TcpStream> {
        let addr = (config.host.as_str(), config.port);
        let tcp = tokio::time::timeout(config.connect_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| Error::protocol("timed out connecting to the media server"))?
            .map_err(|e| Error::protocol(format!("media server connection failed: {e}")))?;
        tcp.set_nodelay(true).ok();
        Ok(tcp)
    }
}

const REQUEST_LINE: &str = "POST /stream HTTP/1.1";

/// Headers of both handshake requests. pytapo sends `Content-Length: -1`; 2026 firmware
/// (seen on a C325WB 1.4.4) drops the connection on that, while `0` (what go2rtc sends)
/// works everywhere.
fn base_headers() -> String {
    format!(
        "Content-Type: multipart/mixed;boundary={CLIENT_BOUNDARY}\r\nConnection: keep-alive\r\nContent-Length: 0\r\n"
    )
}

/// Sends the unauthenticated request and returns the Digest challenge parameters.
async fn request_challenge<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut BufReader<S>,
) -> Result<HashMap<String, String>> {
    write_all(
        stream,
        format!("{REQUEST_LINE}\r\n{}\r\n", base_headers()).as_bytes(),
    )
    .await?;
    let (status, headers) = read_head(stream).await?;
    if status != 401 {
        return Err(Error::protocol(format!(
            "expected a 401 challenge, got HTTP {status}"
        )));
    }
    skip_body(stream, &headers).await?;
    let challenge_header = headers
        .get("www-authenticate")
        .ok_or_else(|| Error::protocol("401 without WWW-Authenticate"))?;
    Ok(parse_params(
        challenge_header
            .strip_prefix("Digest")
            .unwrap_or(challenge_header),
    ))
}

impl<S: AsyncRead + AsyncWrite + Unpin> MediaSession<S> {
    /// Runs the Digest authentication and key exchange over one existing connection.
    pub async fn handshake(io: S, config: &MediaConfig) -> Result<Self> {
        let mut stream = BufReader::new(io);
        let challenge = request_challenge(&mut stream).await?;
        Self::authenticate(stream, config, &challenge, None).await
    }

    /// Sends the authenticated request for `challenge` and sets up decryption.
    async fn authenticate(
        mut stream: BufReader<S>,
        config: &MediaConfig,
        challenge: &HashMap<String, String>,
        first_request: Option<&StreamRequest>,
    ) -> Result<Self> {
        let hashed_password = hash_media_password(config.password.expose_secret(), challenge);
        let realm = challenge.get("realm").map(String::as_str).unwrap_or("");
        let nonce = challenge
            .get("nonce")
            .ok_or_else(|| Error::protocol("Digest challenge without nonce"))?;
        let mut cnonce_bytes = [0u8; 24];
        getrandom::fill(&mut cnonce_bytes).expect("OS random number generator");
        let cnonce = hex::encode(cnonce_bytes);
        let (nc, qop) = ("00000001", "auth");
        let response = digest_response(
            &config.username,
            realm,
            &hashed_password,
            "POST",
            "/stream",
            nonce,
            nc,
            &cnonce,
            qop,
        );
        let mut authorization = format!(
            "Digest username=\"{}\",realm=\"{realm}\",uri=\"/stream\",algorithm=MD5,nonce=\"{nonce}\",nc={nc},cnonce=\"{cnonce}\",qop={qop},response=\"{response}\"",
            config.username
        );
        if let Some(opaque) = challenge.get("opaque") {
            authorization.push_str(&format!(",opaque=\"{opaque}\""));
        }

        let mut message = format!(
            "{REQUEST_LINE}\r\n{}Authorization: {authorization}\r\n\r\n",
            base_headers()
        )
        .into_bytes();
        if let Some(request) = first_request {
            message.extend_from_slice(&request_part(request, config.window_size));
        }
        write_all(&mut stream, &message).await?;
        let (status, headers) = read_head(&mut stream).await?;
        match status {
            200 => {}
            401 => return Err(Error::BadCredentials),
            other => {
                return Err(Error::protocol(format!(
                    "media server answered HTTP {other}"
                )));
            }
        }

        let exchange = parse_params(
            headers
                .get("key-exchange")
                .ok_or_else(|| Error::protocol("media server sent no Key-Exchange"))?,
        );
        let cipher = StreamCipher::from_key_exchange(&exchange, &hashed_password)?;
        let device_boundary = headers
            .get("content-type")
            .and_then(|ct| {
                ct.split(';')
                    .find_map(|p| p.trim().strip_prefix("boundary="))
                    .map(|b| b.trim_matches('"').to_owned())
            })
            .unwrap_or_else(|| DEFAULT_DEVICE_BOUNDARY.to_owned());

        Ok(Self {
            stream,
            cipher,
            device_boundary: device_boundary.into_bytes(),
            window_size: config.window_size,
            session_id: None,
        })
    }

    /// Sends a stream request. Read the camera's answer and media with [`Self::next_part`].
    pub async fn start(&mut self, request: &StreamRequest) -> Result<()> {
        let message = request_part(request, self.window_size);
        write_all(&mut self.stream, &message).await
    }

    /// Reads the next part. Returns `Ok(None)` when the camera closes the connection.
    /// Errors reported by the camera in JSON responses become [`Error::Camera`].
    pub async fn next_part(&mut self) -> Result<Option<StreamPart>> {
        if !skip_past(&mut self.stream, &self.device_boundary).await? {
            return Ok(None);
        }
        let headers = read_headers(&mut self.stream).await?;
        let length: usize = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| Error::protocol("media part without Content-Length"))?;
        if length > MAX_PART_SIZE {
            return Err(Error::protocol(format!(
                "media part too large: {length} bytes"
            )));
        }
        let mut data = vec![0u8; length];
        self.stream
            .read_exact(&mut data)
            .await
            .map_err(|e| Error::protocol(format!("media stream ended mid-part: {e}")))?;

        let encrypted = headers.get("x-if-encrypt").map(String::as_str) == Some("1");
        let data = if encrypted {
            self.cipher.decrypt(&data)?
        } else {
            data
        };
        let content_type = headers.get("content-type").cloned().unwrap_or_default();

        if let Some(session) = headers.get("x-session-id") {
            self.session_id = Some(session.clone());
        }
        if let Some(seq) = headers
            .get("x-data-sequence")
            .and_then(|s| s.parse::<u64>().ok())
        {
            self.acknowledge(seq).await?;
        }

        match content_type.as_str() {
            "video/mp2t" => Ok(Some(StreamPart::Media {
                data: Bytes::from(data),
                headers,
            })),
            "application/json" => {
                let value: Value = serde_json::from_slice(&data)
                    .map_err(|e| Error::protocol(format!("invalid JSON part: {e}")))?;
                if let Some(session) = value.pointer("/params/session_id") {
                    self.session_id = Some(match session {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    });
                }
                if value.get("type").and_then(Value::as_str) == Some("response")
                    && let Some(code) = value.pointer("/params/error_code").and_then(Value::as_i64)
                    && code != 0
                {
                    return Err(Error::Camera {
                        code,
                        method: Some("stream".into()),
                    });
                }
                Ok(Some(StreamPart::Json(value)))
            }
            _ => Ok(Some(StreamPart::Other {
                content_type,
                data: Bytes::from(data),
            })),
        }
    }

    /// Download flow control: every `window` parts, tell the camera how much arrived.
    async fn acknowledge(&mut self, seq: u64) -> Result<()> {
        let (Some(window), Some(session)) = (self.window_size, self.session_id.as_deref()) else {
            return Ok(());
        };
        let window = u64::from(window);
        if window == 0 || seq == 0 || !seq.is_multiple_of(window) {
            return Ok(());
        }
        let body = br#"{"type":"notification","params":{"event_type":"stream_sequence"}}"#;
        let mut message = format!(
            "--{CLIENT_BOUNDARY}\r\nX-Session-Id: {session}\r\nX-Data-Received: {}\r\nContent-Length: {}\r\n\r\n",
            window * (seq / window),
            body.len()
        )
        .into_bytes();
        message.extend_from_slice(body);
        write_all(&mut self.stream, &message).await
    }

    /// The media session id the camera assigned, once known.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

/// A JSON request as one client multipart part.
fn request_part(request: &StreamRequest, window_size: Option<u32>) -> Vec<u8> {
    let seq = 1000 + (rand_u32() % (0x7FFF - 1000));
    let body = serde_json::to_vec(&request.to_json(seq)).expect("JSON");
    let mut head = format!(
        "--{CLIENT_BOUNDARY}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(window) = window_size {
        head.push_str(&format!("X-Data-Window-Size: {window}\r\n"));
    }
    head.push_str("\r\n");
    let mut message = head.into_bytes();
    message.extend_from_slice(&body);
    message.extend_from_slice(b"\r\n");
    message
}

fn rand_u32() -> u32 {
    let mut bytes = [0u8; 4];
    getrandom::fill(&mut bytes).expect("OS random number generator");
    u32::from_le_bytes(bytes)
}

async fn write_all<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut BufReader<S>,
    data: &[u8],
) -> Result<()> {
    let inner = stream.get_mut();
    inner
        .write_all(data)
        .await
        .map_err(|e| Error::protocol(format!("media stream write failed: {e}")))?;
    inner
        .flush()
        .await
        .map_err(|e| Error::protocol(format!("media stream write failed: {e}")))
}

async fn read_line<R: AsyncBufReadExt + Unpin>(reader: &mut R) -> Result<String> {
    let mut line = Vec::new();
    let n = reader
        .read_until(b'\n', &mut line)
        .await
        .map_err(|e| Error::protocol(format!("media stream read failed: {e}")))?;
    if n == 0 {
        return Err(Error::protocol("media server closed the connection"));
    }
    if line.len() > 64 * 1024 {
        return Err(Error::protocol("header line too long"));
    }
    Ok(String::from_utf8_lossy(&line)
        .trim_end_matches(['\r', '\n'])
        .to_owned())
}

/// Header lines until an empty line; names lowercased.
async fn read_headers<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
) -> Result<HashMap<String, String>> {
    let mut headers = HashMap::new();
    loop {
        let line = read_line(reader).await?;
        if line.is_empty() {
            if headers.is_empty() {
                // The blank line right after a boundary: headers start on the next line.
                continue;
            }
            return Ok(headers);
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
        if headers.len() > 64 {
            return Err(Error::protocol("too many headers"));
        }
    }
}

/// Status line plus headers of an HTTP response.
async fn read_head<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
) -> Result<(u16, HashMap<String, String>)> {
    let line = read_line(reader).await?;
    // Some cameras prefix the status line with junk such as "HTTP ERROR 401".
    let status_line = line
        .find("HTTP/")
        .map(|i| &line[i..])
        .ok_or_else(|| Error::protocol(format!("not an HTTP response: {line:?}")))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::protocol(format!("bad status line: {status_line:?}")))?;
    let mut headers = HashMap::new();
    loop {
        let line = read_line(reader).await?;
        if line.is_empty() {
            return Ok((status, headers));
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
}

async fn skip_body<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
    headers: &HashMap<String, String>,
) -> Result<()> {
    let length: u64 = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if length > 0 && length < 1024 * 1024 {
        let mut sink = vec![0u8; length as usize];
        reader
            .read_exact(&mut sink)
            .await
            .map_err(|e| Error::protocol(format!("media stream read failed: {e}")))?;
    }
    Ok(())
}

/// Consumes bytes up to and including `needle`. Returns `false` on a clean EOF.
async fn skip_past<R: AsyncBufReadExt + Unpin>(reader: &mut R, needle: &[u8]) -> Result<bool> {
    let mut matched = 0usize;
    let mut skipped = 0usize;
    loop {
        let buf = reader
            .fill_buf()
            .await
            .map_err(|e| Error::protocol(format!("media stream read failed: {e}")))?;
        if buf.is_empty() {
            return if matched == 0 {
                Ok(false)
            } else {
                Err(Error::protocol("media stream ended inside a boundary"))
            };
        }
        let mut consumed = 0;
        for &byte in buf {
            consumed += 1;
            if byte == needle[matched] {
                matched += 1;
                if matched == needle.len() {
                    reader.consume(consumed);
                    return Ok(true);
                }
            } else {
                matched = usize::from(byte == needle[0]);
            }
        }
        reader.consume(consumed);
        skipped += consumed;
        if skipped > MAX_PART_SIZE {
            return Err(Error::protocol("no multipart boundary found"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockModeEncrypt;
    use tokio::io::duplex;

    type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

    #[test]
    fn digest_matches_rfc2617_example() {
        let response = digest_response(
            "Mufasa",
            "testrealm@host.com",
            "Circle Of Life",
            "GET",
            "/dir/index.html",
            "dcd98b7102dd2f0e8b11d0f600bfb0c093",
            "00000001",
            "0a4f113b",
            "auth",
        );
        assert_eq!(response, "6629fae49393a05397450978507c4ef1");
    }

    #[test]
    fn parses_challenge_and_key_exchange() {
        let c = parse_params(
            r#"realm="TP-Link IP-Camera",qop="auth",nonce="abc",opaque="xyz",encrypt_type="3""#,
        );
        assert_eq!(c["realm"], "TP-Link IP-Camera");
        assert_eq!(c["encrypt_type"], "3");
        let k = parse_params(r#"username="admin" nonce="1234""#);
        assert_eq!(k["username"], "admin");
        assert_eq!(k["nonce"], "1234");
    }

    #[test]
    fn media_password_hash_follows_challenge() {
        let mut c = HashMap::new();
        assert_eq!(
            hash_media_password("password", &c),
            "5F4DCC3B5AA765D61D8327DEB882CF99"
        );
        c.insert("encrypt_type".into(), "3".into());
        assert_eq!(hash_media_password("password", &c).len(), 64);
    }

    /// A fake media server: checks the Digest response, then sends one JSON response
    /// and one encrypted TS part.
    async fn fake_camera(io: tokio::io::DuplexStream, password: &str) {
        let mut stream = BufReader::new(io);
        let (_, _) = read_request(&mut stream).await;
        let challenge = r#"Digest realm="TP-Link IP-Camera",qop="auth",nonce="N0NCE",opaque="OPQ",encrypt_type="3""#;
        stream
            .get_mut()
            .write_all(format!("HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: {challenge}\r\nContent-Length: 0\r\n\r\n").as_bytes())
            .await
            .unwrap();

        let (_, headers) = read_request(&mut stream).await;
        let auth = parse_params(headers["authorization"].strip_prefix("Digest ").unwrap());
        let hashed = hex::encode_upper(Sha256::digest(password.as_bytes()));
        let expected = digest_response(
            "admin",
            "TP-Link IP-Camera",
            &hashed,
            "POST",
            "/stream",
            "N0NCE",
            &auth["nc"],
            &auth["cnonce"],
            "auth",
        );
        if auth["response"] != expected {
            stream
                .get_mut()
                .write_all(b"HTTP/1.1 401 Unauthorized\r\n\r\n")
                .await
                .unwrap();
            return;
        }
        stream
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: multipart/mixed;boundary=--device-stream-boundary--\r\nKey-Exchange: username=\"admin\" nonce=\"KEYNONCE\"\r\n\r\n")
            .await
            .unwrap();

        // The client's request part.
        let _ = skip_past(&mut stream, CLIENT_BOUNDARY.as_bytes())
            .await
            .unwrap();
        let headers = read_headers(&mut stream).await.unwrap();
        let len: usize = headers["content-length"].parse().unwrap();
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await.unwrap();
        let request: Value = serde_json::from_slice(&body).unwrap();
        let seq = request["seq"].clone();

        let response = serde_json::to_vec(&json!({"type": "response", "seq": seq, "params": {"error_code": 0, "session_id": "7"}})).unwrap();
        let mut out = format!("--device-stream-boundary--\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", response.len()).into_bytes();
        out.extend_from_slice(&response);
        out.extend_from_slice(b"\r\n");

        let cipher = StreamCipher::from_key_exchange(
            &parse_params(r#"username="admin" nonce="KEYNONCE""#),
            &hashed,
        )
        .unwrap();
        let ts = vec![0x47u8; 188];
        let encrypted = Aes128CbcEnc::new(&cipher.key.into(), &cipher.iv.into())
            .encrypt_padded_vec::<Pkcs7>(&ts);
        out.extend_from_slice(format!("--device-stream-boundary--\r\nContent-Type: video/mp2t\r\nContent-Length: {}\r\nX-If-Encrypt: 1\r\nX-Session-Id: 7\r\n\r\n", encrypted.len()).as_bytes());
        out.extend_from_slice(&encrypted);
        out.extend_from_slice(b"\r\n");
        stream.get_mut().write_all(&out).await.unwrap();
    }

    async fn read_request<R: AsyncBufReadExt + Unpin>(
        r: &mut R,
    ) -> (String, HashMap<String, String>) {
        let line = read_line(r).await.unwrap();
        let mut headers = HashMap::new();
        loop {
            let l = read_line(r).await.unwrap();
            if l.is_empty() {
                break;
            }
            let (k, v) = l.split_once(':').unwrap();
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
        (line, headers)
    }

    #[tokio::test]
    async fn handshake_request_and_decrypt() {
        let (client, server) = duplex(64 * 1024);
        tokio::spawn(fake_camera(server, "hunter2"));

        let config = MediaConfig::new("camera", "hunter2");
        let mut session = MediaSession::handshake(client, &config).await.unwrap();
        session
            .start(&StreamRequest::Live {
                quality: Quality::High,
                channel: 0,
            })
            .await
            .unwrap();

        match session.next_part().await.unwrap().unwrap() {
            StreamPart::Json(v) => assert_eq!(v["params"]["session_id"], "7"),
            other => panic!("expected JSON, got {other:?}"),
        }
        assert_eq!(session.session_id(), Some("7"));
        match session.next_part().await.unwrap().unwrap() {
            StreamPart::Media { data, .. } => assert_eq!(&data[..], &[0x47u8; 188][..]),
            other => panic!("expected media, got {other:?}"),
        }
        assert!(session.next_part().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn wrong_password_is_rejected() {
        let (client, server) = duplex(64 * 1024);
        tokio::spawn(fake_camera(server, "correct"));
        let config = MediaConfig::new("camera", "wrong");
        assert!(matches!(
            MediaSession::handshake(client, &config).await,
            Err(Error::BadCredentials)
        ));
    }

    #[test]
    fn request_json_shapes() {
        let live = StreamRequest::Live {
            quality: Quality::Low,
            channel: 0,
        }
        .to_json(1234);
        assert_eq!(live["params"]["preview"]["resolutions"][0], "VGA");
        assert_eq!(live["seq"], 1234);
        let dl = StreamRequest::Download {
            client_id: 5,
            start: 10,
            end: 20,
            player_id: "p".into(),
        }
        .to_json(1);
        assert_eq!(dl["params"]["download"]["start_time"], "10");
        assert_eq!(dl["params"]["method"], "get");
    }
}
