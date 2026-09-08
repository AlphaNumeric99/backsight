//! Fast-start MP4 writer for exports.
//!
//! [`Mp4Writer`] turns a [`VideoConfig`] and [`VideoFrame`]s, plus optional raw AAC
//! frames, into a regular (non-fragmented) MP4 file with the `moov` box before `mdat`,
//! so players can start without reading the whole file.
//!
//! - H.264 uses an `avc1` sample entry and H.265 an `hvc1` one (all parameter sets in
//!   `hvcC`, none in the samples), the variant Apple players require.
//! - Sample data is spooled to an anonymous temporary file while writing, so memory use
//!   stays small for hour-long exports: only per-sample metadata (about 24 bytes a frame)
//!   and one chunk of media stay in memory. [`Mp4Writer::finish`] writes `ftyp`, `moov`
//!   and `mdat` to the output in one sequential pass.
//! - Timing comes from the frames' timestamps. Each sample lasts until the next one, so
//!   gaps show as a held frame (or silence) and audio stays in sync; the last video
//!   sample repeats the previous frame duration. Timestamps that go backwards are
//!   stitched: the sample gets the usual frame duration. With
//!   [`Mp4WriterOptions::max_gap`], long gaps are collapsed instead.
//! - The movie starts at the first video frame, which must be a keyframe (earlier
//!   frames are dropped). Audio before it is dropped; an audio track that starts later,
//!   or with an encoder delay, gets an edit list.
//! - A change of [`VideoConfig`] mid-file is an error
//!   ([`MediaError::VideoConfigChanged`]): finish the file and start another.
//!
//! Boxes are encoded with the [`shiguredo_mp4`] crate.

use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::num::{NonZeroU16, NonZeroU32};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use shiguredo_mp4::boxes::{
    AudioSampleEntryFields, Avc1Box, AvccBox, Brand, Co64Box, CttsBox, CttsEntry, DinfBox, EdtsBox,
    ElstBox, ElstEntry, EsdsBox, FtypBox, HdlrBox, Hvc1Box, HvccBox, MdhdBox, MdiaBox, MinfBox,
    MoovBox, Mp4aBox, MvhdBox, SampleEntry, SmhdBox, StblBox, StcoBox, StscBox, StscEntry, StsdBox,
    StssBox, StszBox, SttsBox, TkhdBox, TrakBox, VisualSampleEntryFields, VmhdBox,
};
use shiguredo_mp4::descriptors::{
    DecoderConfigDescriptor, DecoderSpecificInfo, EsDescriptor, SlConfigDescriptor,
};
use shiguredo_mp4::{Decode, Either, Encode, FixedPointNumber, Mp4FileTime, Uint};

use super::{MediaError, Ticks90k, VideoCodec, VideoConfig, VideoFrame, aac};

/// Timescale of the video track: the 90 kHz clock of the frame timestamps.
pub const VIDEO_TIMESCALE: u32 = 90_000;

/// Timescale of the movie header and edit lists (milliseconds).
const MOVIE_TIMESCALE: u32 = 1_000;
const TICKS_PER_SECOND: i64 = VIDEO_TIMESCALE as i64;
/// Target duration of an interleaving chunk.
const CHUNK_TICKS: i64 = TICKS_PER_SECOND;
/// A chunk is also written once it holds this much data.
const MAX_CHUNK_BYTES: usize = 2 << 20;
/// Frame duration assumed when the frames do not reveal one (30 fps).
const DEFAULT_FRAME_TICKS: i64 = 3_000;
/// Audio kept while waiting for the first video frame.
const MAX_AUDIO_LEAD_TICKS: i64 = 10 * TICKS_PER_SECOND;
const VIDEO_TRACK_ID: u32 = 1;
const AUDIO_TRACK_ID: u32 = 2;

/// Settings for an [`Mp4Writer`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Mp4WriterOptions {
    /// Directory for the temporary sample spool. Defaults to the system temporary
    /// directory, or the output file's directory for [`Mp4Writer::create`].
    pub temp_dir: Option<PathBuf>,
    /// Holes in the timeline longer than this (time not covered by any frame beyond the
    /// usual frame duration, for example between recordings) are removed in every
    /// track, so playback continues with the next frame. `None` keeps the real
    /// timeline: the frame before a hole stays on screen.
    pub max_gap: Option<Duration>,
    /// Creation time recorded in the movie, for example the wall-clock time of the first
    /// frame. Unset by default.
    pub creation_time: Option<SystemTime>,
}

impl Mp4WriterOptions {
    /// Default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets [`temp_dir`](Self::temp_dir).
    #[must_use]
    pub fn temp_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.temp_dir = Some(dir.into());
        self
    }

    /// Sets [`max_gap`](Self::max_gap).
    #[must_use]
    pub fn max_gap(mut self, gap: Duration) -> Self {
        self.max_gap = Some(gap);
        self
    }

    /// Sets [`creation_time`](Self::creation_time).
    #[must_use]
    pub fn creation_time(mut self, time: SystemTime) -> Self {
        self.creation_time = Some(time);
        self
    }
}

/// Describes the optional AAC track of an [`Mp4Writer`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AacTrackConfig {
    /// The MPEG-4 `AudioSpecificConfig`, as produced by the encoder.
    pub audio_specific_config: Bytes,
    /// Sample rate, which is also the track timescale.
    pub sample_rate: u32,
    /// Number of channels.
    pub channels: u8,
    /// Samples per channel in one frame: 1024 for AAC-LC.
    pub samples_per_frame: u32,
    /// Priming samples at the start of the stream that players should skip (the
    /// encoder delay, commonly 1024 or 2112). Frame timestamps include them.
    pub encoder_delay: u32,
}

impl AacTrackConfig {
    /// Builds a config from an `AudioSpecificConfig`, taking the sample rate and channel
    /// count from it.
    pub fn new(audio_specific_config: impl Into<Bytes>) -> Result<Self, MediaError> {
        let audio_specific_config = audio_specific_config.into();
        let parsed = aac::AudioSpecificConfig::parse(&audio_specific_config)?;
        let channels = aac::channel_count(parsed.channel_configuration);
        if channels == 0 {
            return Err(MediaError::Unsupported(
                "AAC channel layout defined by a program config element".to_owned(),
            ));
        }
        Ok(Self {
            audio_specific_config,
            sample_rate: parsed.sample_rate,
            channels,
            samples_per_frame: aac::SAMPLES_PER_FRAME,
            encoder_delay: 0,
        })
    }

    /// Builds a config for the stream an ADTS header describes (to remux ADTS audio,
    /// write each frame's [`payload`](aac::AdtsFrame::payload)).
    pub fn from_adts(header: &aac::AdtsHeader) -> Result<Self, MediaError> {
        Self::new(header.audio_specific_config().to_bytes())
    }

    /// Sets [`encoder_delay`](Self::encoder_delay).
    #[must_use]
    pub fn with_encoder_delay(mut self, samples: u32) -> Self {
        self.encoder_delay = samples;
        self
    }

    /// Sets [`samples_per_frame`](Self::samples_per_frame).
    #[must_use]
    pub fn with_samples_per_frame(mut self, samples: u32) -> Self {
        self.samples_per_frame = samples.max(1);
        self
    }

    fn frame_ticks(&self) -> i64 {
        i64::from(self.samples_per_frame) * TICKS_PER_SECOND / i64::from(self.sample_rate)
    }
}

