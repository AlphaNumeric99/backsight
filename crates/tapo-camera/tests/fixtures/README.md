# MPEG-TS test fixtures

Short synthetic clips for the media layer tests (`tests/media_pipeline.rs`, the `tapo remux`
tests in `crates/tapo-cli`). Each is 3 seconds of ffmpeg's `testsrc2` pattern at 15 fps, with
no B-frames and a keyframe every 15 frames, so every clip has 45 frames and 3 keyframes.

| File | Layout | Video | Audio |
|---|---|---|---|
| `h264_alaw_tapo.mpegts` | Tapo | H.264 High, 640x360, AUD before every access unit, PTS only, starts at PTS 900000 (10 s) | G.711 A-law, 8 kHz, stream type `0x90` |
| `h265_alaw_tapo.mpegts` | Tapo | H.265 Main, 640x360, no AUDs, PTS and DTS, starts 1.5 s before the 33-bit PTS wrap and crosses it | G.711 A-law, 8 kHz, stream type `0x90` |
| `h264_aac_ffmpeg.mpegts` | ffmpeg | H.264 Main, 480x270 (coded as 480x272 with cropping) | AAC-LC in ADTS, 16 kHz mono, several frames per PES packet |

"Tapo layout" follows what go2rtc documents for Tapo cameras: PMT on PID 18, video on PID 68
(stream id `0xE0`, unbounded PES packets with one access unit each, PCR), audio on PID 69
(stream id `0xC0`, bounded 320-byte PES packets, i.e. 40 ms of 8 kHz G.711), with PAT and PMT
before every keyframe. ffmpeg's muxer cannot write G.711 with Tapo's private stream type, so
those two files are written by the test-only muxer in `tests/support/mod.rs` from elementary
streams that ffmpeg produces. The third file comes straight from ffmpeg's `mpegts` muxer.

## Regenerating

Needs `ffmpeg` with libx264 and libx265 on `PATH` (the fixtures were made with ffmpeg 8.1):

```sh
cargo test -p tapo-camera --test regenerate_fixtures -- --ignored
```

That test (`tests/regenerate_fixtures.rs`) runs these commands in a temporary directory,
then muxes the results:

```sh
# H.264 elementary stream (AUDs let the test muxer split access units)
ffmpeg -f lavfi -i testsrc2=size=640x360:rate=15 -t 3 -an \
  -c:v libx264 -threads 1 -preset veryfast -profile:v high -pix_fmt yuv420p \
  -b:v 300k -maxrate 300k -bufsize 300k \
  -x264-params keyint=15:min-keyint=15:scenecut=0:bframes=0:repeat-headers=1:aud=1 \
  -f h264 video.h264

# H.265 elementary stream
ffmpeg -f lavfi -i testsrc2=size=640x360:rate=15 -t 3 -an \
  -c:v libx265 -preset veryfast -pix_fmt yuv420p -b:v 300k \
  -x265-params keyint=15:min-keyint=15:scenecut=0:bframes=0:repeat-headers=1:aud=1:log-level=error \
  -f hevc video.h265

# 3 s of a 440 Hz tone as raw G.711 A-law, 8 kHz
ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000 -t 3 -c:a pcm_alaw -f alaw audio.alaw

# h264_aac_ffmpeg.mpegts, written directly
ffmpeg -f lavfi -i testsrc2=size=480x270:rate=15 \
  -f lavfi -i sine=frequency=440:sample_rate=16000 -t 3 \
  -c:v libx264 -threads 1 -preset veryfast -profile:v main -pix_fmt yuv420p \
  -b:v 250k -maxrate 250k -bufsize 250k \
  -x264-params keyint=15:min-keyint=15:scenecut=0:bframes=0 \
  -c:a aac -b:a 32k -ac 1 -ar 16000 -f mpegts h264_aac_ffmpeg.mpegts
```

(The test also passes `-hide_banner -loglevel error -y`.) With the same ffmpeg build the
output is byte-identical; x264 runs single-threaded because its threaded encoding is not
reproducible. Other encoder versions change the bytes but not what the tests check: frame
counts, sizes, profiles, timing.

## Checking a fixture

```sh
ffprobe -v error -count_frames -show_entries stream=codec_name,width,height,nb_read_frames,start_time,duration -of compact h264_alaw_tapo.mpegts
```

ffprobe lists the Tapo audio stream as `unknown`, since `0x90` is a private stream type.
