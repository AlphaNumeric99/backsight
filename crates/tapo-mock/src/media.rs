//! The port-8800 media server: Digest auth, key exchange, and encrypted multipart parts
//! carrying the fixture video.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::MockOptions;
use crate::crypto::{self, md5_hex};
use crate::data;

const REALM: &str = "TP-Link IP-Camera";
const TS_PACKET: usize = 188;
/// 50 TS packets per part, about what cameras send.
const PART_PACKETS: usize = 50;

/// Digest nonces handed out, valid on any later connection (like real cameras).
type Nonces = Arc<Mutex<HashSet<String>>>;

pub async fn serve(listener: TcpListener, options: Arc<MockOptions>) {
    let nonces: Nonces = Arc::default();
    loop {
        let Ok((tcp, _)) = listener.accept().await else {
            continue;
        };
        let options = options.clone();
        let nonces = nonces.clone();
        tokio::spawn(async move {
            if let Err(err) = handle(tcp, &options, &nonces).await {
                tracing::debug!(%err, "media connection ended");
            }
        });
    }
}

fn parse_params(input: &str) -> HashMap<String, String> {
    input
        .split(',')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((k.trim().to_owned(), v.trim().trim_matches('"').to_owned()))
        })
        .collect()
}

async fn read_head<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
) -> anyhow::Result<HashMap<String, String>> {
    let mut headers = HashMap::new();
    let mut first = true;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            anyhow::bail!("closed");
        }
        let line = line.trim_end();
        if line.is_empty() {
            if first {
                continue;
            }
            return Ok(headers);
        }
        first = false;
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
}

struct Cipher {
    key: [u8; 16],
    iv: [u8; 16],
}

async fn handle(tcp: TcpStream, options: &MockOptions, nonces: &Nonces) -> anyhow::Result<()> {
    tcp.set_nodelay(true).ok();
    let (read, mut writer) = tcp.into_split();
    let mut reader = BufReader::new(read);

    // Requests without credentials get a Digest challenge; the authenticated retry may
    // come on this connection or a new one.
    let headers = loop {
        let headers = read_head(&mut reader).await?;
        if headers.contains_key("authorization") {
            break headers;
        }
        if headers.get("content-length").map(String::as_str) == Some("-1") {
            // Like 2026 firmware: drop the connection.
            return Ok(());
        }
        let nonce = crypto::random_hex(16, false);
        nonces.lock().expect("nonces").insert(nonce.clone());
        let opaque = crypto::random_hex(8, false);
        writer
            .write_all(
                format!(
                    "HTTP/1.0 401 Unauthorized\r\nWWW-Authenticate: Digest realm=\"{REALM}\",algorithm=\"MD5\",encrypt_type=\"3\",qop=\"auth\",nonce=\"{nonce}\",opaque=\"{opaque}\"\r\n\r\n"
                )
                .as_bytes(),
            )
            .await?;
    };
    let auth = parse_params(
        headers
            .get("authorization")
            .and_then(|a| a.strip_prefix("Digest"))
            .unwrap_or(""),
    );
    let hashed = crypto::sha256_hex_upper(&options.password);
    let get = |k: &str| auth.get(k).map(String::as_str).unwrap_or("");
    let ha1 = md5_hex(&format!("{}:{REALM}:{hashed}", get("username")));
    let ha2 = md5_hex("POST:/stream");
    let expected = md5_hex(&format!(
        "{ha1}:{}:{}:{}:{}:{ha2}",
        get("nonce"),
        get("nc"),
        get("cnonce"),
        get("qop")
    ));
    let known_nonce = nonces.lock().expect("nonces").contains(get("nonce"));
    if get("response") != expected || !known_nonce {
        writer
            .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n")
            .await?;
        return Ok(());
    }
    let kx_nonce = crypto::random_hex(16, false);
    writer
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: multipart/mixed;boundary=--device-stream-boundary--\r\nConnection: keep-alive\r\nKey-Exchange: username=\"admin\" nonce=\"{kx_nonce}\"\r\n\r\n"
            )
            .as_bytes(),
        )
        .await?;
    let cipher = Cipher {
        key: crypto::md5_bytes(&format!("{kx_nonce}:{hashed}")),
        iv: crypto::md5_bytes(&format!("admin:{kx_nonce}")),
    };

    // Client parts arrive on their own task so new requests can interrupt a stream.
    let (tx, mut rx) = mpsc::channel::<Value>(16);
    tokio::spawn(async move {
        loop {
            match read_client_part(&mut reader).await {
                Ok(Some(value)) => {
                    if tx.send(value).await.is_err() {
                        break;
                    }
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
    });

    let mut next = rx.recv().await;
    let mut session = 0u32;
    while let Some(message) = next.take() {
        if message["type"] != "request" {
            next = rx.recv().await;
            continue;
        }
        session += 1;
        let seq = message["seq"].clone();
        send_json(
            &mut writer,
            &json!({ "type": "response", "seq": seq, "params": { "error_code": 0, "session_id": session.to_string() } }),
        )
        .await?;
        next = run_job(
            &mut writer,
            &mut rx,
            &message["params"],
            session,
            &cipher,
            options,
        )
        .await?;
        if next.is_none() {
            next = rx.recv().await;
        }
    }
    Ok(())
}

/// Reads one client part; returns its JSON (acks and other parts yield `None`).
async fn read_client_part<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
) -> anyhow::Result<Option<Value>> {
    // Skip to the boundary.
    let mut window = Vec::new();
    let needle = b"--client-stream-boundary--";
    loop {
        let byte = reader.read_u8().await?;
        window.push(byte);
        if window.len() > needle.len() {
            window.remove(0);
        }
        if window == needle {
            break;
        }
    }
    let headers = read_head(reader).await?;
    let length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length.min(1 << 20)];
    reader.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body).ok())
}