/// What [`Mp4Writer::finish`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Mp4Summary {
    /// Video codec.
    pub codec: VideoCodec,
    /// Codec string of the video configuration.
    pub codec_string: String,
    /// Video width in pixels.
    pub width: u32,
    /// Video height in pixels.
    pub height: u32,
    /// Video samples written.
    pub video_frames: u64,
    /// Video samples that are keyframes.
    pub keyframes: u64,
    /// AAC frames written.
    pub audio_frames: u64,
    /// Presentation duration of the video track.
    pub duration: Duration,
    /// Size of the MP4 file.
    pub bytes_written: u64,
    /// Video frames dropped before the first keyframe.
    pub dropped_video_frames: u64,
    /// Audio frames dropped because they end before the first video frame.
    pub dropped_audio_frames: u64,
}

#[derive(Debug, Clone, Copy)]
struct SampleInfo {
    size: u32,
    dts: Ticks90k,
    pts: Ticks90k,
    keyframe: bool,
}

#[derive(Debug, Clone, Copy)]
struct ChunkInfo {
    /// Offset in the spool.
    offset: u64,
    samples: u32,
}

/// Samples of one track; the last `pending_samples` are still in `pending`.
#[derive(Debug, Default)]
struct Track {
    samples: Vec<SampleInfo>,
    chunks: Vec<ChunkInfo>,
    pending: Vec<u8>,
    pending_samples: usize,
}

impl Track {
    fn push(&mut self, sample: SampleInfo, data: &[u8]) {
        self.samples.push(sample);
        self.pending.extend_from_slice(data);
        self.pending_samples += 1;
    }

    /// Duration covered by the pending chunk, by decode time.
    fn pending_ticks(&self) -> i64 {
        if self.pending_samples == 0 {
            return 0;
        }
        let first = self.samples[self.samples.len() - self.pending_samples].dts;
        let last = self.samples[self.samples.len() - 1].dts;
        last - first
    }

    fn flush(&mut self, spool: &mut File, spool_len: &mut u64) -> io::Result<()> {
        if self.pending_samples == 0 {
            return Ok(());
        }
        spool.write_all(&self.pending)?;
        self.chunks.push(ChunkInfo {
            offset: *spool_len,
            // At most a few thousand samples per chunk.
            samples: self.pending_samples as u32,
        });
        *spool_len += self.pending.len() as u64;
        self.pending.clear();
        self.pending_samples = 0;
        Ok(())
    }

    /// Drops the first `count` samples, which must all be pending.
    fn drop_front(&mut self, count: usize) {
        debug_assert!(count <= self.pending_samples && self.chunks.is_empty());
        let bytes: usize = self.samples[..count].iter().map(|s| s.size as usize).sum();
        self.samples.drain(..count);
        self.pending.drain(..bytes);
        self.pending_samples -= count;
    }
}

struct AudioTrack {
    config: AacTrackConfig,
    track: Track,
}

/// Writes a fast-start MP4 file. See the [module documentation](self).
///
/// ```no_run
/// use tapo_camera::media::{MediaEvent, Mp4Writer};
/// # fn events() -> Vec<MediaEvent> { Vec::new() }
///
/// let mut writer = None;
/// for event in events() {
///     match event {
///         MediaEvent::VideoConfig(config) => match writer.as_mut() {
///             None => writer = Some(Mp4Writer::create("clip.mp4", config)?),
///             Some(writer) => writer.set_video_config(&config)?,
///         },
///         MediaEvent::Video(frame) => {
///             if let Some(writer) = writer.as_mut() {
///                 writer.write_video(&frame)?;
///             }
///         }
///         _ => {}
///     }
/// }
/// if let Some(writer) = writer {
///     let summary = writer.finish()?;
///     println!("{} frames, {:?}", summary.video_frames, summary.duration);
/// }
/// # Ok::<(), tapo_camera::media::MediaError>(())
/// ```
pub struct Mp4Writer<W: Write> {
    output: W,
    spool: File,
    spool_len: u64,
    options: Mp4WriterOptions,
    video_config: VideoConfig,
    video: Track,
    audio: Option<AudioTrack>,
    /// PTS of the first video frame: time zero of the movie.
    start_pts: Option<Ticks90k>,
    dropped_video_frames: u64,
    dropped_audio_frames: u64,
}

impl Mp4Writer<BufWriter<File>> {
    /// Creates the MP4 file at `path`, spooling sample data in the same directory.
    pub fn create(path: impl AsRef<Path>, video: VideoConfig) -> Result<Self, MediaError> {
        Self::create_with_options(path, video, Mp4WriterOptions::default())
    }

    /// Like [`create`](Self::create), with options. Unless `options.temp_dir` is set,
    /// the spool goes next to the output file.
    pub fn create_with_options(
        path: impl AsRef<Path>,
        video: VideoConfig,
        mut options: Mp4WriterOptions,
    ) -> Result<Self, MediaError> {
        let path = path.as_ref();
        if options.temp_dir.is_none() {
            options.temp_dir = path
                .parent()
                .filter(|dir| !dir.as_os_str().is_empty())
                .map(Path::to_path_buf)
                .or_else(|| Some(PathBuf::from(".")));
        }
        let file = File::create(path)?;
        Self::with_options(BufWriter::with_capacity(1 << 20, file), video, options)
    }
}

impl<W: Write> Mp4Writer<W> {
    /// Creates a writer that writes the finished file to `output`, spooling sample data
    /// in the system temporary directory.
    pub fn new(output: W, video: VideoConfig) -> Result<Self, MediaError> {
        Self::with_options(output, video, Mp4WriterOptions::default())
    }

    /// Creates a writer with options.
    pub fn with_options(
        output: W,
        video: VideoConfig,
        options: Mp4WriterOptions,
    ) -> Result<Self, MediaError> {
        // Fail early on configs the sample entry cannot represent.
        video_sample_entry(&video)?;
        let spool = match &options.temp_dir {
            Some(dir) => tempfile::tempfile_in(dir)?,
            None => tempfile::tempfile()?,
        };
        Ok(Self {
            output,
            spool,
            spool_len: 0,
            options,
            video_config: video,
            video: Track::default(),
            audio: None,
            start_pts: None,
            dropped_video_frames: 0,
            dropped_audio_frames: 0,
        })
    }

    /// The video configuration the file is written with.
    pub fn video_config(&self) -> &VideoConfig {
        &self.video_config
    }

    /// Accepts a repeated [`VideoConfig`]. A config that differs from the one the file
    /// was started with is rejected with [`MediaError::VideoConfigChanged`].
    pub fn set_video_config(&mut self, config: &VideoConfig) -> Result<(), MediaError> {
        if *config == self.video_config {
            return Ok(());
        }
        let describe = |c: &VideoConfig| format!("{} {}x{}", c.codec_string, c.width, c.height);
        Err(MediaError::VideoConfigChanged {
            from: describe(&self.video_config),
            to: describe(config),
        })
    }

    /// Adds an AAC audio track. Call it once, before the first [`write_aac`](Self::write_aac).
    pub fn add_aac_track(&mut self, config: AacTrackConfig) -> Result<(), MediaError> {
        if self.audio.is_some() {
            return Err(MediaError::InvalidInput(
                "the file already has an audio track".to_owned(),
            ));
        }
        if config.sample_rate == 0 || config.channels == 0 {
            return Err(MediaError::InvalidInput(
                "AAC track needs a sample rate and channels".to_owned(),
            ));
        }
        self.audio = Some(AudioTrack {
            config,
            track: Track::default(),
        });
        Ok(())
    }

    /// Whether an audio track has been added.
    pub fn has_audio_track(&self) -> bool {
        self.audio.is_some()
    }

