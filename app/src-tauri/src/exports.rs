//! Clip export: download a time range from the camera's SD card and write an MP4.
//!
//! Jobs run one at a time per camera (the download itself is ~7× real time), report
//! progress through `export-progress` events, and can be cancelled. The file is written
//! as `<name>.mp4.part` and renamed when complete. Audio is re-encoded to AAC where the
//! OS offers an encoder (see `audio_aac.rs`); otherwise the clip has video only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tapo_camera::media::{AacTrackConfig, MediaEvent, Mp4Writer, TsDemuxer, g711};
use tapo_camera::stream::{MediaConfig, MediaSession, StreamPart, StreamRequest};

use crate::audio_aac::{AacEncoder, FRAME_SAMPLES, OUTPUT_RATE, Upsampler};
use crate::cameras::{CameraHandle, CameraManager, ClockInfo};
use crate::commands::sanitize_file_name;
use crate::db::Db;
use crate::error::{ApiError, ApiResult};
use crate::model::{AppEvent, ExportJob, ExportRequest, ExportState, Settings};

/// Longest clip we accept in one export.
const MAX_SECONDS: i64 = 6 * 3600;
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

pub struct Exports {
    db: Arc<Db>,
    cameras: Arc<CameraManager>,
    player_id: String,
    running: Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>,
    camera_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

fn format_local(utc_seconds: i64, clock: ClockInfo, pattern: &str) -> String {
    let local = utc_seconds + i64::from(clock.utc_offset_minutes) * 60;
    jiff::Timestamp::from_second(local)
        .map(|t| t.strftime(pattern).to_string())
        .unwrap_or_default()
}

/// Fills the name template (`{camera}`, `{date}`, `{start}`, `{end}`) in camera-local time.
pub fn file_name(template: &str, camera: &str, start: i64, end: i64, clock: ClockInfo) -> String {
    let name = template
        .replace("{camera}", camera)
        .replace("{date}", &format_local(start, clock, "%Y-%m-%d"))
        .replace("{start}", &format_local(start, clock, "%H-%M-%S"))
        .replace("{end}", &format_local(end, clock, "%H-%M-%S"));
    sanitize_file_name(&name)
}

/// `dir/name.mp4`, or `dir/name (2).mp4` … if that exists.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{name}.mp4"));
    let mut n = 2;
    while candidate.exists() || part_path(&candidate).exists() {
        candidate = dir.join(format!("{name} ({n}).mp4"));
        n += 1;
    }
    candidate
}

fn part_path(path: &Path) -> PathBuf {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    PathBuf::from(part)
}

impl Exports {
    pub fn new(db: Arc<Db>, cameras: Arc<CameraManager>) -> Self {
        let mut id = [0u8; 8];
        getrandom::fill(&mut id).expect("OS random number generator");
        // Jobs interrupted by a previous shutdown can't resume; mark them failed.
        if let Ok(jobs) = db.exports() {
            for mut job in jobs {
                if matches!(job.state, ExportState::Queued | ExportState::Running) {
                    job.state = ExportState::Failed;
                    job.error = Some("Backsight was closed before the export finished.".into());
                    let _ = db.save_export(&job);
                }
            }
        }
        Self {
            db,
            cameras,
            player_id: format!("backsight-export-{}", hex::encode(id)),
            running: Mutex::new(HashMap::new()),
            camera_locks: Mutex::new(HashMap::new()),
        }
    }

    fn publish(&self, job: &ExportJob) {
        if let Err(err) = self.db.save_export(job) {
            tracing::warn!(%err, "could not save export state");
        }
        self.cameras
            .emit(AppEvent::ExportProgress { job: job.clone() });
    }

