//! Live view and SD-card playback, streamed to the player over a Tauri channel in the
//! wire format (`wire.rs`).
//!
//! Each stream is one task: camera media session → MPEG-TS demuxer → wire packets. Times
//! sent to the player are wall-clock microseconds (UTC), so the playback timeline can
//! follow the picture directly.
//!
//! Speeds: 1× playback uses the camera's `playback` request, which the camera paces.
//! Other speeds use the `download` request (the camera sends ~7× real time with every
//! frame) and pace frames here; the download window acknowledgements turn our pacing
//! into backpressure on the camera. Above 4× only keyframes are sent. Audio plays at 1×.
//!
//! A stream runs until the page closes it, the camera ends it, or the webview that opened it
//! loads a new document (see `close_webview`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tapo_camera::media::{MediaEvent, TsDemuxer, g711};
use tapo_camera::stream::{
    MediaConfig, MediaSession, Quality, StreamPart, StreamRequest as CameraRequest,
};
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::time::Instant;

use crate::cameras::{CameraHandle, CameraManager, ClockInfo};
use crate::error::{ApiError, ApiResult};
use crate::model::{StreamQuality, StreamRequest};
use crate::wire::{self, Status, StreamState};

/// Give up when the camera sends nothing for this long.
const STALL_TIMEOUT: Duration = Duration::from_secs(15);
/// In paced mode, send frames this far ahead of their display time.
const PACE_LEAD: Duration = Duration::from_millis(400);
/// How far a download request reaches; playback past it ends the stream.
const DOWNLOAD_SPAN_SECONDS: i64 = 6 * 3600;

fn now_us() -> i64 {
    jiff::Timestamp::now().as_microsecond()
}

fn ticks_to_us(ticks: i64) -> i64 {
    ticks * 1_000_000 / 90_000
}

/// Maps the demuxer's 90 kHz timestamps to wall-clock microseconds.
struct TimeMapper {
    /// (pts, wall µs) of the reference frame.
    anchor: Option<(i64, i64)>,
    /// Wall time the next anchor should use (from `X-Data-PTS` or the requested start).
    pending_wall_us: Option<i64>,
    live: bool,
    correction_s: i64,
    last_us: i64,
}

impl TimeMapper {
    fn new(live: bool, start_wall_us: Option<i64>, clock: ClockInfo) -> Self {
        Self {
            anchor: None,
            pending_wall_us: start_wall_us,
            live,
            correction_s: clock.correction,
            last_us: 0,
        }
    }

    /// `X-Data-PTS`: camera-clock milliseconds of the part's first frame.
    fn observe_part_pts(&mut self, camera_ms: i64) {
        if self.live {
            return;
        }
        let wall_us = (camera_ms + self.correction_s * 1000) * 1000;
        match self.anchor {
            None => self.pending_wall_us = Some(wall_us),
            // The camera skipped over a gap between recordings: re-anchor.
            Some(_) if (wall_us - self.last_us).abs() > 3_000_000 => {
                self.pending_wall_us = Some(wall_us);
                self.anchor = None;
            }
            Some(_) => {}
        }
    }

    fn map(&mut self, pts: i64) -> i64 {
        let (anchor_pts, anchor_us) = *self.anchor.get_or_insert_with(|| {
            let wall = self.pending_wall_us.take().unwrap_or_else(now_us);
            (pts, wall)
        });
        self.last_us = anchor_us + ticks_to_us(pts - anchor_pts);
        self.last_us
    }
}

/// Holds frames back so they leave at `speed`× their timeline.
struct Pacer {
    speed: f64,
    origin: Option<(i64, Instant)>,
}

impl Pacer {
    fn new(speed: f64) -> Self {
        Self {
            speed,
            origin: None,
        }
    }

    async fn wait_for(&mut self, timestamp_us: i64) {
        let (first_us, started) = *self.origin.get_or_insert((timestamp_us, Instant::now()));
        let offset =
            Duration::from_secs_f64(((timestamp_us - first_us).max(0) as f64 / 1e6) / self.speed);
        let due = started + offset;
        if let Some(wait) = due.checked_duration_since(Instant::now() + PACE_LEAD) {
            tokio::time::sleep(wait).await;
        }
    }
}

fn send(channel: &Channel, batch: Vec<u8>) -> ApiResult<()> {
    channel
        .send(InvokeResponseBody::Raw(batch))
        .map_err(|_| ApiError::new("internal", "the player went away"))
}

fn send_status(channel: &Channel, state: StreamState, error: Option<&ApiError>) {
    let mut batch = Vec::new();
    wire::push_status(
        &mut batch,
        now_us(),
        &Status {
            state,
            code: error.map(|e| e.code),
            message: error.map(|e| e.message.as_str()),
        },
    );
    let _ = channel.send(InvokeResponseBody::Raw(batch));
}

