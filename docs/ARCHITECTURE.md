# Architecture

```
 Tapo camera ──HTTPS :443 (control API)──┐
             ──TCP :8800 (media stream)──┤
             ──RTSP :554 (optional)──────┤
                                         ▼
 crates/tapo-camera        transports · typed API · media session · TS demux · MP4 writer
                                         ▼
 app/src-tauri             one actor per camera · MediaHub (1 media session / camera)
                           SQLite cache · OS keychain · export queue · Tauri commands
                                         ▼  Tauri Channel (raw ArrayBuffer batches)
 app/src (React)           ipc/ (BacksightApi) · player/ (Worker + WebCodecs + AudioWorklet)
                           features/ (home, live, playback, multiview, exports, settings)
```

## Contracts

- **UI ↔ backend:** `app/src/ipc/api.ts` (`BacksightApi`). Screens use nothing else.
  `app/src/ipc/mock/` implements it for browser development; `app/src/ipc/tauri.ts` wraps the
  real Tauri commands.
- **Stream layer ↔ player/exporter:** `crates/tapo-camera/src/media/mod.rs` (`MediaEvent`,
  `VideoConfig`, `VideoFrame`, `AudioConfig`, `AudioFrame`).
- **Backend → player:** the wire format below.

## Wire format (v1)

Media reaches the webview over a Tauri `Channel` as raw `ArrayBuffer`s. Each channel message is
a **batch**: one or more packets back to back. All integers are little-endian.

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | `u32` total packet length in bytes, including this 16-byte header |
| 4 | 1 | `u8` kind (see below) |
| 5 | 1 | `u8` flags: bit 0 = keyframe, bit 1 = discontinuity (reset decoders and clock) |
| 6 | 2 | `u16` reserved, 0 |
| 8 | 8 | `i64` timestamp: microseconds since the Unix epoch (UTC), 0 when not applicable |
| 16 | … | payload (`total length − 16` bytes) |

| Kind | Name | Payload |
|---|---|---|
| 1 | `VideoConfig` | `u16` JSON length, UTF-8 JSON `{"codec":"avc1.64001F","codedWidth":1920,"codedHeight":1080}`, then the decoder description (avcC / hvcC) |
| 2 | `VideoFrame` | one access unit, length-prefixed NAL units matching the description |
| 3 | `AudioConfig` | UTF-8 JSON `{"sampleRate":8000,"channels":1,"format":"s16le"}` |
| 4 | `AudioPcm` | interleaved signed 16-bit PCM |
| 5 | `Status` | UTF-8 JSON `{"state":"buffering"\|"playing"\|"ended"\|"error","code"?:string,"message"?:string}` |
| 6 | `EndOfStream` | empty |

Timestamps are wall-clock times (camera clock corrected to UTC), so the player can report its
position straight to the playback timeline. A `VideoConfig` always precedes the first
`VideoFrame` and is re-sent whenever the parameter sets change. A packet with the
discontinuity flag follows every seek.

Implementations: `app/src-tauri/src/wire.rs` (encoder) and `app/src/player/wire.ts` (decoder)
share the golden test vectors in `app/src/player/__fixtures__/`.

Rules the player relies on:

- Packets arrive in order (one channel per stream).
- A `VideoConfig` comes right before a keyframe.
- Flag every seek with the discontinuity bit. Unflagged jumps (more than 3 s forward at the
  current speed, or more than 1 s back) are treated as discontinuities too, but cost a wait for
  the next keyframe.
- Playback timestamps advance at the stream's speed; the backend paces delivery at that speed
  and the player's clock plays them at the same rate.
- Audio is only sent at 1×.
- A `Status` with state `error` is final; `buffering` shows until the next frame arrives.

## Backend (`app/src-tauri/src`)

| Module | Role |
|---|---|
| `commands.rs` | Tauri commands behind `app/src/ipc/tauri.ts`; `AppState` |
| `model.rs` | Serde types mirroring `api.ts` (camelCase JSON) |
| `error.rs` | `ApiError { code, message, retryAt }` and the camera-error → UI-code mapping |
| `cameras.rs` | Saved cameras, their clients, 60 s status polling, `backsight://event` events |
| `recordings.rs` | Days with footage and the day index (segments + detection events), cached in SQLite |
| `thumbnails.rs` | Detection thumbnails via the `thumb://<camera>/<start>` scheme, disk-cached |
| `streams.rs` | Live and playback streams to the player (1× via `playback`, other speeds via `download` paced here). Streams end when the page closes them or its webview reloads |
| `exports.rs` | Clip export jobs: `download` → MP4, one job at a time per camera, `export-progress` events |
| `audio_aac.rs` | 48 kHz upsampling + AAC encoding (Media Foundation) for exports |
| `db.rs` / `secrets.rs` | SQLite storage / OS keychain for passwords |

**Time.** The camera's clock is not necessarily UTC. Every poll records
`correction = host_utc_now − camera_seconds` and the camera's UTC offset (from its local wall
time). Recording and event times are cached in camera-clock seconds and converted to UTC when
served; requests back to the camera (playback start, thumbnails) use camera-clock seconds.

**Credentials.** Passwords live only in the OS keychain (service `Backsight`). The certificate
fingerprint seen when a camera is added is pinned in the database; a different certificate at
the same address is refused.

## Camera constraints the backend must respect

- Control requests go out one at a time per camera.
- A failed login is never retried automatically (10 failures lock the camera for ~30 min).
- The camera limits SD playback clients (`-71101`/`-71102`). A refusal shows as "Playback is
  busy" and is not retried in a loop. Live view and an SD download can run side by side (seen
  on C325WB); exports queue one at a time per camera.
- At most 3 local live viewers per camera, and RTSP clients such as NVR software count.