    pub fn start(
        self: &Arc<Self>,
        request: ExportRequest,
        settings: &Settings,
    ) -> ApiResult<ExportJob> {
        let handle = self.cameras.get(&request.camera_id)?;
        handle.tapo_client()?;
        let parse = |s: &str| {
            s.parse::<jiff::Timestamp>()
                .map(|t| t.as_second())
                .map_err(|_| ApiError::invalid(format!("invalid time {s:?}")))
        };
        let (start, end) = (parse(&request.start)?, parse(&request.end)?);
        if end <= start {
            return Err(ApiError::invalid("The clip must end after it starts."));
        }
        if end - start > MAX_SECONDS {
            return Err(ApiError::invalid("Clips can be at most 6 hours long."));
        }

        let clock = handle.clock().unwrap_or(ClockInfo {
            correction: 0,
            utc_offset_minutes: 0,
        });
        let record = handle.record();
        let path = match &request.output_path {
            Some(path) => PathBuf::from(path),
            None => {
                let dir = PathBuf::from(&settings.export_dir);
                std::fs::create_dir_all(&dir).map_err(|e| {
                    ApiError::invalid(format!(
                        "Can't use the export folder {}: {e}",
                        dir.display()
                    ))
                })?;
                let name = file_name(
                    &settings.export_name_template,
                    &record.name,
                    start,
                    end,
                    clock,
                );
                unique_path(&dir, &name)
            }
        };

        let mut id = [0u8; 6];
        getrandom::fill(&mut id).expect("OS random number generator");
        let job = ExportJob {
            id: hex::encode(id),
            camera_id: record.id.clone(),
            camera_name: record.name.clone(),
            start: request.start.clone(),
            end: request.end.clone(),
            state: ExportState::Queued,
            progress: 0.0,
            bytes_written: 0,
            eta_seconds: None,
            output_path: Some(path.to_string_lossy().into_owned()),
            error: None,
            created_at: jiff::Timestamp::now().to_string(),
        };
        self.publish(&job);

        let lock = self
            .camera_locks
            .lock()
            .expect("locks")
            .entry(record.id.clone())
            .or_default()
            .clone();
        let exports = self.clone();
        let mut task_job = job.clone();
        let task = tauri::async_runtime::spawn(async move {
            let _turn = lock.lock().await;
            task_job.state = ExportState::Running;
            exports.publish(&task_job);
            let camera_start = start - clock.correction;
            let camera_end = end - clock.correction;
            match exports
                .run(&handle, &mut task_job, camera_start, camera_end, &path)
                .await
            {
                Ok(()) => {
                    task_job.state = ExportState::Done;
                    task_job.progress = 1.0;
                    task_job.eta_seconds = None;
                }
                Err(err) => {
                    let _ = std::fs::remove_file(part_path(&path));
                    task_job.state = ExportState::Failed;
                    task_job.error = Some(err.message);
                }
            }
            exports.publish(&task_job);
            exports
                .running
                .lock()
                .expect("running")
                .remove(&task_job.id);
        });
        self.running
            .lock()
            .expect("running")
            .insert(job.id.clone(), task);
        Ok(job)
    }

    pub fn cancel(&self, id: &str) -> ApiResult<()> {
        if let Some(task) = self.running.lock().expect("running").remove(id) {
            task.abort();
        }
        let mut job = self
            .db
            .exports()?
            .into_iter()
            .find(|job| job.id == id)
            .ok_or_else(|| ApiError::not_found("export"))?;
        if matches!(job.state, ExportState::Queued | ExportState::Running) {
            if let Some(path) = &job.output_path {
                let _ = std::fs::remove_file(part_path(Path::new(path)));
            }
            job.state = ExportState::Cancelled;
            job.eta_seconds = None;
            self.publish(&job);
        }
        Ok(())
    }

