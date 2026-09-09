//! End-to-end tests: MPEG-TS fixtures through `TsDemuxer`, then `Mp4Writer`, checked with
//! the MP4 demuxer of `shiguredo_mp4` and, when installed, `ffprobe`/`ffmpeg`.

mod support;

use std::path::Path;
use std::process::Command;

use bytes::Bytes;
use shiguredo_mp4::TrackKind;
use shiguredo_mp4::demux::{Input, Mp4FileDemuxer};
use tapo_camera::media::{
    AacTrackConfig, AudioCodec, AudioConfig, AudioFrame, MediaEvent, Mp4Summary, Mp4Writer,
    TsDemuxer, VideoCodec, VideoConfig, VideoFrame, aac,
};

use support::{
    AccessUnit, Codec, PesOptions, Rng, STREAM_TYPE_PCMU_TAPO, StreamOptions, TAPO_VIDEO_PID,
    TsMuxer, WRAP, extract_es, fixture, have_tool, split_access_units, tapo_stream,
};

const H264_TAPO: &str = "h264_alaw_tapo.mpegts";
const H265_TAPO: &str = "h265_alaw_tapo.mpegts";
const H264_AAC: &str = "h264_aac_ffmpeg.mpegts";
const FIXTURES: [&str; 3] = [H264_TAPO, H265_TAPO, H264_AAC];

fn demux_with(mut demuxer: TsDemuxer, data: &[u8]) -> (Vec<MediaEvent>, TsDemuxer) {
    let mut events = Vec::new();
    demuxer.push(data, &mut events).unwrap();
    demuxer.flush(&mut events).unwrap();
    (events, demuxer)
}

fn demux(data: &[u8]) -> Vec<MediaEvent> {
    demux_with(TsDemuxer::new(), data).0
}

fn demux_chunked(data: &[u8], mut chunk_size: impl FnMut() -> usize) -> Vec<MediaEvent> {
    let mut demuxer = TsDemuxer::new();
    let mut events = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let (chunk, tail) = rest.split_at(chunk_size().min(rest.len()));
        demuxer.push(chunk, &mut events).unwrap();
        rest = tail;
    }
    demuxer.flush(&mut events).unwrap();
    events
}

#[derive(Default)]
struct Streams {
    video_configs: Vec<VideoConfig>,
    frames: Vec<VideoFrame>,
    audio_configs: Vec<AudioConfig>,
    audio: Vec<AudioFrame>,
}

fn split(events: &[MediaEvent]) -> Streams {
    let mut streams = Streams::default();
    for event in events {
        match event {
            MediaEvent::VideoConfig(c) => streams.video_configs.push(c.clone()),
            MediaEvent::Video(f) => {
                assert!(
                    !streams.video_configs.is_empty(),
                    "frame before any VideoConfig"
                );
                streams.frames.push(f.clone());
            }
            MediaEvent::AudioConfig(c) => streams.audio_configs.push(*c),
            MediaEvent::Audio(a) => {
                assert!(!streams.audio_configs.is_empty(), "audio before its config");
                streams.audio.push(a.clone());
            }
        }
    }
    streams
}

/// Checks the frames of a 3 s, 15 fps, GOP 15 fixture clip.
fn check_clip_frames(frames: &[VideoFrame], codec: VideoCodec) {
    assert_eq!(frames.len(), 45);
    let keyframes: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|(_, f)| f.keyframe)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(keyframes, vec![0, 15, 30]);
    for pair in frames.windows(2) {
        assert_eq!(pair[1].pts - pair[0].pts, 6000, "15 fps timestamps");
        assert_eq!(pair[1].dts - pair[0].dts, 6000);
    }
    for frame in frames {
        assert_eq!(frame.pts, frame.dts, "no B-frames");
        let mut total = 0;
        for nal in frame.nal_units() {
            total += 4 + nal.len();
            let parameter_set_or_aud = match codec {
                VideoCodec::H264 => matches!(nal[0] & 0x1F, 7..=9),
                VideoCodec::H265 => matches!((nal[0] >> 1) & 0x3F, 32..=35),
            };
            assert!(
                !parameter_set_or_aud,
                "parameter sets and AUDs are stripped"
            );
        }
        assert_eq!(total, frame.data.len(), "length prefixes cover the frame");
    }
}

