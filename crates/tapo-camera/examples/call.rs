//! Calls read-only control API methods on a camera saved with the `login` example and
//! prints the JSON results.
//!
//! ```text
//! cargo run -p tapo-camera --example call -- <camera-ip> <command> [args]
//!
//! commands:
//!   info | clock | tz | dst | sd | audio | privacy | caps | userid
//!   days <YYYYMMDD> <YYYYMMDD>     days that have recordings
//!   day <YYYYMMDD>                 recordings of one day
//!   events <start> <end>           detection events (camera-clock unix seconds)
//!   <getSomething|searchSomething> [params-json]   any read-only method
//! ```

use keyring::v1::Entry;
use serde_json::Value;
use tapo_camera::{Camera, CameraConfig, CertFingerprint};

const SERVICE: &str = "backsight-dev";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let [host, command, rest @ ..] = args.as_slice() else {
        return Err("usage: call <camera-ip> <command> [args]".into());
    };
    let password = Entry::new(SERVICE, host)?
        .get_password()
        .map_err(|_| format!("no saved password for {host}; run the login example first"))?;
    let pin = Entry::new(SERVICE, &format!("{host}#cert"))?
        .get_password()
        .ok()
        .and_then(|hex| CertFingerprint::from_hex(&hex));
    let camera = Camera::new(CameraConfig::new(host, password).with_certificate(pin))?;

    let arg = |i: usize| rest.get(i).map(String::as_str).ok_or("missing argument");
    let result: Value = match command.as_str() {
        "info" => camera.device_info().await?,
        "clock" => camera.clock_status().await?,
        "tz" => camera.timezone().await?,
        "dst" => camera.dst_rule().await?,
        "sd" => camera.sd_cards().await?,
        "audio" => camera.audio_config().await?,
        "privacy" => camera.privacy_mode().await?,
        "caps" => camera.video_capability().await?,
        "userid" => camera.user_id().await?.into(),
        "days" => camera.days_with_recordings(arg(0)?, arg(1)?).await?,
        "day" => {
            let user_id = camera.user_id().await?;
            camera.recordings_of_day(arg(0)?, user_id).await?
        }
        "events" => {
            camera
                .detection_events(arg(0)?.parse()?, arg(1)?.parse()?)
                .await?
        }
        method if method.starts_with("get") || method.starts_with("search") => {
            let params = match rest.first() {
                Some(json) => serde_json::from_str(json)?,
                None => Value::Object(Default::default()),
            };
            camera.execute(method, params).await?
        }
        other => return Err(format!("unknown or non-read-only command: {other}").into()),
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
