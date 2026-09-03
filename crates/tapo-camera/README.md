# tapo-camera

Unofficial Rust client for TP-Link Tapo cameras on your local network: log in, read
device info, list SD-card recordings and detection events, and receive the media stream
used for live view, playback and downloads. It is a port of
[pytapo](https://github.com/JurajNyiri/pytapo) and powers the
[Backsight](../../README.md) desktop app.

```rust
use tapo_camera::{Camera, CameraConfig};

# async fn demo() -> tapo_camera::Result<()> {
// The camera owner's TP-Link account password (checked locally by the camera).
let camera = Camera::connect(CameraConfig::new("192.168.1.20", "password")).await?;
let info = camera.device_info().await?;
println!("{} running {}", info["device_model"], info["sw_version"]);

// Pin the certificate for next time.
let fingerprint = camera.certificate().await;
# Ok(()) }
```

## What it supports

- **Discovery** of Tapo devices on the LAN (UDP 20002), including the login scheme each
  camera speaks.
- **Control API** over HTTPS with the "secure connection" login (`encrypt_type` 3), which
  most current firmware uses, and the older plain login. TLS certificates are pinned on
  first use instead of accepting any certificate.
- **Media stream** on port 8800: live view (main and sub stream), SD-card playback,
  fast downloads (about 10× real time) and recording thumbnails.
- **Media helpers**: MPEG-TS demuxing into H.264/H.265 access units and decoder
  configuration, G.711 decoding, MP4 export.

Firmware that only offers the newer TPAP login (reported as `encrypt_type` 4 by discovery)
is not supported; connecting to it returns [`Error::UnsupportedLogin`].

Requirements on the camera side: **Third-Party Compatibility** must be on in the Tapo app
(Me → Third-Party Services).

## Being a good citizen on the camera

- Requests to one camera are sent one at a time.
- A rejected password is never retried. Cameras lock logins for about 30 minutes after
  10 failures, and the client detects a wrong password before it counts as a failed
  attempt where the protocol allows it.
- Cameras allow one media session at a time and a few live viewers (RTSP clients count).

## Disclaimer

This crate is not affiliated with, endorsed by, or supported by TP-Link. "Tapo" and
"TP-Link" are trademarks of their respective owners. It uses the cameras' local interfaces
for interoperability, with credentials you own, and comes with no warranty.

## License

MIT. Portions are derived from MIT-licensed projects; see
[NOTICE](../../NOTICE).