#[test]
fn h264_tapo_fixture() {
    let streams = split(&demux(&fixture(H264_TAPO)));
    assert_eq!(streams.video_configs.len(), 1);
    let config = &streams.video_configs[0];
    assert_eq!(config.codec, VideoCodec::H264);
    assert!(
        config.codec_string.starts_with("avc1.64"),
        "{}",
        config.codec_string
    );
    assert_eq!((config.width, config.height), (640, 360));
    assert_eq!(config.description[0], 1, "avcC version");
    check_clip_frames(&streams.frames, VideoCodec::H264);
    assert_eq!(streams.frames[0].pts, 900_000);

    assert_eq!(
        streams.audio_configs,
        vec![AudioConfig {
            codec: AudioCodec::PcmAlaw,
            sample_rate: 8000,
            channels: 1
        }]
    );
    let samples: usize = streams.audio.iter().map(|a| a.data.len()).sum();
    assert_eq!(samples, 24_000, "3 s of 8 kHz A-law");
    assert_eq!(streams.audio[0].pts, 900_000);
    for pair in streams.audio.windows(2) {
        assert_eq!(
            pair[1].pts - pair[0].pts,
            3600,
            "320-byte PES packets are 40 ms"
        );
    }
}

#[test]
fn h265_tapo_fixture_across_pts_wrap() {
    let streams = split(&demux(&fixture(H265_TAPO)));
    assert_eq!(streams.video_configs.len(), 1);
    let config = &streams.video_configs[0];
    assert_eq!(config.codec, VideoCodec::H265);
    assert!(
        config.codec_string.starts_with("hvc1.1.6.L"),
        "{}",
        config.codec_string
    );
    assert_eq!((config.width, config.height), (640, 360));
    check_clip_frames(&streams.frames, VideoCodec::H265);
    // The clip starts 1.5 s before the 33-bit wrap and continues past it.
    let first = streams.frames[0].pts;
    assert_eq!(first, (WRAP - 135_000) as i64);
    assert!(streams.frames[44].pts > WRAP as i64);
    assert!(streams.audio.windows(2).all(|p| p[1].pts > p[0].pts));
    assert!(streams.audio.last().unwrap().pts > WRAP as i64);
}

#[test]
fn aac_ffmpeg_fixture() {
    let streams = split(&demux(&fixture(H264_AAC)));
    assert_eq!(streams.video_configs.len(), 1);
    let config = &streams.video_configs[0];
    assert!(
        config.codec_string.starts_with("avc1.4D"),
        "Main profile: {}",
        config.codec_string
    );
    assert_eq!(
        (config.width, config.height),
        (480, 270),
        "cropped from 480x272"
    );
    check_clip_frames(&streams.frames, VideoCodec::H264);
    assert_eq!(
        streams.audio_configs,
        vec![AudioConfig {
            codec: AudioCodec::Aac,
            sample_rate: 16_000,
            channels: 1
        }]
    );
    // ffprobe counts 48 AAC frames; ffmpeg packs several per PES packet, and each
    // becomes its own AudioFrame with its own timestamp.
    assert_eq!(streams.audio.len(), 48);
    for pair in streams.audio.windows(2) {
        assert_eq!(pair[1].pts - pair[0].pts, 5760, "1024 samples at 16 kHz");
    }
    for frame in &streams.audio {
        let header = aac::AdtsHeader::parse(&frame.data).unwrap();
        assert_eq!(header.frame_length, frame.data.len());
    }
}

#[test]
fn output_does_not_depend_on_chunking() {
    for name in FIXTURES {
        let data = fixture(name);
        let whole = demux(&data);
        assert_eq!(demux_chunked(&data, || 1), whole, "{name}: byte by byte");
        assert_eq!(
            demux_chunked(&data, || 187),
            whole,
            "{name}: 187-byte chunks"
        );
        assert_eq!(
            demux_chunked(&data, || 189),
            whole,
            "{name}: 189-byte chunks"
        );
        for seed in 1..=8 {
            let mut rng = Rng::new(seed);
            let chunked = demux_chunked(&data, || rng.size(if seed % 2 == 0 { 64 } else { 4096 }));
            assert_eq!(chunked, whole, "{name}: random chunks, seed {seed}");
        }
    }
}

// ---------------------------------------------------------------------------------------
// MP4 export
// ---------------------------------------------------------------------------------------