    /// Appends a video frame (length-prefixed NAL units matching the config).
    ///
    /// Frames before the first keyframe are dropped.
    pub fn write_video(&mut self, frame: &VideoFrame) -> Result<(), MediaError> {
        let size = u32::try_from(frame.data.len())
            .map_err(|_| MediaError::InvalidInput("video frame larger than 4 GiB".to_owned()))?;
        if self.start_pts.is_none() {
            if !frame.keyframe {
                self.dropped_video_frames += 1;
                return Ok(());
            }
            self.start_pts = Some(frame.pts);
            self.trim_early_audio(frame.pts);
        }
        self.video.push(
            SampleInfo {
                size,
                dts: frame.dts,
                pts: frame.pts,
                keyframe: frame.keyframe,
            },
            &frame.data,
        );
        let ticks = self.video.pending_ticks();
        if !(0..CHUNK_TICKS).contains(&ticks) || self.video.pending.len() >= MAX_CHUNK_BYTES {
            self.video.flush(&mut self.spool, &mut self.spool_len)?;
            if let Some(audio) = self.audio.as_mut() {
                audio.track.flush(&mut self.spool, &mut self.spool_len)?;
            }
        }
        Ok(())
    }

    /// Appends one raw AAC frame (no ADTS header) whose first sample is presented at
    /// `pts`. Requires [`add_aac_track`](Self::add_aac_track).
    pub fn write_aac(&mut self, pts: Ticks90k, frame: &[u8]) -> Result<(), MediaError> {
        let size = u32::try_from(frame.len())
            .map_err(|_| MediaError::InvalidInput("audio frame larger than 4 GiB".to_owned()))?;
        let Some(audio) = self.audio.as_mut() else {
            return Err(MediaError::InvalidInput(
                "write_aac needs an AAC track (add_aac_track)".to_owned(),
            ));
        };
        let frame_ticks = audio.config.frame_ticks();
        match self.start_pts {
            Some(start) if pts + frame_ticks <= start => {
                self.dropped_audio_frames += 1;
                return Ok(());
            }
            Some(_) => {}
            None => {
                // Keep a bounded lead of audio until the first video frame arrives.
                let track = &mut audio.track;
                let excess = track
                    .samples
                    .iter()
                    .take_while(|s| pts - s.pts > MAX_AUDIO_LEAD_TICKS)
                    .count();
                if excess > 0 {
                    track.drop_front(excess);
                    self.dropped_audio_frames += excess as u64;
                }
            }
        }
        let sample = SampleInfo {
            size,
            dts: pts,
            pts,
            keyframe: true,
        };
        audio.track.push(sample, frame);
        let flush_audio = self.start_pts.is_some()
            && (audio.track.pending_ticks() >= 2 * CHUNK_TICKS
                || audio.track.pending_ticks() < 0
                || audio.track.pending.len() >= MAX_CHUNK_BYTES);
        if flush_audio {
            audio.track.flush(&mut self.spool, &mut self.spool_len)?;
        }
        Ok(())
    }

    /// Drops buffered audio that ends before the first video frame.
    fn trim_early_audio(&mut self, start: Ticks90k) {
        if let Some(audio) = self.audio.as_mut() {
            let frame_ticks = audio.config.frame_ticks();
            let early = audio
                .track
                .samples
                .iter()
                .take_while(|s| s.pts + frame_ticks <= start)
                .count();
            if early > 0 {
                audio.track.drop_front(early);
                self.dropped_audio_frames += early as u64;
            }
        }
    }

    /// Writes the file: `ftyp`, `moov`, then `mdat` with the spooled samples. Fails if no
    /// video frame was written.
    pub fn finish(mut self) -> Result<Mp4Summary, MediaError> {
        if self.video.samples.is_empty() {
            return Err(MediaError::InvalidInput(
                "no video frames were written (the first must be a keyframe)".to_owned(),
            ));
        }
        self.video.flush(&mut self.spool, &mut self.spool_len)?;
        if let Some(audio) = self.audio.as_mut() {
            audio.track.flush(&mut self.spool, &mut self.spool_len)?;
        }
        let audio_frames = self
            .audio
            .as_ref()
            .map_or(0, |a| a.track.samples.len() as u64);

        let max_gap = self
            .options
            .max_gap
            .map(|gap| i64::try_from(gap.as_micros() * 9 / 100).unwrap_or(i64::MAX));
        let video_timing = video_timing(&self.video.samples, max_gap);
        // Time zero of the movie: the first presented video frame.
        let start_pts = self.video.samples[0].dts + video_timing.media_start;
        let audio_timing = self
            .audio
            .as_ref()
            .filter(|a| !a.track.samples.is_empty())
            .map(|a| audio_timing(&a.track.samples, &a.config, start_pts, max_gap));
        let creation_time = self
            .options
            .creation_time
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(Mp4FileTime::from_secs(0), Mp4FileTime::from_unix_time);

        let tracks = MovieTracks {
            video_config: &self.video_config,
            video: &self.video,
            video_timing: &video_timing,
            audio: self.audio.as_ref().zip(audio_timing.as_ref()),
            creation_time,
        };
        let ftyp = encode(&ftyp_box(self.video_config.codec))?;
        let mdat_header_len: u64 = if self.spool_len + 8 > u64::from(u32::MAX) {
            16
        } else {
            8
        };

        // The moov size does not depend on the offset values, only on stco vs co64.
        let mut use_co64 = false;
        let mut moov = tracks.moov(0, use_co64)?;
        let mut moov_len = encoded_len(&moov)?;
        let mut data_start = ftyp.len() as u64 + moov_len + mdat_header_len;
        if data_start + self.spool_len > u64::from(u32::MAX) {
            use_co64 = true;
            moov = tracks.moov(0, use_co64)?;
            moov_len = encoded_len(&moov)?;
            data_start = ftyp.len() as u64 + moov_len + mdat_header_len;
        }
        moov = tracks.moov(data_start, use_co64)?;
        let moov_bytes = encode(&moov)?;
        debug_assert_eq!(moov_bytes.len() as u64, moov_len);

        let mut output = self.output;
        output.write_all(&ftyp)?;
        output.write_all(&moov_bytes)?;
        let mdat_size = mdat_header_len + self.spool_len;
        if mdat_header_len == 16 {
            output.write_all(&1u32.to_be_bytes())?;
            output.write_all(b"mdat")?;
            output.write_all(&mdat_size.to_be_bytes())?;
        } else {
            // Checked above: fits in 32 bits.
            output.write_all(&(mdat_size as u32).to_be_bytes())?;
            output.write_all(b"mdat")?;
        }
        self.spool.seek(SeekFrom::Start(0))?;
        let copied = io::copy(&mut (&mut self.spool).take(self.spool_len), &mut output)?;
        if copied != self.spool_len {
            return Err(MediaError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "sample spool is shorter than expected",
            )));
        }
        output.flush()?;

        Ok(Mp4Summary {
            codec: self.video_config.codec,
            codec_string: self.video_config.codec_string.clone(),
            width: self.video_config.width,
            height: self.video_config.height,
            video_frames: self.video.samples.len() as u64,
            keyframes: self.video.samples.iter().filter(|s| s.keyframe).count() as u64,
            audio_frames,
            duration: ticks_to_duration(video_timing.presentation_ticks()),
            bytes_written: data_start + self.spool_len,
            dropped_video_frames: self.dropped_video_frames,
            dropped_audio_frames: self.dropped_audio_frames,
        })
    }
}

