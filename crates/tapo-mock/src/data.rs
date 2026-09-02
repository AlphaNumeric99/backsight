//! Fixture media and generated recordings.

use jiff::{Timestamp, ToSpan, Zoned, civil::Date, tz::TimeZone};
use serde_json::{Value, json};

/// The media the fake camera streams.
#[derive(Debug)]
pub struct Fixture {
    /// MPEG-TS, looped for live view and playback.
    pub video: Vec<u8>,
    /// Duration of `video` in seconds, for pacing.
    pub video_seconds: f64,
    /// JPEG served as the thumbnail of detection recordings.
    pub thumbnail: Vec<u8>,
}

impl Fixture {
    /// 10 s of 640x360 H.264 test pattern and a thumbnail (see `fixtures/README.md`).
    pub fn builtin() -> Self {
        Self {
            video: include_bytes!("../fixtures/video.mpegts").to_vec(),
            video_seconds: 10.0,
            thumbnail: include_bytes!("../fixtures/thumbnail.jpg").to_vec(),
        }
    }
}

/// The camera's time zone: the machine's.
pub fn zone() -> TimeZone {
    TimeZone::system()
}

/// A tiny deterministic PRNG so every run shows the same recordings.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo) as u64) as i64
    }
}

/// One recording on the fake SD card, in unix seconds (the mock's clock is UTC).
#[derive(Debug, Clone, Copy)]
pub struct Recording {
    pub start: i64,
    pub end: i64,
    /// 1 = continuous ("timed"), 2 = detection.
    pub video_type: i64,
    /// Detection type code (2 motion, 6 person, 8 vehicle, 9 pet); 0 for continuous.
    pub event_type: i64,
}

fn local_midnight(date: Date) -> i64 {
    date.at(0, 0, 0, 0)
        .to_zoned(zone())
        .map(|z| z.timestamp().as_second())
        .unwrap_or(0)
}

/// Recordings of a camera-local day: two hours of continuous recording in the morning,
/// then detection clips through the day. Nothing in the future, nothing older than 14 days.
pub fn recordings_of(date: Date) -> Vec<Recording> {
    let today = Zoned::now().with_time_zone(zone()).date();
    let age_days = (today - date).get_days();
    if !(0..14).contains(&age_days) {
        return Vec::new();
    }
    let midnight = local_midnight(date);
    let mut rng = Rng::new(date.year() as u64 * 400 + date.day_of_year() as u64);
    let mut recordings = Vec::new();

    // 08:00–10:00 continuous, in 10-minute files like real cameras write them.
    for i in 0..12 {
        let start = midnight + 8 * 3600 + i * 600;
        recordings.push(Recording {
            start,
            end: start + 600,
            video_type: 1,
            event_type: 0,
        });
    }
    // Detection clips between 06:00 and 23:30.
    let count = rng.range(20, 60);
    let mut t = midnight + 6 * 3600;
    for _ in 0..count {
        t += rng.range(300, 1800);
        if t > midnight + 23 * 3600 + 1800 {
            break;
        }
        if (8 * 3600..10 * 3600).contains(&(t - midnight)) {
            continue;
        }
        let duration = rng.range(20, 120);
        let event_type = [2, 2, 6, 6, 8, 9][rng.range(0, 6) as usize];
        recordings.push(Recording {
            start: t,
            end: t + duration,
            video_type: 2,
            event_type,
        });
    }
    let now = Timestamp::now().as_second();
    recordings.retain(|r| r.end < now);
    recordings.sort_by_key(|r| r.start);
    recordings
}

/// Camera-local dates between `start` and `end` (`YYYYMMDD`) that have recordings.
pub fn days_with_recordings(start: &str, end: &str) -> Value {
    let parse = |s: &str| Date::strptime("%Y%m%d", s).ok();
    let (Some(mut day), Some(end)) = (parse(start), parse(end)) else {
        return json!([]);
    };
    let mut days = Vec::new();
    while day <= end {
        if !recordings_of(day).is_empty() {
            days.push(json!({ "date": day.strftime("%Y%m%d").to_string() }));
        }
        match day.checked_add(1.day()) {
            Ok(next) => day = next,
            Err(_) => break,
        }
    }
    Value::Array(days)
}

/// All recordings overlapping `[start, end]`.
pub fn recordings_between(start: i64, end: i64) -> Vec<Recording> {
    let (Ok(first), Ok(last)) = (Timestamp::from_second(start), Timestamp::from_second(end)) else {
        return Vec::new();
    };
    let mut day = first.to_zoned(zone()).date();
    let last = last.to_zoned(zone()).date();
    let mut out = Vec::new();
    while day <= last {
        out.extend(
            recordings_of(day)
                .into_iter()
                .filter(|r| r.end > start && r.start < end),
        );
        match day.checked_add(1.day()) {
            Ok(next) => day = next,
            Err(_) => break,
        }
    }
    out
}