/// Remuxes demuxed events into an MP4 file, as an exporter would: AAC audio that
/// arrives before the first video config is held until the writer exists.
fn remux(events: &[MediaEvent], path: &Path) -> Mp4Summary {
    let mut writer: Option<Mp4Writer<_>> = None;
    let mut early_audio: Vec<&AudioFrame> = Vec::new();
    let write_aac = |writer: &mut Mp4Writer<_>, frame: &AudioFrame| {
        let header = aac::AdtsHeader::parse(&frame.data).unwrap();
        if !writer.has_audio_track() {
            writer
                .add_aac_track(AacTrackConfig::from_adts(&header).unwrap())
                .unwrap();
        }
        writer
            .write_aac(frame.pts, &frame.data[header.header_len()..])
            .unwrap();
    };
    let mut audio_is_aac = false;
    for event in events {
        match event {
            MediaEvent::VideoConfig(config) => match writer.as_mut() {
                None => {
                    let mut new = Mp4Writer::create(path, config.clone()).unwrap();
                    for frame in early_audio.drain(..) {
                        write_aac(&mut new, frame);
                    }
                    writer = Some(new);
                }
                Some(writer) => writer.set_video_config(config).unwrap(),
            },
            MediaEvent::Video(frame) => writer.as_mut().unwrap().write_video(frame).unwrap(),
            // G.711 cannot go into MP4.
            MediaEvent::AudioConfig(config) => audio_is_aac = config.codec == AudioCodec::Aac,
            MediaEvent::Audio(frame) if audio_is_aac => match writer.as_mut() {
                Some(writer) => write_aac(writer, frame),
                None => early_audio.push(frame),
            },
            MediaEvent::Audio(_) => {}
        }
    }
    writer.unwrap().finish().unwrap()
}

struct Mp4Track {
    kind: TrackKind,
    timescale: u32,
    samples: Vec<(u64, u32, bool)>,
}

fn read_mp4(file: &[u8]) -> Vec<Mp4Track> {
    let mut demuxer = Mp4FileDemuxer::new();
    while let Some(required) = demuxer.required_input() {
        let start = required.position as usize;
        let end = required
            .size
            .map_or(file.len(), |s| (start + s).min(file.len()));
        demuxer.handle_input(Input {
            position: required.position,
            data: &file[start..end],
        });
    }
    let mut tracks: Vec<Mp4Track> = demuxer
        .tracks()
        .unwrap()
        .iter()
        .map(|t| Mp4Track {
            kind: t.kind,
            timescale: t.timescale.get(),
            samples: Vec::new(),
        })
        .collect();
    let ids: Vec<u32> = demuxer
        .tracks()
        .unwrap()
        .iter()
        .map(|t| t.track_id)
        .collect();
    while let Some(sample) = demuxer.next_sample().unwrap() {
        let index = ids
            .iter()
            .position(|&id| id == sample.track.track_id)
            .unwrap();
        tracks[index]
            .samples
            .push((sample.timestamp, sample.duration, sample.keyframe));
    }
    tracks
}

fn top_level_boxes(file: &[u8]) -> Vec<String> {
    let mut boxes = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= file.len() {
        let mut size = u64::from(u32::from_be_bytes(file[pos..pos + 4].try_into().unwrap()));
        if size == 1 {
            size = u64::from_be_bytes(file[pos + 8..pos + 16].try_into().unwrap());
        }
        boxes.push(String::from_utf8_lossy(&file[pos + 4..pos + 8]).into_owned());
        pos += size as usize;
    }
    boxes
}