impl<W: Write> std::fmt::Debug for Mp4Writer<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mp4Writer")
            .field("codec", &self.video_config.codec_string)
            .field("video_frames", &self.video.samples.len())
            .field(
                "audio_frames",
                &self.audio.as_ref().map(|a| a.track.samples.len()),
            )
            .field("spooled_bytes", &self.spool_len)
            .finish_non_exhaustive()
    }
}

fn ticks_to_duration(ticks: i64) -> Duration {
    let ticks = u128::try_from(ticks).unwrap_or(0);
    // 1 tick = 100_000 / 9 ns.
    Duration::from_nanos(u64::try_from(ticks * 100_000 / 9).unwrap_or(u64::MAX))
}

// ---------------------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------------------

/// Sample durations and composition offsets of the video track, in 90 kHz ticks.
struct VideoTiming {
    durations: Vec<u32>,
    composition_offsets: Option<Vec<i64>>,
    /// Composition time of the first presented frame (the edit list's media time).
    media_start: i64,
    /// From the first presented frame to the end of the last one.
    presentation: i64,
}

impl VideoTiming {
    fn media_duration(&self) -> i64 {
        self.durations.iter().map(|&d| i64::from(d)).sum()
    }

    fn presentation_ticks(&self) -> i64 {
        self.presentation
    }
}

fn video_timing(samples: &[SampleInfo], max_gap: Option<i64>) -> VideoTiming {
    let is_normal = |delta: i64| delta > 0 && delta <= TICKS_PER_SECOND;
    let mut nominal = samples
        .windows(2)
        .map(|w| w[1].dts - w[0].dts)
        .find(|&delta| is_normal(delta))
        .unwrap_or(DEFAULT_FRAME_TICKS);
    let mut durations = Vec::with_capacity(samples.len());
    for w in samples.windows(2) {
        let delta = w[1].dts - w[0].dts;
        let collapse = delta <= 0 || max_gap.is_some_and(|gap| delta > gap + nominal);
        let duration = if collapse {
            nominal
        } else {
            if is_normal(delta) {
                nominal = delta;
            }
            delta
        };
        durations.push(u32::try_from(duration).unwrap_or(u32::MAX));
    }
    durations.push(u32::try_from(nominal).unwrap_or(u32::MAX));

    let offsets: Vec<i64> = samples
        .iter()
        .map(|s| (s.pts - s.dts).clamp(i64::from(i32::MIN), i64::from(i32::MAX)))
        .collect();
    // Composition time = decode time + offset. The presentation runs from the earliest
    // to one frame past the latest; with dropped frames (e.g. RASL pictures) or B-frames
    // this differs from the sum of decode durations.
    let mut decode_time = 0i64;
    let (mut earliest, mut latest) = (i64::MAX, i64::MIN);
    for (&duration, &offset) in durations.iter().zip(&offsets) {
        earliest = earliest.min(decode_time + offset);
        latest = latest.max(decode_time + offset);
        decode_time += i64::from(duration);
    }
    let media_start = earliest.max(0);
    let presentation = (latest + nominal - media_start).max(0);
    let composition_offsets = offsets.iter().any(|&o| o != 0).then_some(offsets);
    VideoTiming {
        durations,
        composition_offsets,
        media_start,
        presentation,
    }
}

/// Sample durations and edit of the audio track, in samples.
struct AudioTiming {
    durations: Vec<u32>,
    /// Silence before the track starts, relative to the first video frame.
    empty: u64,
    /// Samples of media to skip: encoder delay and audio before the first video frame.
    media_start: u64,
}

impl AudioTiming {
    fn media_duration(&self) -> u64 {
        self.durations.iter().map(|&d| u64::from(d)).sum()
    }
}

fn audio_timing(
    samples: &[SampleInfo],
    config: &AacTrackConfig,
    start_pts: Ticks90k,
    max_gap: Option<i64>,
) -> AudioTiming {
    let rate = i64::from(config.sample_rate);
    let frame = i64::from(config.samples_per_frame);
    let to_samples = |ticks: i64| -> i64 {
        // Rounded to the nearest sample; i128 avoids overflow for extreme timestamps.
        let scaled = i128::from(ticks) * i128::from(rate);
        let half = i128::from(TICKS_PER_SECOND / 2) * scaled.signum();
        ((scaled + half) / i128::from(TICKS_PER_SECOND)) as i64
    };
    let max_gap_samples = max_gap.map(to_samples);

    // Positions snap to back-to-back frames unless the timestamps show a real gap.
    let mut positions: Vec<i64> = Vec::with_capacity(samples.len());
    for sample in samples {
        let position = to_samples(sample.pts - start_pts);
        let snapped = match positions.last() {
            None => position,
            Some(&previous) => {
                let expected = previous + frame;
                let jitter = (position - expected).abs() <= frame / 2;
                let backwards = position < expected;
                let collapsed = max_gap_samples.is_some_and(|gap| position - expected > gap);
                if jitter || backwards || collapsed {
                    expected
                } else {
                    position
                }
            }
        };
        positions.push(snapped);
    }
    let mut durations: Vec<u32> = positions
        .windows(2)
        .map(|w| u32::try_from(w[1] - w[0]).unwrap_or(u32::MAX))
        .collect();
    durations.push(config.samples_per_frame);

    // The first real (non-priming) sample should play at its timestamp.
    let first = positions[0] + i64::from(config.encoder_delay);
    let empty = u64::try_from(first).unwrap_or(0);
    let media_start = u64::from(config.encoder_delay) + u64::try_from(-first).unwrap_or(0);
    AudioTiming {
        durations,
        empty,
        media_start,
    }
}

// ---------------------------------------------------------------------------------------
// Boxes
// ---------------------------------------------------------------------------------------

fn mp4_error(err: shiguredo_mp4::Error) -> MediaError {
    MediaError::Mp4(err.to_string())
}

fn encode<T: Encode>(value: &T) -> Result<Vec<u8>, MediaError> {
    value.encode_to_vec().map_err(mp4_error)
}

fn encoded_len<T: Encode>(value: &T) -> Result<u64, MediaError> {
    Ok(encode(value)?.len() as u64)
}

/// Converts `ticks` in `timescale` units to the movie timescale, rounding up.
fn to_movie_time(ticks: u64, timescale: u32) -> u64 {
    let scaled = u128::from(ticks) * u128::from(MOVIE_TIMESCALE);
    scaled.div_ceil(u128::from(timescale)) as u64
}

fn ftyp_box(codec: VideoCodec) -> FtypBox {
    let codec_brand = match codec {
        VideoCodec::H264 => Brand::AVC1,
        VideoCodec::H265 => Brand::HVC1,
    };
    FtypBox {
        major_brand: Brand::ISOM,
        minor_version: 0x200,
        compatible_brands: vec![Brand::ISOM, Brand::ISO2, codec_brand, Brand::MP41],
    }
}

/// Wraps a decoder configuration record in a box header so the box parser can read it.
fn config_box_bytes(fourcc: &[u8; 4], record: &[u8]) -> Result<Vec<u8>, MediaError> {
    let size = u32::try_from(record.len() + 8)
        .map_err(|_| MediaError::InvalidInput("decoder configuration too large".to_owned()))?;
    let mut bytes = Vec::with_capacity(record.len() + 8);
    bytes.extend_from_slice(&size.to_be_bytes());
    bytes.extend_from_slice(fourcc);
    bytes.extend_from_slice(record);
    Ok(bytes)
}

