//! Runs `tapo remux` on the tapo-camera fixtures.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tapo-camera/tests/fixtures")
        .join(name)
}

fn tapo(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tapo"))
        .args(args)
        .output()
        .expect("run tapo")
}

fn remux(input: &Path, output: &Path, extra: &[&str]) -> String {
    let mut args = vec!["remux".as_ref(), input.as_os_str(), output.as_os_str()];
    args.extend(extra.iter().map(std::ffi::OsStr::new));
    let result = tapo(&args);
    assert!(
        result.status.success(),
        "tapo remux failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn remuxes_h264_with_g711() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("clip.mp4");
    let stdout = remux(&fixture("h264_alaw_tapo.mpegts"), &output, &[]);
    assert!(stdout.contains("Video:     H.264 avc1.64"), "{stdout}");
    assert!(stdout.contains("640x360"), "{stdout}");
    assert!(stdout.contains("Frames:    45 (3 keyframes)"), "{stdout}");
    assert!(stdout.contains("Duration:  3.000 s"), "{stdout}");
    assert!(
        stdout.contains("G.711 A-law 8000 Hz mono (not written"),
        "{stdout}"
    );
    let file = std::fs::read(&output).unwrap();
    assert_eq!(&file[4..8], b"ftyp");
}

#[test]
fn remuxes_h265_and_reports_the_audio_rate() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("clip.mp4");
    let stdout = remux(
        &fixture("h265_alaw_tapo.mpegts"),
        &output,
        &["--audio-rate", "16000"],
    );
    assert!(stdout.contains("Video:     H.265 hvc1.1.6.L"), "{stdout}");
    assert!(stdout.contains("Frames:    45"), "{stdout}");
    assert!(stdout.contains("16000 Hz"), "{stdout}");
    assert!(output.exists());
}

#[test]
fn remuxes_aac_audio() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("clip.mp4");
    let stdout = remux(&fixture("h264_aac_ffmpeg.mpegts"), &output, &[]);
    assert!(stdout.contains("480x270"), "{stdout}");
    assert!(stdout.contains("Audio:     AAC 16000 Hz mono"), "{stdout}");

    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-of",
            "json",
            "-show_entries",
            "stream=codec_type,codec_name",
        ])
        .arg(&output)
        .output();
    let Ok(probe) = probe else {
        eprintln!("ffprobe not on PATH: skipping");
        return;
    };
    let json: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    let codecs: Vec<&str> = json["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["codec_name"].as_str().unwrap())
        .collect();
    assert_eq!(codecs, vec!["h264", "aac"]);
}

#[test]
fn rejects_non_ts_input() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("noise.bin");
    std::fs::write(&input, vec![0u8; 2 << 20]).unwrap();
    let output = dir.path().join("out.mp4");
    let result = tapo(&["remux".as_ref(), input.as_os_str(), output.as_os_str()]);
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("does not look like MPEG-TS"), "{stderr}");
}

#[test]
fn reports_missing_input() {
    let result = tapo(&[
        "remux".as_ref(),
        "missing.mpegts".as_ref(),
        "out.mp4".as_ref(),
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("cannot open"));
}