fn ffprobe(path: &Path) -> serde_json::Value {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-of",
            "json",
            "-show_entries",
        ])
        .arg(
            "stream=codec_type,codec_name,codec_tag_string,width,height,nb_frames,\
             nb_read_frames,duration,sample_rate,channels:format=duration",
        )
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn decode_errors(path: &Path) -> String {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn seconds(value: &serde_json::Value) -> f64 {
    value.as_str().unwrap().parse().unwrap()
}

#[test]
fn remuxes_fixtures_to_mp4() {
    let dir = tempfile::tempdir().unwrap();
    let tools = have_tool("ffprobe") && have_tool("ffmpeg");
    if !tools {
        eprintln!("ffprobe/ffmpeg not on PATH: skipping the external checks");
    }
    let cases = [
        (H264_TAPO, "h264", "avc1", 640, 360, false),
        (H265_TAPO, "hevc", "hvc1", 640, 360, false),
        (H264_AAC, "h264", "avc1", 480, 270, true),
    ];
    for (name, codec_name, tag, width, height, has_audio) in cases {
        let path = dir.path().join(name.replace(".mpegts", ".mp4"));
        let summary = remux(&demux(&fixture(name)), &path);
        assert_eq!(summary.video_frames, 45, "{name}");
        assert_eq!(summary.keyframes, 3, "{name}");
        assert_eq!((summary.width, summary.height), (width, height), "{name}");
        assert_eq!(summary.duration.as_millis(), 3000, "{name}");
        assert_eq!(summary.audio_frames > 0, has_audio, "{name}");

        let file = std::fs::read(&path).unwrap();
        assert_eq!(summary.bytes_written, file.len() as u64);
        assert_eq!(
            top_level_boxes(&file),
            ["ftyp", "moov", "mdat"],
            "{name}: fast start"
        );
        let tracks = read_mp4(&file);
        assert_eq!(tracks.len(), if has_audio { 2 } else { 1 }, "{name}");
        let video = tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap();
        assert_eq!(video.timescale, 90_000);
        assert_eq!(video.samples.len(), 45);
        assert!(
            video
                .samples
                .iter()
                .all(|&(_, duration, _)| duration == 6000)
        );
        let sync: Vec<usize> = video
            .samples
            .iter()
            .enumerate()
            .filter(|(_, s)| s.2)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(sync, vec![0, 15, 30], "{name}: sync samples");

        if !tools {
            continue;
        }
        let probe = ffprobe(&path);
        let streams = probe["streams"].as_array().unwrap();
        let video = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
        assert_eq!(video["codec_name"], codec_name, "{name}");
        assert_eq!(video["codec_tag_string"], tag, "{name}");
        assert_eq!(video["width"], width, "{name}");
        assert_eq!(video["height"], height, "{name}");
        assert_eq!(video["nb_frames"], "45", "{name}");
        assert_eq!(video["nb_read_frames"], "45", "{name}: every frame decodes");
        assert!(
            (seconds(&video["duration"]) - 3.0).abs() < 0.001,
            "{name}: {video}"
        );
        if has_audio {
            let audio = streams.iter().find(|s| s["codec_type"] == "audio").unwrap();
            assert_eq!(audio["codec_name"], "aac");
            assert_eq!(audio["sample_rate"], "16000");
            assert_eq!(audio["channels"], 1);
            assert!((seconds(&audio["duration"]) - 3.0).abs() < 0.2, "{audio}");
        }
        assert!(
            (seconds(&probe["format"]["duration"]) - 3.0).abs() < 0.2,
            "{name}"
        );
        assert_eq!(decode_errors(&path), "", "{name}: decode errors");
    }
}

// ---------------------------------------------------------------------------------------
// Stream variations built from the H.264 fixture's elementary stream
// ---------------------------------------------------------------------------------------

fn h264_units() -> Vec<AccessUnit> {
    let es = extract_es(&fixture(H264_TAPO), TAPO_VIDEO_PID);
    let units = split_access_units(&es, Codec::H264);
    assert_eq!(units.len(), 45);
    units
}

fn alaw() -> Vec<u8> {
    let events = demux(&fixture(H264_TAPO));
    split(&events)
        .audio
        .iter()
        .flat_map(|a| a.data.to_vec())
        .collect()
}

fn without_parameter_sets(unit: &AccessUnit) -> AccessUnit {
    AccessUnit {
        nals: unit
            .nals
            .iter()
            .filter(|nal| !Codec::H264.is_parameter_set(nal))
            .cloned()
            .collect(),
    }
}

#[test]
fn stream_joined_mid_gop_starts_at_the_next_keyframe() {
    let units = h264_units();
    let ts = tapo_stream(&units[5..], &[], &StreamOptions::tapo(Codec::H264));
    let (events, demuxer) = demux_with(TsDemuxer::new(), &ts);
    let streams = split(&events);
    assert!(matches!(events[0], MediaEvent::VideoConfig(_)));
    // Frames 5..15 reference an IDR that was never received.
    assert_eq!(streams.frames.len(), 30);
    assert!(streams.frames[0].keyframe);
    assert_eq!(demuxer.stats().dropped_video_frames, 10);
}

#[test]
fn parameter_sets_sent_once_are_remembered() {
    let units = h264_units();
    let mut edited = vec![units[0].clone()];
    edited.extend(units[1..].iter().map(without_parameter_sets));
    let streams = split(&demux(&tapo_stream(
        &edited,
        &[],
        &StreamOptions::tapo(Codec::H264),
    )));
    assert_eq!(streams.video_configs.len(), 1);
    assert_eq!(streams.frames.len(), 45);
}

#[test]
fn stream_without_parameter_sets_emits_no_video() {
    let units: Vec<AccessUnit> = h264_units().iter().map(without_parameter_sets).collect();
    let ts = tapo_stream(&units, &alaw(), &StreamOptions::tapo(Codec::H264));
    let (events, demuxer) = demux_with(TsDemuxer::new(), &ts);
    let streams = split(&events);
    assert!(streams.video_configs.is_empty());
    assert!(streams.frames.is_empty());
    assert_eq!(demuxer.stats().dropped_video_frames, 45);
    // Audio does not depend on video.
    assert_eq!(streams.audio.len(), 75);
}

#[test]
fn parameter_sets_arriving_late_are_used_from_then_on() {
    let units = h264_units();
    // SPS/PPS only from the second GOP on.
    let edited: Vec<AccessUnit> = units
        .iter()
        .enumerate()
        .map(|(i, u)| {
            if i < 15 {
                without_parameter_sets(u)
            } else {
                u.clone()
            }
        })
        .collect();
    let streams = split(&demux(&tapo_stream(
        &edited,
        &[],
        &StreamOptions::tapo(Codec::H264),
    )));
    assert_eq!(streams.video_configs.len(), 1);
    assert_eq!(streams.frames.len(), 30);
    assert_eq!(streams.frames[0].pts, 900_000 + 15 * 6000);
}

#[test]
fn works_without_access_unit_delimiters() {
    let units = h264_units();
    let options = StreamOptions {
        strip_aud: true,
        ..StreamOptions::tapo(Codec::H264)
    };
    let with = split(&demux(&tapo_stream(
        &units,
        &[],
        &StreamOptions::tapo(Codec::H264),
    )));
    let without = split(&demux(&tapo_stream(&units, &[], &options)));
    assert_eq!(with.frames, without.frames);
    assert_eq!(with.video_configs, without.video_configs);
}

#[test]
fn configuration_change_emits_a_new_config() {
    // 640x360 High profile followed by 480x270 Main profile. ffmpeg's muxer inserts
    // AUDs, so its elementary stream splits the same way.
    let mut units = h264_units();
    let second = split_access_units(&extract_es(&fixture(H264_AAC), 0x100), Codec::H264);
    assert_eq!(second.len(), 45);
    units.extend(second);
    let streams = split(&demux(&tapo_stream(
        &units,
        &[],
        &StreamOptions::tapo(Codec::H264),
    )));
    assert_eq!(streams.video_configs.len(), 2);
    assert_eq!(
        (
            streams.video_configs[0].width,
            streams.video_configs[1].width
        ),
        (640, 480)
    );
    assert_eq!(streams.frames.len(), 90);
    assert!(streams.frames[45].keyframe);
    // The second config is emitted right before the first frame that uses it.
    let events = demux(&tapo_stream(&units, &[], &StreamOptions::tapo(Codec::H264)));
    let position = events
        .iter()
        .rposition(|e| matches!(e, MediaEvent::VideoConfig(_)))
        .unwrap();
    assert!(matches!(&events[position + 1], MediaEvent::Video(f) if f.keyframe));
}

#[test]
fn lost_packet_drops_frames_until_the_next_keyframe() {
    let ts = fixture(H264_TAPO);
    let (packets, _) = ts.as_chunks::<188>();
    // Find the second packet of frame 5's PES and drop it.
    let mut video_starts = 0;
    let mut drop_index = None;
    for (i, packet) in packets.iter().enumerate() {
        let pid = (u16::from(packet[1] & 0x1F) << 8) | u16::from(packet[2]);
        if pid == TAPO_VIDEO_PID && packet[1] & 0x40 != 0 {
            video_starts += 1;
            if video_starts == 6 {
                drop_index = Some(i + 1);
                break;
            }
        }
    }
    let drop_index = drop_index.unwrap();
    let damaged: Vec<u8> = packets
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != drop_index)
        .flat_map(|(_, p)| p.to_vec())
        .collect();
    let (events, demuxer) = demux_with(TsDemuxer::new(), &damaged);
    let frames = split(&events).frames;
    assert_eq!(demuxer.stats().continuity_errors, 1);
    // Frames 5 to 14 are gone; the stream resumes at the keyframe of frame 15.
    assert_eq!(frames.len(), 35);
    assert_eq!(frames[5].pts, 900_000 + 15 * 6000);
    assert!(frames[5].keyframe);
}