fn video_sample_entry(config: &VideoConfig) -> Result<SampleEntry, MediaError> {
    let dimension = |value: u32| {
        u16::try_from(value)
            .ok()
            .filter(|&v| v > 0 && v <= i16::MAX as u16)
            .ok_or_else(|| MediaError::InvalidInput(format!("unsupported video size {value}")))
    };
    let visual = VisualSampleEntryFields {
        data_reference_index: VisualSampleEntryFields::DEFAULT_DATA_REFERENCE_INDEX,
        width: dimension(config.width)?,
        height: dimension(config.height)?,
        horizresolution: VisualSampleEntryFields::DEFAULT_HORIZRESOLUTION,
        vertresolution: VisualSampleEntryFields::DEFAULT_VERTRESOLUTION,
        frame_count: VisualSampleEntryFields::DEFAULT_FRAME_COUNT,
        compressorname: VisualSampleEntryFields::NULL_COMPRESSORNAME,
        depth: VisualSampleEntryFields::DEFAULT_DEPTH,
    };
    let invalid = |_| MediaError::invalid("decoder configuration record", "cannot be parsed");
    Ok(match config.codec {
        VideoCodec::H264 => {
            let bytes = config_box_bytes(b"avcC", &config.description)?;
            let (mut avcc_box, _) = AvccBox::decode(&bytes).map_err(invalid)?;
            if avcc_box.length_size_minus_one.get() != 3 {
                return Err(MediaError::Unsupported(
                    "avcC with NAL length fields other than 4 bytes".to_owned(),
                ));
            }
            if !matches!(avcc_box.avc_profile_indication, 66 | 77 | 88) {
                // Records without the high-profile fields are common; assume 4:2:0 8-bit.
                avcc_box.chroma_format.get_or_insert(Uint::new(1));
                avcc_box.bit_depth_luma_minus8.get_or_insert(Uint::new(0));
                avcc_box.bit_depth_chroma_minus8.get_or_insert(Uint::new(0));
            }
            SampleEntry::Avc1(Avc1Box {
                visual,
                avcc_box,
                unknown_boxes: Vec::new(),
            })
        }
        VideoCodec::H265 => {
            let bytes = config_box_bytes(b"hvcC", &config.description)?;
            let (hvcc_box, _) = HvccBox::decode(&bytes).map_err(invalid)?;
            if hvcc_box.length_size_minus_one.get() != 3 {
                return Err(MediaError::Unsupported(
                    "hvcC with NAL length fields other than 4 bytes".to_owned(),
                ));
            }
            SampleEntry::Hvc1(Hvc1Box {
                visual,
                hvcc_box,
                unknown_boxes: Vec::new(),
            })
        }
    })
}

fn audio_sample_entry(config: &AacTrackConfig, track: &Track, duration: u64) -> SampleEntry {
    let total_bytes: u64 = track.samples.iter().map(|s| u64::from(s.size)).sum();
    let max_sample = track.samples.iter().map(|s| s.size).max().unwrap_or(0);
    let seconds = duration as f64 / f64::from(config.sample_rate);
    let avg_bitrate = if seconds > 0.0 {
        (total_bytes as f64 * 8.0 / seconds) as u32
    } else {
        0
    };
    let max_bitrate = peak_bitrate(track, config);
    SampleEntry::Mp4a(Mp4aBox {
        audio: AudioSampleEntryFields {
            data_reference_index: NonZeroU16::MIN,
            channelcount: u16::from(config.channels),
            samplesize: AudioSampleEntryFields::DEFAULT_SAMPLESIZE,
            // 16.16 fixed point; rates above 65535 Hz do not fit and are written as 0.
            samplerate: FixedPointNumber::new(u16::try_from(config.sample_rate).unwrap_or(0), 0),
        },
        esds_box: EsdsBox {
            es: EsDescriptor {
                es_id: EsDescriptor::MIN_ES_ID,
                stream_priority: EsDescriptor::LOWEST_STREAM_PRIORITY,
                depends_on_es_id: None,
                url_string: None,
                ocr_es_id: None,
                dec_config_descr: DecoderConfigDescriptor {
                    object_type_indication:
                        DecoderConfigDescriptor::OBJECT_TYPE_INDICATION_AUDIO_ISO_IEC_14496_3,
                    stream_type: DecoderConfigDescriptor::STREAM_TYPE_AUDIO,
                    up_stream: DecoderConfigDescriptor::UP_STREAM_FALSE,
                    buffer_size_db: Uint::new(max_sample.min(0xFF_FFFF)),
                    max_bitrate: max_bitrate.max(avg_bitrate),
                    avg_bitrate,
                    dec_specific_info: Some(DecoderSpecificInfo {
                        payload: config.audio_specific_config.to_vec(),
                    }),
                },
                sl_config_descr: SlConfigDescriptor,
            },
        },
        unknown_boxes: Vec::new(),
    })
}

/// Highest bitrate over any one-second window of frames.
fn peak_bitrate(track: &Track, config: &AacTrackConfig) -> u32 {
    let frames_per_second = (config.sample_rate / config.samples_per_frame).max(1) as usize;
    let sizes: Vec<u64> = track.samples.iter().map(|s| u64::from(s.size)).collect();
    let peak_bytes = sizes
        .windows(frames_per_second.min(sizes.len()).max(1))
        .map(|w| w.iter().sum::<u64>())
        .max()
        .unwrap_or(0);
    let window_seconds = frames_per_second as f64 * f64::from(config.samples_per_frame)
        / f64::from(config.sample_rate);
    (peak_bytes as f64 * 8.0 / window_seconds.max(1e-3)) as u32
}

/// Everything needed to build the `moov` box.
struct MovieTracks<'a> {
    video_config: &'a VideoConfig,
    video: &'a Track,
    video_timing: &'a VideoTiming,
    audio: Option<(&'a AudioTrack, &'a AudioTiming)>,
    creation_time: Mp4FileTime,
}

