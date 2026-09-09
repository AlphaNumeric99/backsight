//! `tapo remux <input.mpegts> <output.mp4>`: converts a camera transport stream (a
//! recording or a captured live stream) into a fast-start MP4 file.
//!
//! H.264 and H.265 video are copied as is. AAC audio is copied; G.711 audio is reported
//! but not written, because MP4 players expect AAC. If the video configuration changes
//! (for example the resolution), the current file is finished and the rest goes to
//! `<output>-2.mp4`, `<output>-3.mp4`, and so on.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;
use tapo_camera::media::{
    AacTrackConfig, AudioCodec, AudioConfig, AudioFrame, DemuxStats, MediaError, MediaEvent,
    Mp4Summary, Mp4Writer, TsDemuxer, VideoConfig, aac, ts,
};

/// Audio frames held while waiting for the first video configuration.
const MAX_EARLY_AUDIO_FRAMES: usize = 1024;

/// Arguments of `tapo remux`.
#[derive(Debug, Args)]
pub struct RemuxArgs {
    /// MPEG-TS input, e.g. a recording downloaded from a camera.
    pub input: PathBuf,

    /// MP4 file to write.
    pub output: PathBuf,

    /// Sample rate of G.711 audio in the stream (the camera's audio config); only
    /// reported, since G.711 is not written to MP4.
    #[arg(long, value_name = "HZ", default_value_t = ts::DEFAULT_AUDIO_SAMPLE_RATE)]
    pub audio_rate: u32,
}

/// Runs `tapo remux` and prints what was written.
pub fn run(args: &RemuxArgs) -> Result<()> {
    let mut input =
        File::open(&args.input).with_context(|| format!("cannot open {}", args.input.display()))?;
    let input_size = input.metadata().map(|m| m.len()).unwrap_or(0);

    let mut demuxer = TsDemuxer::new().with_audio_rate(args.audio_rate);
    let mut remuxer = Remuxer::new(&args.output);
    let mut events = Vec::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let read = input
            .read(&mut buf)
            .with_context(|| format!("cannot read {}", args.input.display()))?;
        if read == 0 {
            break;
        }
        demux_result(demuxer.push(&buf[..read], &mut events))?;
        remuxer.handle(events.drain(..))?;
    }
    demux_result(demuxer.flush(&mut events))?;
    remuxer.handle(events.drain(..))?;
    let report = remuxer.finish()?;

    print_report(&args.input, input_size, &report, demuxer.stats());
    Ok(())
}

fn demux_result(result: std::result::Result<(), MediaError>) -> Result<()> {
    match result {
        Err(err @ MediaError::LostSync { .. }) => {
            Err(err).context("the input does not look like MPEG-TS")
        }
        other => other.context("demuxing failed"),
    }
}

/// One finished MP4 file.
struct Segment {
    path: PathBuf,
    summary: Mp4Summary,
    audio: Option<AudioConfig>,
}

struct Report {
    segments: Vec<Segment>,
    /// Audio that could not be written (G.711).
    skipped_audio: Option<AudioConfig>,
    /// AAC frames dropped because their configuration changed mid-file.
    dropped_aac_frames: u64,
}

/// Feeds demuxer events into MP4 writers, one per video configuration.
struct Remuxer {
    output: PathBuf,
    writer: Option<Mp4Writer<std::io::BufWriter<File>>>,
    segment_path: PathBuf,
    segments: Vec<Segment>,
    audio_config: Option<AudioConfig>,
    /// The AAC config of the current file's audio track.
    track_audio: Option<(AacTrackConfig, AudioConfig)>,
    early_audio: Vec<AudioFrame>,
    skipped_audio: Option<AudioConfig>,
    dropped_aac_frames: u64,
}

impl Remuxer {
    fn new(output: &Path) -> Self {
        Self {
            output: output.to_path_buf(),
            writer: None,
            segment_path: output.to_path_buf(),
            segments: Vec::new(),
            audio_config: None,
            track_audio: None,
            early_audio: Vec::new(),
            skipped_audio: None,
            dropped_aac_frames: 0,
        }
    }

    fn handle(&mut self, events: impl Iterator<Item = MediaEvent>) -> Result<()> {
        for event in events {
            match event {
                MediaEvent::VideoConfig(config) => self.on_video_config(config)?,
                MediaEvent::Video(frame) => {
                    if let Some(writer) = self.writer.as_mut() {
                        writer.write_video(&frame)?;
                    }
                }
                MediaEvent::AudioConfig(config) => {
                    if config.codec != AudioCodec::Aac {
                        self.skipped_audio = Some(config);
                    }
                    self.audio_config = Some(config);
                }
                MediaEvent::Audio(frame) => self.on_audio(frame)?,
            }
        }
        Ok(())
    }