    async fn run(
        &self,
        handle: &CameraHandle,
        job: &mut ExportJob,
        start: i64,
        end: i64,
        path: &Path,
    ) -> ApiResult<()> {
        let mut config = MediaConfig::new(handle.record().host, handle.cloud_password()?);
        config.window_size = Some(200);
        let request = StreamRequest::Download {
            client_id: handle.user_id(false).await?,
            start,
            end,
            player_id: self.player_id.clone(),
        };
        let mut session = MediaSession::connect_and_start(&config, Some(&request)).await?;
        let mut demuxer = TsDemuxer::new().with_audio_rate(handle.audio_rate().await);
        let part = part_path(path);

        let mut writer: Option<Mp4Writer<std::io::BufWriter<std::fs::File>>> = None;
        let mut audio = AudioPipeline::default();
        let mut events = Vec::new();
        let mut first_pts = None;
        let mut last_pts = 0i64;
        let mut bytes = 0u64;
        let total_ticks = ((end - start) * 90_000) as f64;
        let started = Instant::now();
        let mut last_report = Instant::now();

        let internal = |e: tapo_camera::media::MediaError| {
            ApiError::internal(format!("writing the clip failed: {e}"))
        };
        loop {
            let next = tokio::time::timeout(STALL_TIMEOUT, session.next_part())
                .await
                .map_err(|_| {
                    ApiError::new("offline", "The camera stopped sending the recording.")
                })??;
            let data = match next {
                None => break,
                Some(part) if part.is_finished() => break,
                Some(StreamPart::Media { data, .. }) => data,
                Some(_) => continue,
            };
            bytes += data.len() as u64;
            if let Err(err) = demuxer.push(&data, &mut events) {
                tracing::debug!(%err, "export demux error");
            }
            for event in events.drain(..) {
                match event {
                    MediaEvent::VideoConfig(video) => match writer.as_mut() {
                        None => writer = Some(Mp4Writer::create(&part, video).map_err(internal)?),
                        Some(w) => {
                            if let Err(err) = w.set_video_config(&video) {
                                // The camera changed resolution mid-clip; keep what we have.
                                tracing::warn!(%err, "video config changed during export");
                            }
                        }
                    },
                    MediaEvent::Video(frame) => {
                        let Some(w) = writer.as_mut() else { continue };
                        first_pts.get_or_insert(frame.pts);
                        last_pts = last_pts.max(frame.pts);
                        w.write_video(&frame).map_err(internal)?;
                    }
                    MediaEvent::AudioConfig(config) => {
                        audio.configure(config.codec, config.sample_rate)
                    }
                    MediaEvent::Audio(frame) => {
                        if let Some(w) = writer.as_mut() {
                            audio.push(w, frame.pts, &frame.data).map_err(internal)?;
                        }
                    }
                }
            }

            if last_report.elapsed() >= PROGRESS_INTERVAL {
                last_report = Instant::now();
                let done = first_pts.map_or(0.0, |f| {
                    ((last_pts - f) as f64 / total_ticks).clamp(0.0, 1.0)
                });
                let elapsed = started.elapsed().as_secs_f64();
                job.progress = done;
                job.bytes_written = bytes;
                job.eta_seconds = (done > 0.02).then(|| elapsed / done * (1.0 - done));
                self.publish(job);
            }
        }

        let Some(mut writer) = writer else {
            return Err(ApiError::new(
                "not_found",
                "There is no video in that time range.",
            ));
        };
        audio.finish(&mut writer).map_err(internal)?;
        let summary = writer.finish().map_err(internal)?;
        std::fs::rename(&part, path)
            .map_err(|e| ApiError::internal(format!("could not save the clip: {e}")))?;
        job.bytes_written = summary.bytes_written;
        tracing::info!(
            path = %path.display(),
            frames = summary.video_frames,
            audio_frames = summary.audio_frames,
            seconds = summary.duration.as_secs_f64(),
            "export finished"
        );
        Ok(())
    }
}

/// G.711 → 48 kHz PCM → AAC, written to the MP4 as it goes.
#[derive(Default)]
struct AudioPipeline {
    state: Option<AudioState>,
    /// Set once encoding is known to be unavailable, so we stop trying.
    disabled: bool,
}

struct AudioState {
    codec: tapo_camera::media::AudioCodec,
    upsampler: Upsampler,
    encoder: AacEncoder,
    track_added: bool,
    first_pts: Option<i64>,
    frames_out: u64,
    pcm: Vec<i16>,
    upsampled: Vec<i16>,
    encoded: Vec<crate::audio_aac::AacFrame>,
}