impl MovieTracks<'_> {
    fn moov(&self, data_start: u64, use_co64: bool) -> Result<MoovBox, MediaError> {
        let mut traks = vec![self.video_trak(data_start, use_co64)?];
        if let Some((audio, timing)) = self.audio {
            traks.push(self.audio_trak(audio, timing, data_start, use_co64));
        }
        let duration = traks
            .iter()
            .map(|trak| trak.tkhd_box.duration)
            .max()
            .unwrap_or(0);
        Ok(MoovBox {
            mvhd_box: MvhdBox {
                creation_time: self.creation_time,
                modification_time: self.creation_time,
                timescale: NonZeroU32::new(MOVIE_TIMESCALE).expect("non-zero"),
                duration,
                rate: MvhdBox::DEFAULT_RATE,
                volume: MvhdBox::DEFAULT_VOLUME,
                matrix: MvhdBox::DEFAULT_MATRIX,
                next_track_id: traks.len() as u32 + 1,
            },
            trak_boxes: traks,
            mvex_box: None,
            unknown_boxes: Vec::new(),
        })
    }

    fn video_trak(&self, data_start: u64, use_co64: bool) -> Result<TrakBox, MediaError> {
        let timing = self.video_timing;
        let media_duration = u64::try_from(timing.media_duration()).unwrap_or(0);
        let media_start = u64::try_from(timing.media_start).unwrap_or(0);
        let presentation_ticks = u64::try_from(timing.presentation_ticks()).unwrap_or(0);
        let presentation = to_movie_time(presentation_ticks, VIDEO_TIMESCALE);
        // Without an edit list the whole media is presented from its start.
        let needs_edit = media_start > 0 || presentation_ticks != media_duration;
        let edts_box = needs_edit.then(|| EdtsBox {
            elst_box: Some(ElstBox {
                entries: vec![ElstEntry {
                    edit_duration: presentation,
                    media_time: media_start as i64,
                    media_rate: FixedPointNumber::new(1, 0),
                }],
            }),
            unknown_boxes: Vec::new(),
        });

        let samples = &self.video.samples;
        let stss_box = (!samples.iter().all(|s| s.keyframe)).then(|| StssBox {
            sample_numbers: samples
                .iter()
                .enumerate()
                .filter(|(_, s)| s.keyframe)
                .filter_map(|(i, _)| NonZeroU32::new(i as u32 + 1))
                .collect(),
        });
        let ctts_box = timing.composition_offsets.as_ref().map(|offsets| {
            let mut entries: Vec<CttsEntry> = Vec::new();
            for &offset in offsets {
                match entries.last_mut() {
                    Some(last) if last.sample_offset == offset => last.sample_count += 1,
                    _ => entries.push(CttsEntry {
                        sample_count: 1,
                        sample_offset: offset,
                    }),
                }
            }
            CttsBox {
                version: u8::from(offsets.iter().any(|&o| o < 0)),
                entries,
            }
        });
        let stbl_box = StblBox {
            stsd_box: StsdBox {
                entries: vec![video_sample_entry(self.video_config)?],
            },
            stts_box: SttsBox::from_sample_deltas(timing.durations.iter().copied()),
            ctts_box,
            cslg_box: None,
            stsc_box: stsc_box(&self.video.chunks),
            stsz_box: stsz_box(samples),
            stco_or_co64_box: chunk_offsets(&self.video.chunks, data_start, use_co64),
            stss_box,
            sdtp_box: None,
            unknown_boxes: Vec::new(),
        };
        let width = self.video_config.width as i16;
        let height = self.video_config.height as i16;
        Ok(TrakBox {
            tkhd_box: TkhdBox {
                flag_track_enabled: true,
                flag_track_in_movie: true,
                flag_track_in_preview: false,
                flag_track_size_is_aspect_ratio: false,
                creation_time: self.creation_time,
                modification_time: self.creation_time,
                track_id: VIDEO_TRACK_ID,
                duration: presentation,
                layer: TkhdBox::DEFAULT_LAYER,
                alternate_group: TkhdBox::DEFAULT_ALTERNATE_GROUP,
                volume: TkhdBox::DEFAULT_VIDEO_VOLUME,
                matrix: TkhdBox::DEFAULT_MATRIX,
                width: FixedPointNumber::new(width, 0),
                height: FixedPointNumber::new(height, 0),
            },
            edts_box,
            mdia_box: MdiaBox {
                mdhd_box: MdhdBox {
                    creation_time: self.creation_time,
                    modification_time: self.creation_time,
                    timescale: NonZeroU32::new(VIDEO_TIMESCALE).expect("non-zero"),
                    duration: media_duration,
                    language: MdhdBox::LANGUAGE_UNDEFINED,
                },
                hdlr_box: HdlrBox {
                    handler_type: HdlrBox::HANDLER_TYPE_VIDE,
                    name: b"VideoHandler\0".to_vec(),
                },
                minf_box: MinfBox {
                    smhd_or_vmhd_box: Some(Either::B(VmhdBox::default())),
                    dinf_box: DinfBox::LOCAL_FILE,
                    stbl_box,
                    unknown_boxes: Vec::new(),
                },
                unknown_boxes: Vec::new(),
            },
            unknown_boxes: Vec::new(),
        })
    }

    fn audio_trak(
        &self,
        audio: &AudioTrack,
        timing: &AudioTiming,
        data_start: u64,
        use_co64: bool,
    ) -> TrakBox {
        let rate = audio.config.sample_rate;
        let media_duration = timing.media_duration();
        let played = to_movie_time(media_duration.saturating_sub(timing.media_start), rate);
        let empty = to_movie_time(timing.empty, rate);
        let mut entries = Vec::new();
        if empty > 0 {
            entries.push(ElstEntry {
                edit_duration: empty,
                media_time: -1,
                media_rate: FixedPointNumber::new(1, 0),
            });
        }
        if empty > 0 || timing.media_start > 0 {
            entries.push(ElstEntry {
                edit_duration: played,
                media_time: timing.media_start as i64,
                media_rate: FixedPointNumber::new(1, 0),
            });
        }
        let edts_box = (!entries.is_empty()).then(|| EdtsBox {
            elst_box: Some(ElstBox { entries }),
            unknown_boxes: Vec::new(),
        });
        let stbl_box = StblBox {
            stsd_box: StsdBox {
                entries: vec![audio_sample_entry(
                    &audio.config,
                    &audio.track,
                    media_duration,
                )],
            },
            stts_box: SttsBox::from_sample_deltas(timing.durations.iter().copied()),
            ctts_box: None,
            cslg_box: None,
            stsc_box: stsc_box(&audio.track.chunks),
            stsz_box: stsz_box(&audio.track.samples),
            stco_or_co64_box: chunk_offsets(&audio.track.chunks, data_start, use_co64),
            stss_box: None,
            sdtp_box: None,
            unknown_boxes: Vec::new(),
        };
        TrakBox {
            tkhd_box: TkhdBox {
                flag_track_enabled: true,
                flag_track_in_movie: true,
                flag_track_in_preview: false,
                flag_track_size_is_aspect_ratio: false,
                creation_time: self.creation_time,
                modification_time: self.creation_time,
                track_id: AUDIO_TRACK_ID,
                duration: empty + played,
                layer: TkhdBox::DEFAULT_LAYER,
                alternate_group: TkhdBox::DEFAULT_ALTERNATE_GROUP,
                volume: TkhdBox::DEFAULT_AUDIO_VOLUME,
                matrix: TkhdBox::DEFAULT_MATRIX,
                width: FixedPointNumber::default(),
                height: FixedPointNumber::default(),
            },
            edts_box,
            mdia_box: MdiaBox {
                mdhd_box: MdhdBox {
                    creation_time: self.creation_time,
                    modification_time: self.creation_time,
                    timescale: NonZeroU32::new(rate).unwrap_or(NonZeroU32::MIN),
                    duration: media_duration,
                    language: MdhdBox::LANGUAGE_UNDEFINED,
                },
                hdlr_box: HdlrBox {
                    handler_type: HdlrBox::HANDLER_TYPE_SOUN,
                    name: b"SoundHandler\0".to_vec(),
                },
                minf_box: MinfBox {
                    smhd_or_vmhd_box: Some(Either::A(SmhdBox::default())),
                    dinf_box: DinfBox::LOCAL_FILE,
                    stbl_box,
                    unknown_boxes: Vec::new(),
                },
                unknown_boxes: Vec::new(),
            },
            unknown_boxes: Vec::new(),
        }
    }
}

fn stsc_box(chunks: &[ChunkInfo]) -> StscBox {
    let mut entries: Vec<StscEntry> = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        if entries
            .last()
            .is_some_and(|last| last.sample_per_chunk == chunk.samples)
        {
            continue;
        }
        entries.push(StscEntry {
            first_chunk: NonZeroU32::new(i as u32 + 1).expect("non-zero"),
            sample_per_chunk: chunk.samples,
            sample_description_index: NonZeroU32::MIN,
        });
    }
    StscBox { entries }
}

fn stsz_box(samples: &[SampleInfo]) -> StszBox {
    let first = samples.first().map_or(0, |s| s.size);
    match NonZeroU32::new(first) {
        Some(size) if samples.iter().all(|s| s.size == first) => StszBox::Fixed {
            sample_size: size,
            sample_count: samples.len() as u32,
        },
        _ => StszBox::Variable {
            entry_sizes: samples.iter().map(|s| s.size).collect(),
        },
    }
}

