//! The HTTPS control API: secure-connection login and encrypted `securePassthrough`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use jiff::{Timestamp, civil::Date};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::{self, pki_types::PrivateKeyDer};

use crate::MockOptions;
use crate::crypto::{self, sha256_upper};
use crate::data;

const MAX_FAILURES: u32 = 10;
const LOCKOUT: Duration = Duration::from_secs(30 * 60);

struct Session {
    lsk: [u8; 16],
    ivb: [u8; 16],
    cnonce: String,
    seq: u64,
}

#[derive(Default)]
struct State {
    pending: HashMap<String, String>,
    sessions: HashMap<String, Session>,
    failures: u32,
    locked_until: Option<Instant>,
}

struct Camera {
    options: Arc<MockOptions>,
    hashed: String,
    state: Mutex<State>,
}

fn tls_acceptor() -> anyhow::Result<TlsAcceptor> {
    let cert = rcgen::generate_simple_self_signed(vec!["TPRI-DEVICE".into()])?;
    let key = PrivateKeyDer::try_from(cert.signing_key.serialize_der())
        .map_err(|e| anyhow::anyhow!("key: {e}"))?;
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS12])?
    .with_no_client_auth()
    .with_single_cert(vec![cert.cert.der().clone()], key)?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub async fn serve(listener: TcpListener, options: Arc<MockOptions>) {
    let acceptor = match tls_acceptor() {
        Ok(acceptor) => acceptor,
        Err(err) => {
            tracing::error!(%err, "TLS setup failed");
            return;
        }
    };
    let camera = Arc::new(Camera {
        hashed: crypto::sha256_hex_upper(&options.password),
        options,
        state: Mutex::new(State::default()),
    });
    loop {
        let Ok((tcp, _)) = listener.accept().await else {
            continue;
        };
        let acceptor = acceptor.clone();
        let camera = camera.clone();
        tokio::spawn(async move {
            match acceptor.accept(tcp).await {
                Ok(tls) => {
                    if let Err(err) = handle_connection(tls, &camera).await {
                        tracing::debug!(%err, "control connection ended");
                    }
                }
                Err(err) => tracing::debug!(%err, "TLS handshake failed"),
            }
        });
    }
}

struct Request {
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn read_request<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
) -> anyhow::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).await?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;
    Ok(Some(Request {
        path,
        headers,
        body,
    }))
}

async fn handle_connection<S>(stream: S, camera: &Camera) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(stream);
    while let Some(request) = read_request(&mut reader).await? {
        let (status, body) = camera.handle(&request);
        let body = serde_json::to_vec(&body)?;
        let head = format!(
            "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
            if status == 200 { "OK" } else { "Error" },
            body.len()
        );
        let stream = reader.get_mut();
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(&body).await?;
        stream.flush().await?;
    }
    Ok(())
}

impl Camera {
    fn handle(&self, request: &Request) -> (u16, Value) {
        let Ok(body) = serde_json::from_slice::<Value>(&request.body) else {
            return (400, json!({ "error_code": -40210 }));
        };
        if request.path == "/" {
            return (200, self.login(&body));
        }
        match request
            .path
            .strip_prefix("/stok=")
            .and_then(|p| p.strip_suffix("/ds"))
        {
            Some(stok) => self.passthrough(stok, request, &body),
            None => (404, json!({ "error_code": -40210 })),
        }
    }

    fn login(&self, body: &Value) -> Value {
        let params = &body["params"];
        let Some(cnonce) = params["cnonce"].as_str() else {
            return json!({ "error_code": -40210 });
        };
        let mut state = self.state.lock().expect("state");
        if let Some(until) = state.locked_until {
            let left = until.saturating_duration_since(Instant::now()).as_secs();
            if left > 0 {
                return json!({ "error_code": -40404, "result": { "data": {
                    "code": -40404, "sec_left": left, "time": MAX_FAILURES, "max_time": MAX_FAILURES
                } } });
            }
            state.locked_until = None;
            state.failures = 0;
        }

        let Some(digest) = params["digest_passwd"].as_str() else {
            let nonce = crypto::random_hex(8, true);
            let proof =
                sha256_upper(&[cnonce.as_bytes(), self.hashed.as_bytes(), nonce.as_bytes()]);
            state.pending.insert(cnonce.to_owned(), nonce.clone());
            return json!({ "error_code": -40413, "result": { "data": {
                "code": -40401,
                "encrypt_type": ["3"],
                "nonce": nonce,
                "device_confirm": format!("{proof}{nonce}{cnonce}"),
            } } });
        };

        let Some(nonce) = state.pending.remove(cnonce) else {
            return json!({ "error_code": -40413 });
        };
        let expected = format!(
            "{}{cnonce}{nonce}",
            sha256_upper(&[self.hashed.as_bytes(), cnonce.as_bytes(), nonce.as_bytes()])
        );
        if digest != expected {
            state.failures += 1;
            if state.failures >= MAX_FAILURES {
                state.locked_until = Some(Instant::now() + LOCKOUT);
            }
            return json!({ "error_code": -40401, "result": { "data": {
                "code": -40411, "time": state.failures, "max_time": MAX_FAILURES, "sec_left": 0
            } } });
        }
        state.failures = 0;
        let stok = crypto::random_hex(16, false);
        let start_seq = 1000;
        state.sessions.insert(
            stok.clone(),
            Session {
                lsk: crypto::derive_key("lsk", cnonce, &self.hashed, &nonce),
                ivb: crypto::derive_key("ivb", cnonce, &self.hashed, &nonce),
                cnonce: cnonce.to_owned(),
                seq: start_seq,
            },
        );
        json!({ "error_code": 0, "result": { "stok": stok, "user_group": "root", "start_seq": start_seq } })
    }