impl AudioPipeline {
    fn configure(&mut self, codec: tapo_camera::media::AudioCodec, sample_rate: u32) {
        if self.disabled || self.state.is_some() {
            return;
        }
        if codec == tapo_camera::media::AudioCodec::Aac {
            // Re-encoding AAC isn't needed, but passthrough isn't wired up yet.
            self.disabled = true;
            return;
        }
        let (Some(upsampler), Ok(encoder)) = (Upsampler::new(sample_rate), AacEncoder::new())
        else {
            tracing::info!("no AAC encoder available; exporting video only");
            self.disabled = true;
            return;
        };
        self.state = Some(AudioState {
            codec,
            upsampler,
            encoder,
            track_added: false,
            first_pts: None,
            frames_out: 0,
            pcm: Vec::new(),
            upsampled: Vec::new(),
            encoded: Vec::new(),
        });
    }

    fn push<W: std::io::Write>(
        &mut self,
        writer: &mut Mp4Writer<W>,
        pts: i64,
        data: &[u8],
    ) -> Result<(), tapo_camera::media::MediaError> {
        let Some(state) = self.state.as_mut() else {
            return Ok(());
        };
        state.first_pts.get_or_insert(pts);
        state.pcm.clear();
        g711::decode(state.codec, data, &mut state.pcm)?;
        state.upsampled.clear();
        state.upsampler.process(&state.pcm, &mut state.upsampled);
        if let Err(err) = state.encoder.encode(&state.upsampled, &mut state.encoded) {
            tracing::warn!(%err, "AAC encoding failed; the rest of the clip has no audio");
            self.state = None;
            self.disabled = true;
            return Ok(());
        }
        state.flush(writer)
    }

    fn finish<W: std::io::Write>(
        &mut self,
        writer: &mut Mp4Writer<W>,
    ) -> Result<(), tapo_camera::media::MediaError> {
        let Some(state) = self.state.as_mut() else {
            return Ok(());
        };
        if state.encoder.finish(&mut state.encoded).is_err() {
            return Ok(());
        }
        state.flush(writer)
    }
}

impl AudioState {
    fn flush<W: std::io::Write>(
        &mut self,
        writer: &mut Mp4Writer<W>,
    ) -> Result<(), tapo_camera::media::MediaError> {
        if self.encoded.is_empty() {
            return Ok(());
        }
        if !self.track_added {
            writer.add_aac_track(AacTrackConfig::new(
                self.encoder.audio_specific_config().to_vec(),
            )?)?;
            self.track_added = true;
        }
        let first = self.first_pts.unwrap_or(0);
        for frame in self.encoded.drain(..) {
            let pts = first
                + (self.frames_out * u64::from(FRAME_SAMPLES) * 90_000 / u64::from(OUTPUT_RATE))
                    as i64;
            writer.write_aac(pts, &frame.data)?;
            self.frames_out += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UTC_PLUS_5_30: ClockInfo = ClockInfo {
        correction: 0,
        utc_offset_minutes: 330,
    };

    #[test]
    fn names_use_camera_local_time() {
        // 2026-09-28T04:30:00Z is 10:00 at UTC+5:30.
        let start = 1_790_569_800;
        let name = file_name(
            "{camera} {date} {start}-{end}",
            "Front Door",
            start,
            start + 90,
            UTC_PLUS_5_30,
        );
        assert_eq!(name, "Front Door 2026-09-28 10-00-00-10-01-30");
        assert_eq!(file_name("{camera}", "a/b:c", 0, 0, UTC_PLUS_5_30), "a_b_c");
    }

    #[test]
    fn unique_paths_avoid_existing_files() {
        let dir =
            std::env::temp_dir().join(format!("backsight-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = unique_path(&dir, "clip");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_path(&dir, "clip");
        assert_eq!(second.file_name().unwrap(), "clip (2).mp4");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