fn chunk_offsets(
    chunks: &[ChunkInfo],
    data_start: u64,
    use_co64: bool,
) -> Either<StcoBox, Co64Box> {
    if use_co64 {
        Either::B(Co64Box {
            chunk_offsets: chunks.iter().map(|c| data_start + c.offset).collect(),
        })
    } else {
        Either::A(StcoBox {
            // The caller checked that every offset fits in 32 bits.
            chunk_offsets: chunks
                .iter()
                .map(|c| (data_start + c.offset) as u32)
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::h264;
    use crate::media::h264::tests::{PPS_HIGH_640X360, SPS_HIGH_640X360, hex};
    use crate::media::h265;
    use crate::media::h265::tests::{PPS_640X360, SPS_640X360, VPS_640X360};
    use shiguredo_mp4::TrackKind;
    use shiguredo_mp4::demux::{Input, Mp4FileDemuxer};

    fn h264_config() -> VideoConfig {
        h264::video_config(&[hex(SPS_HIGH_640X360)], &[hex(PPS_HIGH_640X360)]).unwrap()
    }

    fn h265_config() -> VideoConfig {
        h265::video_config(
            &[hex(VPS_640X360)],
            &[hex(SPS_640X360)],
            &[hex(PPS_640X360)],
        )
        .unwrap()
    }

    fn frame(pts: i64, keyframe: bool, size: usize) -> VideoFrame {
        let mut data = (size as u32 - 4).to_be_bytes().to_vec();
        data.push(if keyframe { 0x65 } else { 0x41 });
        data.resize(size, 0xAB);
        VideoFrame {
            pts,
            dts: pts,
            keyframe,
            data: Bytes::from(data),
        }
    }

    struct DemuxedSample {
        kind: TrackKind,
        timestamp: u64,
        duration: u32,
        keyframe: bool,
        size: usize,
        offset: u64,
        composition_offset: Option<i64>,
    }

    struct Demuxed {
        /// (kind, duration, timescale)
        tracks: Vec<(TrackKind, u64, u32)>,
        /// In presentation order across tracks.
        samples: Vec<DemuxedSample>,
        video_entry: Option<SampleEntry>,
    }

    fn demux(file: &[u8]) -> Demuxed {
        let mut demuxer = Mp4FileDemuxer::new();
        while let Some(required) = demuxer.required_input() {
            let start = required.position as usize;
            let end = required
                .size
                .map_or(file.len(), |size| (start + size).min(file.len()));
            demuxer.handle_input(Input {
                position: required.position,
                data: &file[start..end],
            });
        }
        let tracks = demuxer
            .tracks()
            .unwrap()
            .iter()
            .map(|t| (t.kind, t.duration, t.timescale.get()))
            .collect();
        let mut samples = Vec::new();
        let mut video_entry = None;
        while let Some(sample) = demuxer.next_sample().unwrap() {
            if sample.track.kind == TrackKind::Video
                && let Some(entry) = sample.sample_entry
            {
                video_entry = Some(entry.clone());
            }
            samples.push(DemuxedSample {
                kind: sample.track.kind,
                timestamp: sample.timestamp,
                duration: sample.duration,
                keyframe: sample.keyframe,
                size: sample.data_size,
                offset: sample.data_offset,
                composition_offset: sample.composition_time_offset,
            });
        }
        Demuxed {
            tracks,
            samples,
            video_entry,
        }
    }

    /// Top-level box types in order.
    fn top_level_boxes(file: &[u8]) -> Vec<String> {
        let mut boxes = Vec::new();
        let mut pos = 0usize;
        while pos + 8 <= file.len() {
            let mut size = u32::from_be_bytes(file[pos..pos + 4].try_into().unwrap()) as u64;
            let kind = String::from_utf8_lossy(&file[pos + 4..pos + 8]).into_owned();
            if size == 1 {
                size = u64::from_be_bytes(file[pos + 8..pos + 16].try_into().unwrap());
            }
            boxes.push(kind);
            pos += size as usize;
        }
        boxes
    }

    #[test]
    fn writes_fast_start_h264_file() {
        let mut file = Vec::new();
        let mut writer = Mp4Writer::new(&mut file, h264_config()).unwrap();
        writer.write_video(&frame(0, false, 100)).unwrap(); // before the first keyframe
        for i in 0..45 {
            writer
                .write_video(&frame(1_000_000 + i * 6000, i % 15 == 0, 200 + i as usize))
                .unwrap();
        }
        let summary = writer.finish().unwrap();
        assert_eq!(summary.video_frames, 45);
        assert_eq!(summary.keyframes, 3);
        assert_eq!(summary.dropped_video_frames, 1);
        assert_eq!(summary.duration, Duration::from_secs(3));
        assert_eq!(summary.bytes_written, file.len() as u64);
        assert_eq!(top_level_boxes(&file), vec!["ftyp", "moov", "mdat"]);

        let demuxed = demux(&file);
        assert_eq!(demuxed.tracks, vec![(TrackKind::Video, 45 * 6000, 90_000)]);
        assert_eq!(demuxed.samples.len(), 45);
        for (i, sample) in demuxed.samples.iter().enumerate() {
            assert_eq!(sample.timestamp, i as u64 * 6000);
            assert_eq!(sample.duration, 6000);
            assert_eq!(sample.keyframe, i % 15 == 0);
            assert_eq!(sample.size, 200 + i);
            assert_eq!(sample.composition_offset, None);
        }
        // Samples are stored back to back after the moov.
        let first = demuxed.samples[0].offset as usize;
        assert_eq!(&file[first - 4..first], b"mdat");
        assert_eq!(&file[first + 4..first + 5], &[0x65]);
        let Some(SampleEntry::Avc1(avc1)) = demuxed.video_entry else {
            panic!("expected avc1");
        };
        assert_eq!((avc1.visual.width, avc1.visual.height), (640, 360));
        let mut avcc = avc1.avcc_box.encode_to_vec().unwrap();
        avcc.drain(..8);
        assert_eq!(avcc, h264_config().description.to_vec());
    }

    #[test]
    fn writes_hvc1_sample_entry() {
        let mut file = Vec::new();
        let config = h265_config();
        let mut writer = Mp4Writer::new(&mut file, config.clone()).unwrap();
        for i in 0..10 {
            writer.write_video(&frame(i * 3000, i == 0, 50)).unwrap();
        }
        writer.finish().unwrap();
        let demuxed = demux(&file);
        let Some(SampleEntry::Hvc1(hvc1)) = demuxed.video_entry else {
            panic!("expected hvc1");
        };
        let mut hvcc = hvc1.hvcc_box.encode_to_vec().unwrap();
        hvcc.drain(..8);
        assert_eq!(hvcc, config.description.to_vec());
        // Fixed-size samples use the compact stsz form; all but one are non-sync.
        assert_eq!(demuxed.samples.len(), 10);
        assert!(demuxed.samples.iter().all(|s| s.size == 50));
        assert_eq!(demuxed.samples.iter().filter(|s| s.keyframe).count(), 1);
    }

    #[test]
    fn gaps_and_discontinuities() {
        let mut file = Vec::new();
        let mut writer = Mp4Writer::new(&mut file, h264_config()).unwrap();
        // A 1.3 s hole after the third frame, then a jump back in time.
        let times = [0, 3000, 6000, 126_000, 129_000, 50_000, 53_000];
        for (i, &t) in times.iter().enumerate() {
            writer.write_video(&frame(t, i == 0, 64)).unwrap();
        }
        writer.finish().unwrap();
        let durations: Vec<u32> = demux(&file).samples.iter().map(|s| s.duration).collect();
        // The gap is kept (held frame); the backward jump is stitched.
        assert_eq!(durations, vec![3000, 3000, 120_000, 3000, 3000, 3000, 3000]);

        let mut file = Vec::new();
        let options = Mp4WriterOptions::new().max_gap(Duration::from_secs(1));
        let mut writer = Mp4Writer::with_options(&mut file, h264_config(), options).unwrap();
        for (i, &t) in times.iter().enumerate() {
            writer.write_video(&frame(t, i == 0, 64)).unwrap();
        }
        writer.finish().unwrap();
        let durations: Vec<u32> = demux(&file).samples.iter().map(|s| s.duration).collect();
        assert_eq!(durations, vec![3000; 7]);
    }

    #[test]
    fn composition_offsets_get_ctts_and_edit_list() {
        let mut file = Vec::new();
        let mut writer = Mp4Writer::new(&mut file, h264_config()).unwrap();
        // I P B B with DTS lagging PTS by two frames.
        let pts = [6000, 15_000, 9000, 12_000];
        for (i, &p) in pts.iter().enumerate() {
            let mut f = frame(p, i == 0, 80);
            f.dts = i as i64 * 3000;
            writer.write_video(&f).unwrap();
        }
        let summary = writer.finish().unwrap();
        let demuxed = demux(&file);
        let mut samples = demuxed.samples;
        samples.sort_by_key(|s| s.offset);
        let offsets: Vec<Option<i64>> = samples.iter().map(|s| s.composition_offset).collect();
        assert_eq!(
            offsets,
            vec![Some(6000), Some(12_000), Some(3000), Some(3000)]
        );
        // Four frames of 3000 ticks, presented from the first frame on.
        assert_eq!(summary.duration, ticks_to_duration(12_000));
        let elst = b"elst";
        let at = file.windows(4).position(|w| w == elst).expect("edit list");
        // version/flags, entry count, then duration (ms) and media time (ticks).
        let entry = &file[at + 12..at + 20];
        assert_eq!(u32::from_be_bytes(entry[..4].try_into().unwrap()), 134);
        assert_eq!(u32::from_be_bytes(entry[4..].try_into().unwrap()), 6000);
    }

    #[test]
    fn aac_track_is_interleaved_and_aligned() {
        let asc = aac::AudioSpecificConfig::new(2, 16_000, 1).to_bytes();
        let config = AacTrackConfig::new(asc).unwrap();
        assert_eq!((config.sample_rate, config.channels), (16_000, 1));
        let frame_ticks = config.frame_ticks(); // 5760
        let mut file = Vec::new();
        let mut writer = Mp4Writer::new(&mut file, h264_config()).unwrap();
        writer.add_aac_track(config).unwrap();
        assert!(
            writer
                .add_aac_track(AacTrackConfig::new(vec![0x14, 0x08]).unwrap())
                .is_err()
        );

        let start = 900_000;
        // Audio from 1 s before the video to 3 s after, with jittery timestamps.
        let mut audio_pts = start - 90_000 + 1000;
        let mut video_pts = start;
        let mut audio_written = 0;
        while video_pts < start + 3 * 90_000 {
            while audio_pts <= video_pts {
                let jitter = if audio_written % 3 == 0 { 7 } else { -5 };
                writer.write_aac(audio_pts + jitter, &[0x21; 40]).unwrap();
                audio_written += 1;
                audio_pts += frame_ticks;
            }
            writer
                .write_video(&frame(video_pts, (video_pts - start) % 90_000 == 0, 300))
                .unwrap();
            video_pts += 6000;
        }
        let summary = writer.finish().unwrap();
        assert_eq!(summary.video_frames, 45);
        // Frames ending before the first video frame are dropped.
        let early = (90_000 - 1000) / frame_ticks;
        assert_eq!(summary.dropped_audio_frames, early as u64);
        assert_eq!(summary.audio_frames, audio_written - early as u64);

        let demuxed = demux(&file);
        assert_eq!(demuxed.tracks.len(), 2);
        assert_eq!(demuxed.tracks[1].0, TrackKind::Audio);
        assert_eq!(demuxed.tracks[1].2, 16_000);
        let audio: Vec<_> = demuxed
            .samples
            .iter()
            .filter(|s| s.kind == TrackKind::Audio)
            .collect();
        assert_eq!(audio.len() as u64, summary.audio_frames);
        // Jitter is absorbed: every frame lasts exactly 1024 samples.
        assert!(audio.iter().all(|s| s.duration == 1024));
        // Chunks of about a second alternate in the file: V A V A V A.
        let mut by_offset: Vec<_> = demuxed.samples.iter().collect();
        by_offset.sort_by_key(|s| s.offset);
        let switches = by_offset
            .windows(2)
            .filter(|w| w[0].kind != w[1].kind)
            .count();
        assert_eq!(switches, 5);
        // The audio edit skips the part of the first frame before the video starts.
        assert!(file.windows(4).any(|w| w == b"elst"));
    }

    #[test]
    fn rejects_config_changes_and_empty_files() {
        let mut writer = Mp4Writer::new(Vec::new(), h264_config()).unwrap();
        writer.set_video_config(&h264_config()).unwrap();
        let err = writer.set_video_config(&h265_config()).unwrap_err();
        assert!(
            matches!(err, MediaError::VideoConfigChanged { .. }),
            "{err}"
        );
        assert!(writer.write_aac(0, &[1]).is_err());
        assert!(matches!(writer.finish(), Err(MediaError::InvalidInput(_))));
    }

    #[test]
    fn audio_timing_edits() {
        let config = AacTrackConfig::new(vec![0x14, 0x08]).unwrap(); // 16 kHz
        let ticks = config.frame_ticks();
        let sample = |pts| SampleInfo {
            size: 10,
            dts: pts,
            pts,
            keyframe: true,
        };
        // Audio starts 0.5 s after the video.
        let samples: Vec<_> = (0..4).map(|i| sample(45_000 + i * ticks)).collect();
        let timing = audio_timing(&samples, &config, 0, None);
        assert_eq!(timing.empty, 8000);
        assert_eq!(timing.media_start, 0);
        assert_eq!(timing.durations, vec![1024; 4]);

        // Audio starts 10 ms before the video, with an encoder delay of 1024 samples.
        let delayed = config.clone().with_encoder_delay(1024);
        let samples: Vec<_> = (0..4).map(|i| sample(-900 + i * ticks)).collect();
        let timing = audio_timing(&samples, &delayed, 0, None);
        assert_eq!(timing.empty, 1024 - 160);
        assert_eq!(timing.media_start, 1024);

        // A 2 s hole is kept unless collapsed.
        let mut samples: Vec<_> = (0..3).map(|i| sample(i * ticks)).collect();
        samples.push(sample(2 * ticks + 180_000 + ticks));
        let timing = audio_timing(&samples, &config, 0, None);
        assert_eq!(timing.durations[2], 1024 + 32_000);
        let timing = audio_timing(&samples, &config, 0, Some(90_000));
        assert_eq!(timing.durations[2], 1024);
    }

    #[test]
    fn duration_conversion() {
        assert_eq!(ticks_to_duration(90_000), Duration::from_secs(1));
        assert_eq!(ticks_to_duration(45), Duration::from_micros(500));
        assert_eq!(ticks_to_duration(-5), Duration::ZERO);
        assert_eq!(to_movie_time(90_000, 90_000), 1000);
        assert_eq!(to_movie_time(1, 90_000), 1);
    }
}