fn parse_start(start: &str) -> ApiResult<i64> {
    start
        .parse::<jiff::Timestamp>()
        .map(|t| t.as_second())
        .map_err(|_| ApiError::invalid(format!("invalid playback start {start:?}")))
}

pub struct Streams {
    cameras: Arc<CameraManager>,
    player_id: String,
    tasks: Mutex<HashMap<String, Task>>,
}

struct Task {
    /// Label of the webview whose channel receives the stream.
    webview: String,
    handle: tauri::async_runtime::JoinHandle<()>,
}

impl Streams {
    pub fn new(cameras: Arc<CameraManager>) -> Self {
        let mut id = [0u8; 8];
        getrandom::fill(&mut id).expect("OS random number generator");
        Self {
            cameras,
            player_id: format!("backsight-{}", hex::encode(id)),
            tasks: Mutex::new(HashMap::new()),
        }
    }

    pub fn open(
        self: &Arc<Self>,
        request: StreamRequest,
        channel: Channel,
        webview: &str,
    ) -> ApiResult<String> {
        let camera_id = match &request {
            StreamRequest::Live { camera_id, .. } | StreamRequest::Playback { camera_id, .. } => {
                camera_id.clone()
            }
        };
        let handle = self.cameras.get(&camera_id)?;
        if let StreamRequest::Playback { start, speed, .. } = &request {
            parse_start(start)?;
            if !(*speed > 0.0 && *speed <= 16.0) {
                return Err(ApiError::invalid(format!("unsupported speed {speed}")));
            }
        }

        let mut id = [0u8; 6];
        getrandom::fill(&mut id).expect("OS random number generator");
        let id = hex::encode(id);
        let streams = self.clone();
        let task_id = id.clone();
        let task = tauri::async_runtime::spawn(async move {
            send_status(&channel, StreamState::Buffering, None);
            match streams.run(&handle, &request, &channel).await {
                Ok(()) => {
                    let mut batch = Vec::new();
                    wire::push_end_of_stream(&mut batch, now_us());
                    let _ = channel.send(InvokeResponseBody::Raw(batch));
                }
                Err(err) => {
                    tracing::info!(camera = %handle.id, code = err.code, %err, "stream ended with an error");
                    send_status(&channel, StreamState::Error, Some(&err));
                }
            }
            streams.tasks.lock().expect("tasks lock").remove(&task_id);
        });
        self.tasks.lock().expect("tasks lock").insert(
            id.clone(),
            Task {
                webview: webview.to_owned(),
                handle: task,
            },
        );
        Ok(id)
    }

    pub fn close(&self, id: &str) {
        if let Some(task) = self.tasks.lock().expect("tasks lock").remove(id) {
            task.handle.abort();
        }
    }

    /// Stops every stream a webview opened. Called when it starts loading a document: a reload
    /// leaves the old page's streams with no receiver and nobody to close them, and they would
    /// otherwise hold the camera's media session until the app exits.
    pub fn close_webview(&self, webview: &str) {
        let mut tasks = self.tasks.lock().expect("tasks lock");
        let mut closed = 0;
        for (_, task) in tasks.extract_if(|_, task| task.webview == webview) {
            task.handle.abort();
            closed += 1;
        }
        if closed > 0 {
            tracing::debug!(
                webview,
                closed,
                "closed streams left open by the previous page"
            );
        }
    }