async fn send_part(
    writer: &mut OwnedWriteHalf,
    content_type: &str,
    body: &[u8],
    extra: &str,
) -> std::io::Result<()> {
    let mut out = format!(
        "--device-stream-boundary--\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{extra}\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out.extend_from_slice(b"\r\n");
    writer.write_all(&out).await
}

async fn send_json(writer: &mut OwnedWriteHalf, value: &Value) -> std::io::Result<()> {
    send_part(
        writer,
        "application/json",
        &serde_json::to_vec(value).expect("JSON"),
        "",
    )
    .await
}

async fn send_finished(writer: &mut OwnedWriteHalf) -> std::io::Result<()> {
    send_json(
        writer,
        &json!({ "type": "notification", "params": { "event_type": "stream_status", "status": "finished" } }),
    )
    .await
}

fn parse_time(value: &Value) -> i64 {
    value
        .as_i64()
        .or_else(|| value.as_str()?.parse().ok())
        .unwrap_or(0)
}

/// Streams what a request asked for. Returns the next client message if one arrived
/// and interrupted the stream.
async fn run_job(
    writer: &mut OwnedWriteHalf,
    rx: &mut mpsc::Receiver<Value>,
    params: &Value,
    session: u32,
    cipher: &Cipher,
    options: &MockOptions,
) -> anyhow::Result<Option<Value>> {
    let fixture = &options.fixture;

    if let Some(download) = params.get("download") {
        let start = parse_time(&download["start_time"]);
        if download["media_type"].as_i64() == Some(2) {
            let has_thumbnail = data::recordings_between(start - 1, start + 1)
                .iter()
                .any(|r| r.start == start && r.video_type != 1);
            if has_thumbnail {
                let encrypted = crypto::encrypt(cipher.key, cipher.iv, &fixture.thumbnail);
                send_part(
                    writer,
                    "image/jpeg",
                    &encrypted,
                    &format!("X-If-Encrypt: 1\r\nX-Session-Id: {session}\r\n"),
                )
                .await?;
                send_finished(writer).await?;
            }
            return Ok(None);
        }
        let end = parse_time(&download["end_time"]);
        let seconds = (end - start).max(1) as f64;
        return stream_video(writer, rx, session, cipher, fixture, 10.0, Some(seconds)).await;
    }

    // Live preview and playback: real-time until interrupted.
    stream_video(writer, rx, session, cipher, fixture, 1.0, None).await
}

async fn stream_video(
    writer: &mut OwnedWriteHalf,
    rx: &mut mpsc::Receiver<Value>,
    session: u32,
    cipher: &Cipher,
    fixture: &data::Fixture,
    speed: f64,
    duration: Option<f64>,
) -> anyhow::Result<Option<Value>> {
    let bytes_per_second = fixture.video.len() as f64 / fixture.video_seconds;
    let part_len = TS_PACKET * PART_PACKETS;
    let interval = Duration::from_secs_f64(part_len as f64 / bytes_per_second / speed);
    let total_bytes = duration.map(|d| (d * bytes_per_second) as usize);

    let mut ticker = tokio::time::interval(interval);
    let mut offset = 0usize;
    let mut sent = 0usize;
    let mut loops: i64 = 0;
    let mut sequence = 0u64;
    loop {
        tokio::select! {
            message = rx.recv() => match message {
                // An ack keeps the stream going; anything else replaces it.
                Some(m) if m["type"] == "notification" => continue,
                Some(m) => return Ok(Some(m)),
                None => return Ok(None),
            },
            _ = ticker.tick() => {}
        }
        if total_bytes.is_some_and(|t| sent >= t) {
            send_finished(writer).await?;
            return Ok(None);
        }
        let end = (offset + part_len).min(fixture.video.len());
        let mut chunk = fixture.video[offset..end].to_vec();
        // Keep timestamps increasing across loops of the fixture.
        shift_timestamps(
            &mut chunk,
            loops * (fixture.video_seconds * 90_000.0) as i64,
        );
        offset = end;
        if offset >= fixture.video.len() {
            offset = 0;
            loops += 1;
        }
        sequence += 1;
        sent += chunk.len();
        let encrypted = crypto::encrypt(cipher.key, cipher.iv, &chunk);
        let extra = format!(
            "X-If-Encrypt: 1\r\nX-Session-Id: {session}\r\nX-Data-Sequence: {sequence}\r\n"
        );
        send_part(writer, "video/mp2t", &encrypted, &extra).await?;
    }
}

/// Adds `offset` (90 kHz) to every PTS/DTS and PCR in a buffer of TS packets.
fn shift_timestamps(buf: &mut [u8], offset: i64) {
    if offset == 0 {
        return;
    }
    for packet in buf.as_chunks_mut::<TS_PACKET>().0 {
        if packet[0] != 0x47 {
            continue;
        }
        let pusi = packet[1] & 0x40 != 0;
        let adaptation = (packet[3] >> 4) & 0x3;
        let mut payload = 4;
        if adaptation & 0x2 != 0 {
            let len = packet[4] as usize;
            if len > 0 && packet[5] & 0x10 != 0 && len >= 7 {
                // PCR: 33-bit base + 6 reserved + 9-bit extension.
                let p = &mut packet[6..12];
                let base = (u64::from(p[0]) << 25)
                    | (u64::from(p[1]) << 17)
                    | (u64::from(p[2]) << 9)
                    | (u64::from(p[3]) << 1)
                    | (u64::from(p[4]) >> 7);
                let base = (base as i64 + offset) as u64 & 0x1_FFFF_FFFF;
                p[0] = (base >> 25) as u8;
                p[1] = (base >> 17) as u8;
                p[2] = (base >> 9) as u8;
                p[3] = (base >> 1) as u8;
                p[4] = (p[4] & 0x7F) | (((base & 1) as u8) << 7);
            }
            payload = 5 + len;
        }
        if !pusi || adaptation & 0x1 == 0 || payload + 14 > TS_PACKET {
            continue;
        }
        let pes = &mut packet[payload..];
        if pes[0..3] != [0, 0, 1] {
            continue;
        }
        let flags = pes[7] >> 6;
        let mut shift = |at: usize| {
            let t = &mut pes[at..at + 5];
            let value = (u64::from(t[0] >> 1) & 0x07) << 30
                | u64::from(t[1]) << 22
                | u64::from(t[2] >> 1) << 15
                | u64::from(t[3]) << 7
                | u64::from(t[4] >> 1);
            let value = (value as i64 + offset) as u64 & 0x1_FFFF_FFFF;
            t[0] = (t[0] & 0xF1) | (((value >> 30) as u8 & 0x07) << 1);
            t[1] = (value >> 22) as u8;
            t[2] = (t[2] & 0x01) | (((value >> 15) as u8) << 1);
            t[3] = (value >> 7) as u8;
            t[4] = (t[4] & 0x01) | ((value as u8) << 1);
        };
        if flags & 0x2 != 0 {
            shift(9);
        }
        if flags == 0x3 {
            shift(14);
        }
    }
}