#[test]
fn garbage_between_packets_is_skipped() {
    let ts = fixture(H264_TAPO);
    let clean = demux(&ts);
    let mut noisy = b"HTTP/1.1 200 OK\r\n\r\n\x47\x47garbage".to_vec();
    let mut rng = Rng::new(7);
    let mut since_garbage = 0;
    for packet in ts.as_chunks::<188>().0 {
        noisy.extend_from_slice(packet);
        since_garbage += 1;
        // Sync is re-acquired on two consecutive packets, so garbage runs are at least
        // two packets apart (a lone packet between garbage is dropped by design).
        if since_garbage >= 2 && rng.next().is_multiple_of(20) {
            let len = rng.size(300);
            // Never 0x47, so garbage cannot fake a sync byte.
            noisy.extend((0..len).map(|_| (rng.next() % 64) as u8));
            since_garbage = 0;
        }
    }
    let (events, demuxer) = demux_with(TsDemuxer::new(), &noisy);
    assert_eq!(events, clean);
    assert!(demuxer.stats().sync_losses > 10);
    assert_eq!(demuxer.stats().continuity_errors, 0);
}

#[test]
fn audio_rate_override_and_mulaw() {
    let units = h264_units();
    let options = StreamOptions {
        audio: Some((STREAM_TYPE_PCMU_TAPO, 640)),
        audio_rate: 16_000,
        ..StreamOptions::tapo(Codec::H264)
    };
    let audio = vec![0xFFu8; 48_000];
    let ts = tapo_stream(&units, &audio, &options);
    let (events, _) = demux_with(TsDemuxer::new().with_audio_rate(16_000), &ts);
    let streams = split(&events);
    assert_eq!(
        streams.audio_configs,
        vec![AudioConfig {
            codec: AudioCodec::PcmMulaw,
            sample_rate: 16_000,
            channels: 1
        }]
    );
    assert!(
        streams
            .audio
            .windows(2)
            .all(|p| p[1].pts - p[0].pts == 3600)
    );

    // Changing the rate mid-stream re-announces the config.
    let mut demuxer = TsDemuxer::new();
    let mut events = Vec::new();
    let (head, tail) = ts.split_at(ts.len() / 2 / 188 * 188);
    demuxer.push(head, &mut events).unwrap();
    demuxer.set_audio_rate(16_000);
    demuxer.push(tail, &mut events).unwrap();
    demuxer.flush(&mut events).unwrap();
    let rates: Vec<u32> = split(&events)
        .audio_configs
        .iter()
        .map(|c| c.sample_rate)
        .collect();
    assert_eq!(rates, vec![8000, 16_000]);
}