    async fn run(
        &self,
        handle: &CameraHandle,
        request: &StreamRequest,
        channel: &Channel,
    ) -> ApiResult<()> {
        let clock = handle.clock().unwrap_or(ClockInfo {
            correction: 0,
            utc_offset_minutes: 0,
        });
        let password = handle.cloud_password()?;
        let mut config = MediaConfig::new(handle.record().host, password);

        let (camera_request, live, speed, start_wall_us) = match request {
            StreamRequest::Live { quality, .. } => {
                let quality = match quality {
                    StreamQuality::Hd => Quality::High,
                    StreamQuality::Sd => Quality::Low,
                };
                (
                    CameraRequest::Live {
                        quality,
                        channel: 0,
                    },
                    true,
                    1.0,
                    None,
                )
            }
            StreamRequest::Playback { start, speed, .. } => {
                let start_utc = parse_start(start)?;
                let start = start_utc - clock.correction;
                let client_id = handle.user_id(false).await?;
                let request = if (*speed - 1.0).abs() < f64::EPSILON {
                    CameraRequest::Playback {
                        client_id,
                        start,
                        end: start + 86_400,
                        speed: 1,
                    }
                } else {
                    config.window_size = Some(50);
                    CameraRequest::Download {
                        client_id,
                        start,
                        end: start + DOWNLOAD_SPAN_SECONDS,
                        player_id: self.player_id.clone(),
                    }
                };
                (request, false, *speed, Some(start_utc * 1_000_000))
            }
        };
        let paced = !live && (speed - 1.0).abs() >= f64::EPSILON;
        let keyframes_only = speed > 4.0;
        let audio = speed == 1.0;

        let mut session = MediaSession::connect_and_start(&config, Some(&camera_request)).await?;
        let mut demuxer = TsDemuxer::new().with_audio_rate(handle.audio_rate().await);
        let mut mapper = TimeMapper::new(live, start_wall_us, clock);
        let mut pacer = Pacer::new(speed);
        let mut events = Vec::new();
        let mut pcm = Vec::new();
        let mut discontinuity = !live;
        let mut audio_codec = None;
        let mut playing = false;

        loop {
            let part = match tokio::time::timeout(STALL_TIMEOUT, session.next_part()).await {
                Ok(part) => part?,
                Err(_) => {
                    return Err(ApiError::new(
                        "offline",
                        "The camera stopped sending video.",
                    ));
                }
            };
            let Some(part) = part else {
                return Ok(());
            };
            let data = match part {
                StreamPart::Media { data, headers } => {
                    if let Some(ms) = headers.get("x-data-pts").and_then(|v| v.parse().ok()) {
                        mapper.observe_part_pts(ms);
                    }
                    data
                }
                part if part.is_finished() => return Ok(()),
                _ => continue,
            };

            if let Err(err) = demuxer.push(&data, &mut events) {
                // Corrupt or unexpected data: the demuxer resynchronises on its own.
                tracing::debug!(camera = %handle.id, %err, "demux error");
            }

            let mut batch = Vec::new();
            for event in events.drain(..) {
                match event {
                    MediaEvent::VideoConfig(config) => {
                        wire::push_video_config(
                            &mut batch,
                            now_us(),
                            std::mem::take(&mut discontinuity),
                            &config,
                        );
                    }
                    MediaEvent::Video(frame) => {
                        if keyframes_only && !frame.keyframe {
                            continue;
                        }
                        let timestamp = mapper.map(frame.pts);
                        if paced {
                            if !batch.is_empty() {
                                send(channel, std::mem::take(&mut batch))?;
                            }
                            pacer.wait_for(timestamp).await;
                        }
                        wire::push_video_frame(
                            &mut batch,
                            timestamp,
                            std::mem::take(&mut discontinuity),
                            &frame,
                        );
                        if !playing {
                            playing = true;
                            send_status(channel, StreamState::Playing, None);
                        }
                    }
                    MediaEvent::AudioConfig(config) if audio => {
                        audio_codec = Some(config.codec);
                        wire::push_audio_config(&mut batch, now_us(), false, &config);
                    }
                    MediaEvent::Audio(frame) if audio => {
                        let Some(codec) = audio_codec else { continue };
                        pcm.clear();
                        if g711::decode(codec, &frame.data, &mut pcm).is_ok() && !pcm.is_empty() {
                            let timestamp = mapper.map(frame.pts);
                            wire::push_audio_pcm(&mut batch, timestamp, false, &pcm);
                        }
                    }
                    _ => {}
                }
            }
            if !batch.is_empty() {
                send(channel, batch)?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLOCK: ClockInfo = ClockInfo {
        correction: 0,
        utc_offset_minutes: 330,
    };

    #[test]
    fn playback_times_follow_x_data_pts() {
        let mut mapper = TimeMapper::new(false, Some(1_000_000_000), CLOCK);
        mapper.observe_part_pts(1_790_534_057_000);
        assert_eq!(mapper.map(90_000), 1_790_534_057_000_000);
        // One second of media later.
        assert_eq!(mapper.map(180_000), 1_790_534_058_000_000);
        // The camera jumps over a gap: the next frame lands at the new time.
        mapper.observe_part_pts(1_790_534_100_000);
        assert_eq!(mapper.map(270_000), 1_790_534_100_000_000);
    }

    #[test]
    fn playback_without_headers_uses_the_requested_start() {
        let mut mapper = TimeMapper::new(false, Some(5_000_000), CLOCK);
        assert_eq!(mapper.map(900_000), 5_000_000);
        assert_eq!(mapper.map(945_000), 5_500_000);
    }

    #[test]
    fn live_ignores_part_times() {
        let mut mapper = TimeMapper::new(true, None, CLOCK);
        mapper.observe_part_pts(1_000);
        let first = mapper.map(0);
        assert!((first - now_us()).abs() < 5_000_000);
    }

    #[tokio::test]
    async fn pacer_spaces_frames_by_speed() {
        let mut pacer = Pacer::new(4.0);
        let started = Instant::now();
        pacer.wait_for(0).await;
        // 2.4 s of media at 4x is due 0.6 s in; with the 0.4 s lead we wait ~0.2 s.
        pacer.wait_for(2_400_000).await;
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(150) && waited < Duration::from_millis(600),
            "{waited:?}"
        );
    }
}
