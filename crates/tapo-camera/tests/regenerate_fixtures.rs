//! Regenerates the MPEG-TS fixtures in `tests/fixtures/` (see the README there).
//!
//! Needs `ffmpeg` with libx264 and libx265 on `PATH`:
//!
//! ```text
//! cargo test -p tapo-camera --test regenerate_fixtures -- --ignored
//! ```

mod support;

use std::path::Path;
use std::process::Command;

use support::{
    Codec, STREAM_TYPE_PCMA_TAPO, StreamOptions, WRAP, fixtures_dir, split_access_units,
    tapo_stream,
};

/// Runs ffmpeg with whitespace-separated `args` followed by the output path.
fn ffmpeg(args: &str, output: &Path) {
    let result = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args.split_whitespace())
        .arg(output)
        .output()
        .expect("ffmpeg must be on PATH");
    assert!(
        result.status.success(),
        "ffmpeg {args} failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
#[ignore = "regenerates the committed fixtures; needs ffmpeg"]
fn regenerate_fixtures() {
    let work = tempfile::tempdir().unwrap();
    let work = work.path();
    let out = fixtures_dir();

    // Elementary streams: 3 s of testsrc2, 640x360 at 15 fps, no B-frames, a keyframe
    // every 15 frames with parameter sets repeated, and an AUD before every access unit
    // (the test muxer splits access units on them).
    ffmpeg(
        "-f lavfi -i testsrc2=size=640x360:rate=15 -t 3 -an \
         -c:v libx264 -threads 1 -preset veryfast -profile:v high -pix_fmt yuv420p \
         -b:v 300k -maxrate 300k -bufsize 300k \
         -x264-params keyint=15:min-keyint=15:scenecut=0:bframes=0:repeat-headers=1:aud=1 \
         -f h264",
        &work.join("video.h264"),
    );
    ffmpeg(
        "-f lavfi -i testsrc2=size=640x360:rate=15 -t 3 -an \
         -c:v libx265 -preset veryfast -pix_fmt yuv420p -b:v 300k \
         -x265-params keyint=15:min-keyint=15:scenecut=0:bframes=0:repeat-headers=1:aud=1:\
         log-level=error -f hevc",
        &work.join("video.h265"),
    );
    // 3 s of a 440 Hz tone as raw G.711 A-law at 8 kHz.
    ffmpeg(
        "-f lavfi -i sine=frequency=440:sample_rate=8000 -t 3 -c:a pcm_alaw -f alaw",
        &work.join("audio.alaw"),
    );
    // A plain ffmpeg transport stream with AAC (ADTS) audio, at a size that needs cropping.
    ffmpeg(
        "-f lavfi -i testsrc2=size=480x270:rate=15 \
         -f lavfi -i sine=frequency=440:sample_rate=16000 -t 3 \
         -c:v libx264 -threads 1 -preset veryfast -profile:v main -pix_fmt yuv420p \
         -b:v 250k -maxrate 250k -bufsize 250k \
         -x264-params keyint=15:min-keyint=15:scenecut=0:bframes=0 \
         -c:a aac -b:a 32k -ac 1 -ar 16000 -f mpegts",
        &out.join("h264_aac_ffmpeg.mpegts"),
    );

    let alaw = std::fs::read(work.join("audio.alaw")).unwrap();

    // Tapo layout, H.264 with AUDs, PTS only, starting at 10 s.
    let h264 = std::fs::read(work.join("video.h264")).unwrap();
    let units = split_access_units(&h264, Codec::H264);
    assert_eq!(units.len(), 45);
    let ts = tapo_stream(&units, &alaw, &StreamOptions::tapo(Codec::H264));
    std::fs::write(out.join("h264_alaw_tapo.mpegts"), ts).unwrap();

    // Tapo layout, H.265 without AUDs, PTS and DTS, wrapping the 33-bit clock after 1.5 s.
    let h265 = std::fs::read(work.join("video.h265")).unwrap();
    let units = split_access_units(&h265, Codec::H265);
    assert_eq!(units.len(), 45);
    let options = StreamOptions {
        start_pts: WRAP - 135_000,
        strip_aud: true,
        write_dts: true,
        audio: Some((STREAM_TYPE_PCMA_TAPO, 320)),
        ..StreamOptions::tapo(Codec::H265)
    };
    let ts = tapo_stream(&units, &alaw, &options);
    std::fs::write(out.join("h265_alaw_tapo.mpegts"), ts).unwrap();

    for name in [
        "h264_alaw_tapo.mpegts",
        "h265_alaw_tapo.mpegts",
        "h264_aac_ffmpeg.mpegts",
    ] {
        let size = std::fs::metadata(out.join(name)).unwrap().len();
        assert!(size < 300_000, "{name} is {size} bytes");
        println!("{name}: {size} bytes");
    }
}