    fn on_video_config(&mut self, config: VideoConfig) -> Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            match writer.set_video_config(&config) {
                Ok(()) => return Ok(()),
                // A new resolution or codec: finish this file and start the next one.
                Err(MediaError::VideoConfigChanged { .. }) => self.finish_segment()?,
                Err(err) => return Err(err.into()),
            }
        }
        self.segment_path = segment_path(&self.output, self.segments.len() + 1);
        let writer = Mp4Writer::create(&self.segment_path, config)
            .with_context(|| format!("cannot create {}", self.segment_path.display()))?;
        self.writer = Some(writer);
        for frame in std::mem::take(&mut self.early_audio) {
            self.on_audio(frame)?;
        }
        Ok(())
    }

    fn on_audio(&mut self, frame: AudioFrame) -> Result<()> {
        if self.audio_config.is_none_or(|c| c.codec != AudioCodec::Aac) {
            return Ok(());
        }
        let Some(writer) = self.writer.as_mut() else {
            if self.early_audio.len() < MAX_EARLY_AUDIO_FRAMES {
                self.early_audio.push(frame);
            }
            return Ok(());
        };
        let Ok(header) = aac::AdtsHeader::parse(&frame.data) else {
            return Ok(());
        };
        let track_config = AacTrackConfig::from_adts(&header)?;
        match &self.track_audio {
            None => {
                writer.add_aac_track(track_config.clone())?;
                let audio = AudioConfig {
                    codec: AudioCodec::Aac,
                    sample_rate: track_config.sample_rate,
                    channels: track_config.channels,
                };
                self.track_audio = Some((track_config, audio));
            }
            Some((current, _)) if *current != track_config => {
                self.dropped_aac_frames += 1;
                return Ok(());
            }
            Some(_) => {}
        }
        writer.write_aac(frame.pts, &frame.data[header.header_len()..])?;
        Ok(())
    }

    fn finish_segment(&mut self) -> Result<()> {
        if let Some(writer) = self.writer.take() {
            let summary = writer
                .finish()
                .with_context(|| format!("cannot write {}", self.segment_path.display()))?;
            self.segments.push(Segment {
                path: self.segment_path.clone(),
                summary,
                audio: self.track_audio.take().map(|(_, audio)| audio),
            });
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Report> {
        self.finish_segment()?;
        if self.segments.is_empty() {
            bail!("no playable video found in the input (no parameter sets or keyframe)");
        }
        Ok(Report {
            segments: self.segments,
            skipped_audio: self.skipped_audio,
            dropped_aac_frames: self.dropped_aac_frames,
        })
    }
}

/// `out.mp4`, then `out-2.mp4`, `out-3.mp4`, … for later segments.
fn segment_path(output: &Path, index: usize) -> PathBuf {
    if index <= 1 {
        return output.to_path_buf();
    }
    let stem = output
        .file_stem()
        .map_or_else(|| "output".into(), |s| s.to_string_lossy().into_owned());
    let name = match output.extension() {
        Some(ext) => format!("{stem}-{index}.{}", ext.to_string_lossy()),
        None => format!("{stem}-{index}"),
    };
    output.with_file_name(name)
}

fn describe_audio(config: &AudioConfig) -> String {
    let channels = match config.channels {
        1 => "mono".to_owned(),
        2 => "stereo".to_owned(),
        n => format!("{n} channels"),
    };
    format!("{} {} Hz {channels}", config.codec, config.sample_rate)
}

fn print_report(input: &Path, input_size: u64, report: &Report, stats: &DemuxStats) {
    println!("Input:     {} ({input_size} bytes)", input.display());
    for segment in &report.segments {
        let s = &segment.summary;
        println!(
            "Output:    {} ({} bytes)",
            segment.path.display(),
            s.bytes_written
        );
        println!(
            "Video:     {} {}, {}x{}",
            s.codec, s.codec_string, s.width, s.height
        );
        println!("Frames:    {} ({} keyframes)", s.video_frames, s.keyframes);
        println!("Duration:  {:.3} s", s.duration.as_secs_f64());
        match &segment.audio {
            Some(audio) => println!(
                "Audio:     {}, {} frames",
                describe_audio(audio),
                s.audio_frames
            ),
            None => match &report.skipped_audio {
                Some(audio) => println!(
                    "Audio:     {} (not written: MP4 audio must be AAC)",
                    describe_audio(audio)
                ),
                None => println!("Audio:     none"),
            },
        }
    }
    let dropped_video = stats.dropped_video_frames
        + report
            .segments
            .iter()
            .map(|s| s.summary.dropped_video_frames)
            .sum::<u64>();
    if dropped_video > 0 {
        println!(
            "Dropped:   {dropped_video} video frames (damaged, or before the first parameter sets/keyframe)"
        );
    }
    if stats.continuity_errors > 0 || stats.skipped_bytes > 0 {
        println!(
            "Warnings:  {} continuity errors, {} bytes of non-TS data skipped",
            stats.continuity_errors, stats.skipped_bytes
        );
    }
    if report.dropped_aac_frames > 0 {
        println!(
            "Warnings:  {} AAC frames dropped after an audio configuration change",
            report.dropped_aac_frames
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_segments() {
        let out = Path::new("dir").join("clip.mp4");
        assert_eq!(segment_path(&out, 1), out);
        assert_eq!(segment_path(&out, 2), Path::new("dir").join("clip-2.mp4"));
        assert_eq!(segment_path(Path::new("clip"), 3), Path::new("clip-3"));
    }

    #[test]
    fn describes_audio() {
        let config = AudioConfig {
            codec: AudioCodec::PcmAlaw,
            sample_rate: 8000,
            channels: 1,
        };
        assert_eq!(describe_audio(&config), "G.711 A-law 8000 Hz mono");
    }
}