#[test]
fn access_units_split_over_and_sharing_pes_packets() {
    let units = h264_units();
    let mut mux = TsMuxer::tapo(Codec::H264.stream_type(), None);
    mux.write_tables();
    let pes = |pts: Option<u64>| PesOptions {
        pts,
        ..PesOptions::default()
    };
    // Frame 0 split into two PES packets, only the first with a PTS.
    let frame0 = units[0].to_annex_b();
    let (a, b) = frame0.split_at(frame0.len() / 2);
    mux.write_pes(TAPO_VIDEO_PID, 0xE0, a, pes(Some(0)));
    mux.write_pes(TAPO_VIDEO_PID, 0xE0, b, pes(None));
    // Frames 1 and 2 in one PES packet.
    let mut both = units[1].to_annex_b();
    both.extend_from_slice(&units[2].to_annex_b());
    mux.write_pes(TAPO_VIDEO_PID, 0xE0, &both, pes(Some(6000)));
    mux.write_pes(
        TAPO_VIDEO_PID,
        0xE0,
        &units[3].to_annex_b(),
        pes(Some(18_000)),
    );
    let frames = split(&demux(&mux.into_bytes())).frames;
    let pts: Vec<i64> = frames.iter().map(|f| f.pts).collect();
    // Frame 2 has no PTS of its own: it continues the frame rate.
    assert_eq!(pts, vec![0, 6000, 12_000, 18_000]);

    let reference = split(&demux(&tapo_stream(
        &units[..4],
        &[],
        &StreamOptions {
            start_pts: 0,
            ..StreamOptions::tapo(Codec::H264)
        },
    )))
    .frames;
    let data =
        |frames: &[VideoFrame]| -> Vec<Bytes> { frames.iter().map(|f| f.data.clone()).collect() };
    assert_eq!(data(&frames), data(&reference));
}
