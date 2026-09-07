//! Saves a camera's media stream to an MPEG-TS file (camera saved with the `login`
//! example).
//!
//! ```text
//! cargo run -p tapo-camera --example stream -- <ip> live <hd|sd> <seconds> <out.mpegts>
//! cargo run -p tapo-camera --example stream -- <ip> playback <start> <seconds> <out.mpegts>
//! cargo run -p tapo-camera --example stream -- <ip> download <start> <end> <out.mpegts>
//! ```
//! `start`/`end` are camera-clock unix seconds (as listed by `call <ip> day <date>`).

use std::time::{Duration, Instant};

use keyring::v1::Entry;
use tapo_camera::stream::{MediaConfig, MediaSession, Quality, StreamPart, StreamRequest};
use tapo_camera::{Camera, CameraConfig, CertFingerprint};
use tokio::io::AsyncWriteExt;

const SERVICE: &str = "backsight-dev";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let [host, mode, a, b, out] = args.as_slice() else {
        return Err("usage: stream <ip> live|playback|download <arg> <arg> <out.mpegts>".into());
    };
    let password = Entry::new(SERVICE, host)?
        .get_password()
        .map_err(|_| format!("no saved password for {host}; run the login example first"))?;

    let mut media_config = MediaConfig::new(host, password.clone());
    let (request, limit) = match mode.as_str() {
        "live" => {
            let quality = if a == "sd" {
                Quality::Low
            } else {
                Quality::High
            };
            (
                StreamRequest::Live {
                    quality,
                    channel: 0,
                },
                Some(Duration::from_secs(b.parse()?)),
            )
        }
        "playback" | "download" => {
            let pin = Entry::new(SERVICE, &format!("{host}#cert"))?
                .get_password()
                .ok()
                .and_then(|hex| CertFingerprint::from_hex(&hex));
            let camera = Camera::new(CameraConfig::new(host, password).with_certificate(pin))?;
            let client_id = camera.user_id().await?;
            let start: i64 = a.parse()?;
            if mode == "playback" {
                let seconds: u64 = b.parse()?;
                (
                    StreamRequest::Playback {
                        client_id,
                        start,
                        end: start + seconds as i64,
                    },
                    Some(Duration::from_secs(seconds)),
                )
            } else {
                media_config.window_size = Some(200);
                let player_id = format!("backsight-{:08x}", std::process::id());
                (
                    StreamRequest::Download {
                        client_id,
                        start,
                        end: b.parse()?,
                        player_id,
                    },
                    None,
                )
            }
        }
        other => return Err(format!("unknown mode {other}").into()),
    };

    let started = Instant::now();
    let mut session = MediaSession::connect_and_start(&media_config, Some(&request)).await?;
    println!(
        "connected, authenticated and requested in {:?}",
        started.elapsed()
    );

    let mut file = tokio::fs::File::create(out).await?;
    let (mut parts, mut bytes, mut printed_headers) = (0usize, 0usize, false);
    let stream_started = Instant::now();
    loop {
        if limit.is_some_and(|l| stream_started.elapsed() >= l) {
            break;
        }
        let part = match tokio::time::timeout(Duration::from_secs(15), session.next_part()).await {
            Ok(part) => part?,
            Err(_) => {
                println!("no data for 15 s, stopping");
                break;
            }
        };
        match part {
            None => {
                println!("camera closed the stream");
                break;
            }
            Some(StreamPart::Media { data, headers }) => {
                if !printed_headers {
                    let mut keys: Vec<_> = headers.iter().collect();
                    keys.sort();
                    println!("first media part headers: {keys:?}");
                    printed_headers = true;
                }
                parts += 1;
                bytes += data.len();
                file.write_all(&data).await?;
            }
            Some(part @ StreamPart::Json(_)) => {
                let finished = part.is_finished();
                if let StreamPart::Json(value) = part {
                    println!("json: {value}");
                }
                if finished {
                    break;
                }
            }
            Some(StreamPart::Other { content_type, data }) => {
                println!("other part: {content_type} ({} bytes)", data.len());
            }
        }
    }
    file.flush().await?;
    let secs = stream_started.elapsed().as_secs_f64();
    println!(
        "{parts} media parts, {bytes} bytes in {secs:.1} s ({:.0} kbit/s) -> {out}",
        bytes as f64 * 8.0 / 1000.0 / secs.max(0.001)
    );
    Ok(())
}