    fn passthrough(&self, stok: &str, request: &Request, body: &Value) -> (u16, Value) {
        let mut state = self.state.lock().expect("state");
        let Some(session) = state.sessions.get_mut(stok) else {
            return (500, json!({ "error_code": -40401 }));
        };
        let seq: u64 = request
            .headers
            .get("seq")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let inner_tag = sha256_upper(&[self.hashed.as_bytes(), session.cnonce.as_bytes()]);
        let expected_tag = sha256_upper(&[
            inner_tag.as_bytes(),
            &request.body,
            seq.to_string().as_bytes(),
        ]);
        if seq != session.seq || request.headers.get("tapo_tag") != Some(&expected_tag) {
            return (200, json!({ "error_code": -1 }));
        }
        session.seq += 1;

        let Some(encoded) = body.pointer("/params/request").and_then(Value::as_str) else {
            return (200, json!({ "error_code": -40210 }));
        };
        let Some(plain) = BASE64
            .decode(encoded)
            .ok()
            .and_then(|ct| crypto::decrypt(session.lsk, session.ivb, &ct))
        else {
            return (200, json!({ "error_code": -1005 }));
        };
        let (lsk, ivb) = (session.lsk, session.ivb);
        drop(state);

        let inner: Value = serde_json::from_slice(&plain).unwrap_or(Value::Null);
        let response = self.execute(&inner);
        let encrypted = crypto::encrypt(lsk, ivb, &serde_json::to_vec(&response).expect("JSON"));
        (
            200,
            json!({ "seq": seq, "error_code": 0, "result": { "response": BASE64.encode(encrypted) } }),
        )
    }

    fn execute(&self, request: &Value) -> Value {
        if request["method"] != "multipleRequest" {
            return json!({ "error_code": -40210 });
        }
        let responses: Vec<Value> = request
            .pointer("/params/requests")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|call| {
                let method = call["method"].as_str().unwrap_or("");
                match self.method(method, &call["params"]) {
                    Ok(result) => json!({ "method": method, "result": result, "error_code": 0 }),
                    Err(code) => json!({ "method": method, "error_code": code }),
                }
            })
            .collect();
        json!({ "error_code": 0, "result": { "responses": responses } })
    }

    fn method(&self, method: &str, params: &Value) -> Result<Value, i64> {
        let options = &self.options;
        Ok(match method {
            "getDeviceInfo" => json!({ "device_info": { "basic_info": {
                "device_type": "SMART.IPCAMERA",
                "device_model": options.model,
                "device_alias": options.name,
                "device_name": options.model,
                "hw_version": "1.0",
                "sw_version": "1.0.0 Build 260101 Rel.00001n (mock)",
                "mac": "02-00-00-00-00-01",
                "dev_id": "0000MOCK",
            } } }),
            "getClockStatus" => {
                let now = Timestamp::now();
                json!({ "system": { "clock_status": {
                    "seconds_from_1970": now.as_second(),
                    "local_time": now.to_zoned(data::zone()).strftime("%Y-%m-%d %H:%M:%S").to_string(),
                } } })
            }
            "getTimezone" => {
                json!({ "system": { "basic": { "zone_id": data::zone().iana_name().unwrap_or("UTC") } } })
            }
            "getSdCardStatus" => json!({ "harddisk_manage": { "hd_info": [ { "hd_info_1": {
                "status": "normal", "total_space": "59.5GB", "free_space": "21.3GB",
                "detect_status": "normal", "record_duration": "103612"
            } } ] } }),
            "getLensMaskConfig" => {
                json!({ "lens_mask": { "lens_mask_info": { "enabled": "off" } } })
            }
            "getAudioConfig" => json!({ "audio_config": {
                "microphone": { "sampling_rate": "8", "encode_type": "G711alaw", "volume": "80" },
                "speaker": { "volume": "80" },
                "record_audio": { "enabled": "on" },
            } }),
            "getUserID" => json!({ "user_id": 42 }),
            "searchDateWithVideo" => {
                let q = &params["playback"]["search_year_utility"];
                json!({ "playback": { "search_results": data::days_with_recordings(
                    q["start_date"].as_str().unwrap_or(""),
                    q["end_date"].as_str().unwrap_or(""),
                ) } })
            }
            "searchVideoOfDay" => {
                let q = &params["playback"]["search_video_utility"];
                if q["id"].as_u64() != Some(42) {
                    return Err(-71103);
                }
                let date = q["date"]
                    .as_str()
                    .and_then(|d| Date::strptime("%Y%m%d", d).ok())
                    .ok_or(-40210)?;
                let results: Vec<Value> = data::recordings_of(date)
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        json!({ format!("search_video_results_{}", i + 1): {
                        "startTime": r.start, "endTime": r.end, "vedio_type": r.video_type
                    } })
                    })
                    .collect();
                json!({ "playback": { "search_video_results": results } })
            }
            "searchDetectionList" => {
                let q = &params["playback"]["search_detection_list"];
                let start = q["start_time"].as_i64().unwrap_or(0);
                let end = q["end_time"].as_i64().unwrap_or(0);
                let events: Vec<Value> = data::recordings_between(start, end)
                    .iter()
                    .filter(|r| r.video_type != 1)
                    .map(|r| json!({ "start_time": r.start, "end_time": r.end, "event_type": [r.event_type] }))
                    .collect();
                json!({ "playback": { "search_detection_list": events } })
            }
            "getVideoCapability" => json!({ "video_capability": {
                "main": { "resolutions": ["640*360"], "encode_types": ["H264"] },
                "minor": { "resolutions": ["640*360"], "encode_types": ["H264"] },
            } }),
            _ => return Err(-40106),
        })
    }
}
