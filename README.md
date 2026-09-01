# Backsight

**A desktop viewer for Tapo cameras** — live view, SD-card playback on a timeline, detection
events, clip export and a multi-camera grid, on Windows, macOS and Linux.

TP-Link's Tapo app only lets you browse a camera's SD-card recordings on a phone. Backsight
brings that to the desktop. It talks to your cameras directly over your local network; nothing
goes through a cloud service.

> **Status:** early development. Not usable yet.

## Features (v1 goals)

- Live view with HD/SD quality switching, snapshots and fullscreen
- SD-card playback on a zoomable timeline, with a calendar of days that have footage
- Detection events (motion, person, vehicle, pet…) with thumbnails and filters
- Clip & download to MP4 at up to ~10× real time
- Multi-view grid (1, 2, 4, 1+5, 9, 16 cameras)

Not planned for v1: pan/tilt control, two-way audio.

## Requirements

- Tapo cameras reachable on your LAN, with an SD card for recordings
- **Third-Party Compatibility** turned on in the Tapo app (Me → Third-Party Services)
- Your TP-Link account password (checked locally by the camera). Optionally the camera's
  "Camera Account" (the RTSP/ONVIF login) for live view over RTSP.

## Repository layout

| Path | What |
|---|---|
| `crates/tapo-camera` | Rust library for the cameras' local protocols (a port of pytapo) |
| `crates/tapo-cli` | `tapo` command-line tool for testing the library |
| `crates/tapo-mock` | A fake camera for tests and UI development |
| `app/` | The Tauri 2 desktop app (Rust backend in `app/src-tauri`, React UI in `app/src`) |
| `docs/` | Architecture notes |

## Development

Prerequisites: Rust (stable), Node.js 22+, and the [Tauri prerequisites](https://tauri.app/start/prerequisites/)
for your OS.

```bash
cd app
npm install
npm run tauri dev
```

The UI can also run in a normal browser against mock data: `npm run dev` inside `app/`.

## Credits

Backsight stands on the shoulders of community reverse-engineering work, in particular
[pytapo](https://github.com/JurajNyiri/pytapo) and the
[Home Assistant Tapo integration](https://github.com/JurajNyiri/HomeAssistant-Tapo-Control) by Juraj Nyíri,
[go2rtc](https://github.com/AlexxIT/go2rtc), the [`tapo`](https://github.com/mihai-dinculescu/tapo) crate and
[freeKC/tapo-v4-protocol](https://github.com/freeKC/tapo-v4-protocol). See [NOTICE](NOTICE).

## Disclaimer

Backsight is an independent, unofficial project. It is **not affiliated with, endorsed by, or
supported by TP-Link**. "Tapo" and "TP-Link" are trademarks of their respective owners and are
used here only to describe compatibility. Backsight uses the cameras' local interfaces for
interoperability, with credentials you own. It is provided as is, without warranty of any kind;
use it at your own risk.

## License

[MIT](LICENSE)
