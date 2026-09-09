//! Incremental MPEG-TS demuxer for the camera's media stream.
//!
//! [`TsDemuxer`] takes transport stream bytes in chunks of any size and produces
//! [`MediaEvent`]s in stream order. Output never depends on how the input was chunked.
//!
//! What it handles:
//!
//! - Packet sync: garbage between packets is skipped. Sync is (re)acquired on a `0x47`
//!   byte whose successor packet also starts with `0x47`.
//! - PAT/PMT (sections may span packets). The first video stream (H.264 `0x1B`, H.265
//!   `0x24`) and the first audio stream (AAC `0x0F`, Tapo G.711 A-law `0x90` and µ-law
//!   `0x91`, as named by go2rtc) of the first program are demuxed.
//! - PES reassembly with continuity counter checks. A PES packet that lost data is
//!   dropped; after a loss in the video stream, frames are dropped until the next
//!   keyframe so decoders never see references to missing pictures.
//! - 33-bit PTS/DTS extended to a monotonic `i64` across wraparound (one clock shared by
//!   audio and video, so they stay comparable).
//! - Video access units are assembled from the NAL unit sequence (H.264 7.4.1.2.3,
//!   H.265 7.4.2.4.4): a new access unit starts at an AUD, a parameter set, a prefix SEI
//!   or the first slice of a picture that follows the previous picture's slices. This
//!   works with or without AUDs, with several slices per picture and with access units
//!   split over or sharing PES packets. Each access unit takes the timestamps of the PES
//!   packet in which it starts, as ISO/IEC 13818-1 specifies. An access unit is emitted
//!   once the next one begins (or on [`TsDemuxer::flush`]), so live output lags input by
//!   one frame.
//! - A [`MediaEvent::VideoConfig`] is emitted before the first frame and whenever the
//!   parameter sets change. Frames are only emitted after a config, and the first frame
//!   after a config is a keyframe. Parameter sets, AUDs and filler data are removed from
//!   frames, and NAL units get 4-byte length prefixes.
//! - Audio: G.711 PES payloads become one [`AudioFrame`] each. The TS does not carry
//!   their sample rate, so it is [`DEFAULT_AUDIO_SAMPLE_RATE`] unless set with
//!   [`TsDemuxer::with_audio_rate`]. AAC PES payloads are split into ADTS frames, one
//!   [`AudioFrame`] (ADTS header included) each, with per-frame timestamps.

use std::fmt;

use bytes::Bytes;
use tracing::{debug, warn};

use super::h264::{self, find_start_code, trim_trailing_zeros};
use super::{
    AudioCodec, AudioConfig, AudioFrame, MediaError, MediaEvent, Ticks90k, VideoCodec, VideoConfig,
    VideoFrame, aac, h265,
};

/// Size of a transport stream packet.
pub const PACKET_SIZE: usize = 188;

/// First byte of every transport stream packet.
pub const SYNC_BYTE: u8 = 0x47;

/// Sample rate assumed for G.711 audio unless [`TsDemuxer::with_audio_rate`] says
/// otherwise.
pub const DEFAULT_AUDIO_SAMPLE_RATE: u32 = 8_000;

/// Frequency of the MPEG system clock that PTS and DTS count in.
pub const CLOCK_RATE: u32 = 90_000;

/// PMT `stream_type` values the demuxer understands.
pub mod stream_type {
    /// AAC with ADTS framing (ISO/IEC 13818-7).
    pub const AAC_ADTS: u8 = 0x0F;
    /// H.264 / AVC.
    pub const H264: u8 = 0x1B;
    /// H.265 / HEVC.
    pub const H265: u8 = 0x24;
    /// G.711 A-law as sent by Tapo cameras (go2rtc's `StreamTypePCMATapo`).
    pub const PCMA_TAPO: u8 = 0x90;
    /// G.711 µ-law as sent by Tapo cameras (go2rtc's `StreamTypePCMUTapo`).
    pub const PCMU_TAPO: u8 = 0x91;
}

const PAT_PID: u16 = 0x0000;
const NULL_PID: u16 = 0x1FFF;

/// Report [`MediaError::LostSync`] after skipping this much data without a packet.
const LOST_SYNC_LIMIT: u64 = 1 << 20;
/// Input is appended to the internal buffer at most this much at a time.
const PUSH_CHUNK: usize = 64 * 1024;
/// Access units larger than this are dropped (4K intra frames are a few MiB at most).
const MAX_ACCESS_UNIT_BYTES: usize = 16 << 20;
/// Audio PES packets larger than this are dropped.
const MAX_AUDIO_PES_BYTES: usize = 1 << 20;
/// Leftover bytes of a split ADTS frame kept for the next PES packet.
const MAX_ADTS_CARRY: usize = 8 * 1024;
/// Frame duration used to timestamp frames without PTS before one is known (30 fps).
const DEFAULT_FRAME_TICKS: i64 = 3_000;
/// After this many consecutive continuity errors on a PID its counter is ignored:
/// real losses are isolated, so the sender does not maintain the counter.
const CC_UNRELIABLE_AFTER: u8 = 4;

/// Counters describing what the demuxer has seen, for diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DemuxStats {
    /// Transport stream packets processed.
    pub packets: u64,
    /// Bytes skipped because they were not part of a packet.
    pub skipped_bytes: u64,
    /// Times packet sync was lost.
    pub sync_losses: u64,
    /// Continuity counter errors (lost or corrupt packets) on audio and video PIDs.
    pub continuity_errors: u64,
    /// Video frames emitted.
    pub video_frames: u64,
    /// Video access units dropped: damaged, before the first config or keyframe, or
    /// without timestamp.
    pub dropped_video_frames: u64,
    /// Audio frames emitted.
    pub audio_frames: u64,
    /// Audio PES packets or frames dropped.
    pub dropped_audio_frames: u64,
    /// Parameter sets that could not be parsed.
    pub invalid_parameter_sets: u64,
}

/// Extends 33-bit MPEG timestamps to a continuous `i64` timeline.
///
/// Each value is placed within ±2³² ticks (about 13 hours) of the previous one, so
/// wraparound at 2³³ (about 26.5 hours) is invisible and small backward steps (B-frames,
/// audio interleaved with video) stay backward steps.
#[derive(Debug, Clone, Default)]
pub struct TimestampExtender {
    last: Option<i64>,
}

impl TimestampExtender {
    /// The 33-bit modulus.
    pub const WRAP: i64 = 1 << 33;

    /// Creates an extender; the first value passes through unchanged.
    pub fn new() -> Self {
        Self::default()
    }

    /// Extends a raw 33-bit timestamp (higher bits are ignored).
    pub fn extend(&mut self, raw: u64) -> Ticks90k {
        let raw = (raw & (Self::WRAP as u64 - 1)) as i64;
        let value = match self.last {
            None => raw,
            Some(last) => {
                let mut delta = raw - last.rem_euclid(Self::WRAP);
                if delta > Self::WRAP / 2 {
                    delta -= Self::WRAP;
                } else if delta <= -Self::WRAP / 2 {
                    delta += Self::WRAP;
                }
                last + delta
            }
        };
        self.last = Some(value);
        value
    }
}

/// Incremental MPEG-TS demuxer. See the [module documentation](self) for details.
///
/// ```
/// use tapo_camera::media::{MediaEvent, TsDemuxer};
///
/// let mut demuxer = TsDemuxer::new().with_audio_rate(16_000);
/// let mut events = Vec::new();
/// # let chunks: Vec<Vec<u8>> = Vec::new();
/// for chunk in chunks {
///     demuxer.push(&chunk, &mut events)?;
///     for event in events.drain(..) {
///         match event {
///             MediaEvent::VideoConfig(config) => println!("{}", config.codec_string),
///             MediaEvent::Video(frame) => println!("frame at {}", frame.pts),
///             _ => {}
///         }
///     }
/// }
/// demuxer.flush(&mut events)?;
/// # Ok::<(), tapo_camera::media::MediaError>(())
/// ```
pub struct TsDemuxer {
    buf: Vec<u8>,
    synced: bool,
    desync_run: u64,
    lost_sync_reported: bool,
    audio_rate: u32,
    pat: SectionReader,
    pmt: SectionReader,
    pmt_pid: Option<u16>,
    program_number: Option<u16>,
    last_pmt: Option<Vec<u8>>,
    video: Option<VideoStream>,
    audio: Option<AudioStream>,
    clock: TimestampExtender,
    stats: DemuxStats,
}

impl Default for TsDemuxer {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for TsDemuxer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TsDemuxer")
            .field("synced", &self.synced)
            .field("pmt_pid", &self.pmt_pid)
            .field("video", &self.video.as_ref().map(|v| (v.pid, v.codec)))
            .field("audio", &self.audio.as_ref().map(|a| (a.pid, a.codec)))
            .field("audio_rate", &self.audio_rate)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl TsDemuxer {
    /// Creates a demuxer that assumes [`DEFAULT_AUDIO_SAMPLE_RATE`] for G.711 audio.
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            synced: false,
            desync_run: 0,
            lost_sync_reported: false,
            audio_rate: DEFAULT_AUDIO_SAMPLE_RATE,
            pat: SectionReader::default(),
            pmt: SectionReader::default(),
            pmt_pid: None,
            program_number: None,
            last_pmt: None,
            video: None,
            audio: None,
            clock: TimestampExtender::new(),
            stats: DemuxStats::default(),
        }
    }

    /// Sets the G.711 sample rate, for example from the camera's `getAudioConfig`.
    /// Zero is ignored. AAC streams carry their own rate.
    #[must_use]
    pub fn with_audio_rate(mut self, sample_rate: u32) -> Self {
        self.set_audio_rate(sample_rate);
        self
    }

    /// Changes the G.711 sample rate. A new [`MediaEvent::AudioConfig`] precedes the next
    /// G.711 frame. Zero is ignored.
    pub fn set_audio_rate(&mut self, sample_rate: u32) {
        if sample_rate > 0 {
            self.audio_rate = sample_rate;
        }
    }

    /// The G.711 sample rate in use.
    pub fn audio_rate(&self) -> u32 {
        self.audio_rate
    }

    /// Counters for diagnostics.
    pub fn stats(&self) -> &DemuxStats {
        &self.stats
    }

    /// The video codec announced by the PMT, once one has been seen.
    pub fn video_codec(&self) -> Option<VideoCodec> {
        self.video.as_ref().map(|v| v.codec)
    }

    /// The audio codec announced by the PMT, once one has been seen.
    pub fn audio_codec(&self) -> Option<AudioCodec> {
        self.audio.as_ref().map(|a| a.codec)
    }

    /// Feeds the next chunk of the transport stream, appending resulting events to `out`.
    ///
    /// All of `data` is consumed; incomplete packets are kept for the next call. Damaged
    /// or unexpected data is skipped, never fatal. The only error is
    /// [`MediaError::LostSync`], returned once when a megabyte passes without a single
    /// packet (the input is probably not MPEG-TS); the demuxer stays usable.
    pub fn push(&mut self, data: &[u8], out: &mut Vec<MediaEvent>) -> Result<(), MediaError> {
        for chunk in data.chunks(PUSH_CHUNK) {
            self.buf.extend_from_slice(chunk);
            self.process_buffer(false, out);
        }
        self.sync_error()
    }

    /// Signals the end of the stream: emits the frames still buffered (the last video
    /// access unit, an unterminated audio PES packet) and discards partial packets.
    ///
    /// The demuxer can be fed again afterwards, as if after a discontinuity: stream
    /// assignments, parameter sets and the current configs are kept.
    pub fn flush(&mut self, out: &mut Vec<MediaEvent>) -> Result<(), MediaError> {
        self.process_buffer(true, out);
        self.stats.skipped_bytes += self.buf.len() as u64;
        self.buf.clear();
        self.synced = false;
        let mut sink = Sink {
            clock: &mut self.clock,
            stats: &mut self.stats,
            out,
        };
        if let Some(video) = self.video.as_mut() {
            video.flush(&mut sink);
        }
        if let Some(audio) = self.audio.as_mut() {
            audio.flush(self.audio_rate, &mut sink);
        }
        self.sync_error()
    }

    /// Forgets all state, including buffered data, as if newly created with the same
    /// audio rate.
    pub fn reset(&mut self) {
        *self = Self::new().with_audio_rate(self.audio_rate);
    }

    fn sync_error(&mut self) -> Result<(), MediaError> {
        if !self.synced && self.desync_run >= LOST_SYNC_LIMIT && !self.lost_sync_reported {
            self.lost_sync_reported = true;
            return Err(MediaError::LostSync {
                skipped: self.desync_run,
            });
        }
        Ok(())
    }

    /// Processes the complete packets in `self.buf`, keeping the unprocessed tail.
    fn process_buffer(&mut self, at_eof: bool, out: &mut Vec<MediaEvent>) {
        let mut buf = std::mem::take(&mut self.buf);
        let mut pos = 0;
        while buf.len() - pos >= PACKET_SIZE {
            if buf[pos] == SYNC_BYTE {
                let accept = if self.synced {
                    true
                } else if let Some(&next) = buf.get(pos + PACKET_SIZE) {
                    next == SYNC_BYTE
                } else if at_eof {
                    true
                } else {
                    // Wait for the next packet's first byte to confirm sync.
                    break;
                };
                if accept {
                    self.synced = true;
                    self.desync_run = 0;
                    self.lost_sync_reported = false;
                    self.process_packet(&buf[pos..pos + PACKET_SIZE], out);
                    pos += PACKET_SIZE;
                    continue;
                }
            }
            if self.synced {
                self.synced = false;
                self.stats.sync_losses += 1;
                debug!(offset = pos, "MPEG-TS sync lost");
            }
            let skip = buf[pos + 1..]
                .iter()
                .position(|&b| b == SYNC_BYTE)
                .map_or(buf.len() - pos, |i| i + 1);
            self.stats.skipped_bytes += skip as u64;
            self.desync_run += skip as u64;
            pos += skip;
        }
        buf.drain(..pos);
        self.buf = buf;
    }

    fn process_packet(&mut self, packet: &[u8], out: &mut Vec<MediaEvent>) {
        self.stats.packets += 1;
        if packet[1] & 0x80 != 0 {
            // transport_error_indicator: the packet is corrupt; the continuity check on
            // the next packet of its PID notices the gap.
            return;
        }
        let pusi = packet[1] & 0x40 != 0;
        let pid = (u16::from(packet[1] & 0x1F) << 8) | u16::from(packet[2]);
        if pid == NULL_PID {
            return;
        }
        let scrambled = packet[3] & 0xC0 != 0;
        let adaptation_field_control = (packet[3] >> 4) & 0x03;
        let continuity_counter = packet[3] & 0x0F;
        if adaptation_field_control == 0 {
            return; // reserved
        }
        let has_payload = adaptation_field_control & 0x01 != 0;
        let mut payload_start = 4;
        let mut discontinuity = false;
        if adaptation_field_control & 0x02 != 0 {
            let length = usize::from(packet[4]);
            if 5 + length > PACKET_SIZE {
                return;
            }
            if length > 0 {
                discontinuity = packet[5] & 0x80 != 0;
            }
            payload_start = 5 + length;
        }
        let payload = if has_payload && !scrambled {
            &packet[payload_start..]
        } else {
            &[][..]
        };

        if pid == PAT_PID {
            if has_payload {
                for section in self.pat.push(pusi, payload) {
                    self.handle_pat(&section);
                }
            }
            return;
        }
        if self.pmt_pid == Some(pid) {
            if has_payload {
                for section in self.pmt.push(pusi, payload) {
                    self.handle_pmt(&section, out);
                }
            }
            return;
        }

        let packet_info = PacketInfo {
            packet,
            pusi,
            has_payload,
            continuity_counter,
            discontinuity,
        };
        let mut sink = Sink {
            clock: &mut self.clock,
            stats: &mut self.stats,
            out,
        };
        if let Some(video) = self.video.as_mut()
            && video.pid == pid
        {
            video.handle(&packet_info, payload, &mut sink);
        } else if let Some(audio) = self.audio.as_mut()
            && audio.pid == pid
        {
            audio.handle(&packet_info, payload, self.audio_rate, &mut sink);
        }
    }

    fn handle_pat(&mut self, section: &[u8]) {
        let Some(programs) = parse_pat(section) else {
            return;
        };
        let Some(&(program_number, pmt_pid)) = programs.first() else {
            return;
        };
        if self.pmt_pid != Some(pmt_pid) || self.program_number != Some(program_number) {
            debug!(program_number, pmt_pid, "PAT");
            self.pmt_pid = Some(pmt_pid);
            self.program_number = Some(program_number);
            self.pmt = SectionReader::default();
            self.last_pmt = None;
        }
    }

    fn handle_pmt(&mut self, section: &[u8], out: &mut Vec<MediaEvent>) {
        if self.last_pmt.as_deref() == Some(section) {
            return;
        }
        let Some((program_number, streams)) = parse_pmt(section) else {
            return;
        };
        if self.program_number.is_some_and(|p| p != program_number) {
            return;
        }
        self.last_pmt = Some(section.to_vec());

        let video = streams.iter().find_map(|s| {
            let codec = match s.stream_type {
                stream_type::H264 => VideoCodec::H264,
                stream_type::H265 => VideoCodec::H265,
                _ => return None,
            };
            Some((s.pid, codec))
        });
        let audio = streams.iter().find_map(|s| {
            let codec = match s.stream_type {
                stream_type::AAC_ADTS => AudioCodec::Aac,
                stream_type::PCMA_TAPO => AudioCodec::PcmAlaw,
                stream_type::PCMU_TAPO => AudioCodec::PcmMulaw,
                _ => return None,
            };
            Some((s.pid, codec))
        });
        debug!(?video, ?audio, "PMT");

        let mut sink = Sink {
            clock: &mut self.clock,
            stats: &mut self.stats,
            out,
        };
        let video_unchanged = matches!(
            (&self.video, video),
            (Some(current), Some((pid, codec))) if current.pid == pid && current.codec == codec
        );
        if !video_unchanged {
            if let Some(mut old) = self.video.take() {
                old.flush(&mut sink);
            }
            self.video = video.map(|(pid, codec)| VideoStream::new(pid, codec));
        }
        let audio_unchanged = matches!(
            (&self.audio, audio),
            (Some(current), Some((pid, codec))) if current.pid == pid && current.codec == codec
        );
        if !audio_unchanged {
            if let Some(mut old) = self.audio.take() {
                old.flush(self.audio_rate, &mut sink);
            }
            self.audio = audio.map(|(pid, codec)| AudioStream::new(pid, codec));
        }
    }
}

/// Mutable state shared by the elementary stream handlers.
struct Sink<'a> {
    clock: &'a mut TimestampExtender,
    stats: &'a mut DemuxStats,
    out: &'a mut Vec<MediaEvent>,
}

/// Header fields of one transport stream packet.
struct PacketInfo<'a> {
    packet: &'a [u8],
    pusi: bool,
    has_payload: bool,
    continuity_counter: u8,
    discontinuity: bool,
}

// ---------------------------------------------------------------------------------------
// Continuity counters
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Continuity {
    Ok,
    /// A retransmitted copy of the previous packet; skip it.
    Duplicate,
    /// Packets were lost (or corrupted) before this one.
    Gap,
}

/// Tracks the continuity counter of one PID.
#[derive(Default)]
struct ContinuityCounter {
    last: Option<u8>,
    previous_packet: Option<Box<[u8; PACKET_SIZE]>>,
    consecutive_errors: u8,
    unreliable: bool,
}

impl ContinuityCounter {
    fn check(&mut self, info: &PacketInfo<'_>) -> Continuity {
        if !info.has_payload {
            // The counter only advances on packets with payload.
            return Continuity::Ok;
        }
        let cc = info.continuity_counter;
        let result = match self.last {
            _ if self.unreliable || info.discontinuity => Continuity::Ok,
            None => Continuity::Ok,
            Some(last) if cc == last => {
                if self
                    .previous_packet
                    .as_deref()
                    .is_some_and(|previous| previous[..] == *info.packet)
                {
                    return Continuity::Duplicate;
                }
                Continuity::Gap
            }
            Some(last) if cc == (last + 1) & 0x0F => Continuity::Ok,
            Some(_) => Continuity::Gap,
        };
        self.last = Some(cc);
        let previous = self
            .previous_packet
            .get_or_insert_with(|| Box::new([0; PACKET_SIZE]));
        previous.copy_from_slice(info.packet);
        if result == Continuity::Gap {
            self.consecutive_errors += 1;
            if self.consecutive_errors >= CC_UNRELIABLE_AFTER {
                warn!("continuity counters look unmaintained; ignoring them for this PID");
                self.unreliable = true;
            }
        } else {
            self.consecutive_errors = 0;
        }
        result
    }

    fn reset(&mut self) {
        self.last = None;
        self.consecutive_errors = 0;
    }
}

// ---------------------------------------------------------------------------------------
// PES packets
// ---------------------------------------------------------------------------------------

/// The parts of a PES header the demuxer uses; timestamps are raw 33-bit values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PesHeader {
    packet_length: u16,
    pts: Option<u64>,
    dts: Option<u64>,
}

/// Whether a PES stream id is followed by the optional PES header (13818-1 Table 2-21).
fn has_optional_header(stream_id: u8) -> bool {
    !matches!(
        stream_id,
        0xBC | 0xBE | 0xBF | 0xF0 | 0xF1 | 0xF2 | 0xF8 | 0xFF
    )
}

/// Number of header bytes needed, given the bytes collected so far.
fn pes_header_len(header: &[u8]) -> usize {
    if header.len() < 6 || !has_optional_header(header[3]) {
        6
    } else if header.len() < 9 {
        9
    } else {
        9 + usize::from(header[8])
    }
}

/// Parses a complete PES header (see [`pes_header_len`]). `None` if it is not one.
fn parse_pes_header(header: &[u8]) -> Option<PesHeader> {
    if header.len() < 6 || header[..3] != [0, 0, 1] {
        return None;
    }
    let packet_length = u16::from_be_bytes([header[4], header[5]]);
    let (mut pts, mut dts) = (None, None);
    if has_optional_header(header[3]) {
        let data = header.get(9..)?;
        match header[7] >> 6 {
            0b10 if data.len() >= 5 => pts = Some(read_timestamp(&data[..5])),
            0b11 if data.len() >= 10 => {
                pts = Some(read_timestamp(&data[..5]));
                dts = Some(read_timestamp(&data[5..10]));
            }
            _ => {}
        }
    }
    Some(PesHeader {
        packet_length,
        pts,
        dts,
    })
}

/// Decodes a 5-byte PTS/DTS field (marker bits are not checked).
fn read_timestamp(b: &[u8]) -> u64 {
    (u64::from(b[0] >> 1) & 0x07) << 30
        | u64::from(b[1]) << 22
        | u64::from(b[2] >> 1) << 15
        | u64::from(b[3]) << 7
        | u64::from(b[4] >> 1)
}

enum PesEvent<'a> {
    /// Data was lost before the current packet.
    Loss,
    /// A PES packet began.
    Start(PesHeader),
    /// Payload bytes of the current PES packet.
    Data(&'a [u8]),
    /// The current PES packet ended; `complete` is false if any of it is missing.
    End { complete: bool },
}

/// Reassembles the PES packets of one PID from transport stream payloads.
#[derive(Default)]
struct PesReader {
    active: bool,
    header: Vec<u8>,
    header_done: bool,
    /// Payload bytes still expected when `PES_packet_length` is non-zero.
    remaining: Option<usize>,
    damaged: bool,
}

impl PesReader {
    fn feed(&mut self, pusi: bool, loss: bool, mut data: &[u8], mut on: impl FnMut(PesEvent<'_>)) {
        if loss {
            self.damaged = true;
            on(PesEvent::Loss);
        }
        if pusi {
            if self.active {
                on(PesEvent::End {
                    complete: self.complete(),
                });
            }
            self.start();
        } else if !self.active {
            return;
        }

        if !self.header_done {
            loop {
                let needed = pes_header_len(&self.header);
                if self.header.len() >= needed {
                    break;
                }
                let take = (needed - self.header.len()).min(data.len());
                self.header.extend_from_slice(&data[..take]);
                data = &data[take..];
                if self.header.len() < pes_header_len(&self.header) && data.is_empty() {
                    return;
                }
            }
            let Some(header) = parse_pes_header(&self.header) else {
                // Not a PES packet: ignore everything up to the next unit start.
                self.active = false;
                return;
            };
            self.header_done = true;
            if header.packet_length > 0 {
                let total = 6 + usize::from(header.packet_length);
                let Some(payload) = total.checked_sub(self.header.len()) else {
                    self.active = false;
                    return;
                };
                self.remaining = Some(payload);
            }
            on(PesEvent::Start(header));
        }

        match self.remaining.as_mut() {
            Some(remaining) => {
                let take = (*remaining).min(data.len());
                *remaining -= take;
                if take > 0 {
                    on(PesEvent::Data(&data[..take]));
                }
                if *remaining == 0 {
                    // Anything after the declared length is stuffing.
                    on(PesEvent::End {
                        complete: !self.damaged,
                    });
                    self.active = false;
                }
            }
            None => {
                if !data.is_empty() {
                    on(PesEvent::Data(data));
                }
            }
        }
    }

    fn complete(&self) -> bool {
        !self.damaged && self.header_done && self.remaining.is_none_or(|r| r == 0)
    }

    fn start(&mut self) {
        self.active = true;
        self.header.clear();
        self.header_done = false;
        self.remaining = None;
        self.damaged = false;
    }

    /// Ends the current PES packet at the end of the stream.
    fn finish(&mut self, mut on: impl FnMut(PesEvent<'_>)) {
        if self.active {
            on(PesEvent::End {
                complete: self.complete(),
            });
        }
        self.active = false;
    }
}

// ---------------------------------------------------------------------------------------
// Video
// ---------------------------------------------------------------------------------------

struct VideoStream {
    pid: u16,
    codec: VideoCodec,
    continuity: ContinuityCounter,
    pes: PesReader,
    scanner: AnnexBScanner,
    assembler: AccessUnitAssembler,
}

impl VideoStream {
    fn new(pid: u16, codec: VideoCodec) -> Self {
        Self {
            pid,
            codec,
            continuity: ContinuityCounter::default(),
            pes: PesReader::default(),
            scanner: AnnexBScanner::default(),
            assembler: AccessUnitAssembler::new(codec),
        }
    }

    fn handle(&mut self, info: &PacketInfo<'_>, payload: &[u8], sink: &mut Sink<'_>) {
        let loss = match self.continuity.check(info) {
            Continuity::Duplicate => return,
            Continuity::Gap => {
                sink.stats.continuity_errors += 1;
                debug!(pid = self.pid, "video continuity error");
                true
            }
            Continuity::Ok => false,
        };
        let Self {
            pes,
            scanner,
            assembler,
            ..
        } = self;
        pes.feed(info.pusi, loss, payload, |event| {
            handle_video_event(event, scanner, assembler, sink);
        });
    }

    fn flush(&mut self, sink: &mut Sink<'_>) {
        let Self {
            pes,
            scanner,
            assembler,
            ..
        } = self;
        pes.finish(|event| handle_video_event(event, scanner, assembler, sink));
        scanner.flush(|event| assembler.on_nal(event, sink));
        assembler.finish(sink);
        assembler.pending_ts = None;
        self.continuity.reset();
    }
}

fn handle_video_event(
    event: PesEvent<'_>,
    scanner: &mut AnnexBScanner,
    assembler: &mut AccessUnitAssembler,
    sink: &mut Sink<'_>,
) {
    match event {
        PesEvent::Loss => {
            assembler.mark_damaged();
            scanner.reset();
        }
        PesEvent::Start(header) => {
            assembler.pending_ts = header.pts.map(|pts| {
                let pts = sink.clock.extend(pts);
                let dts = header.dts.map_or(pts, |dts| sink.clock.extend(dts));
                (pts, dts)
            });
        }
        PesEvent::Data(data) => scanner.push(data, |event| assembler.on_nal(event, sink)),
        PesEvent::End { complete } => {
            if !complete {
                assembler.mark_damaged();
            }
        }
    }
}

enum NalEvent<'a> {
    /// A NAL unit began; the slice holds its first bytes (up to [`NAL_PEEK`]), or the
    /// whole unit if it is shorter.
    Start(&'a [u8]),
    /// A NAL unit is complete (trailing zero bytes removed).
    Complete(&'a [u8]),
    /// Buffered data was discarded (runaway NAL unit).
    Discarded,
}

/// Bytes of a NAL unit needed to classify it: the header plus the first slice header bit.
const NAL_PEEK: usize = 3;

/// Splits an Annex B byte stream that arrives in pieces into NAL units, reporting the
/// start of each unit as soon as its first bytes are available.
#[derive(Default)]
struct AnnexBScanner {
    buf: Vec<u8>,
    /// Offset of the current NAL unit's first byte (after its start code).
    nal_start: Option<usize>,
    /// The current NAL unit's start has been reported.
    start_reported: bool,
    /// Offset from which to search for the next start code.
    scan_from: usize,
}

impl AnnexBScanner {
    fn push(&mut self, data: &[u8], mut on: impl FnMut(NalEvent<'_>)) {
        self.buf.extend_from_slice(data);
        loop {
            if let Some(start) = self.nal_start
                && !self.start_reported
                && self.buf.len() >= start + NAL_PEEK
            {
                on(NalEvent::Start(&self.buf[start..start + NAL_PEEK]));
                self.start_reported = true;
            }
            let Some(found) = find_start_code(&self.buf[self.scan_from..]) else {
                // Keep two bytes: they may be the beginning of a start code.
                self.scan_from = self
                    .buf
                    .len()
                    .saturating_sub(2)
                    .max(self.nal_start.unwrap_or(0));
                break;
            };
            let start_code = self.scan_from + found;
            if let Some(start) = self.nal_start {
                self.complete(start, start_code, &mut on);
            }
            self.nal_start = Some(start_code + 3);
            self.start_reported = false;
            self.scan_from = start_code + 3;
        }
        self.compact();
        if self.buf.len() > MAX_ACCESS_UNIT_BYTES {
            self.reset();
            on(NalEvent::Discarded);
        }
    }

    fn complete(&self, start: usize, end: usize, on: &mut impl FnMut(NalEvent<'_>)) {
        let nal = trim_trailing_zeros(&self.buf[start..end]);
        if nal.is_empty() {
            return;
        }
        if !self.start_reported {
            on(NalEvent::Start(nal));
        }
        on(NalEvent::Complete(nal));
    }

    /// Drops bytes that are no longer needed.
    fn compact(&mut self) {
        let keep_from = self.nal_start.unwrap_or(self.scan_from).min(self.scan_from);
        if keep_from > 0 {
            self.buf.drain(..keep_from);
            self.scan_from -= keep_from;
            if let Some(start) = self.nal_start.as_mut() {
                *start -= keep_from;
            }
        }
    }

    /// Completes the last NAL unit at the end of the stream.
    fn flush(&mut self, mut on: impl FnMut(NalEvent<'_>)) {
        if let Some(start) = self.nal_start {
            let end = self.buf.len();
            self.complete(start, end, &mut on);
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.nal_start = None;
        self.start_reported = false;
        self.scan_from = 0;
    }
}

/// How a NAL unit relates to access unit boundaries.
#[derive(Debug, Clone, Copy, Default)]
struct NalClass {
    nal_type: u8,
    /// Slice data of a picture.
    vcl: bool,
    /// A new access unit starts here if the current one already has slice data.
    starts_access_unit: bool,
    /// Slice of an IDR (H.264) or IRAP (H.265) picture.
    keyframe: bool,
}

/// H.265 RASL pictures: leading pictures that reference pictures before their IRAP.
fn is_rasl(nal_type: u8) -> bool {
    matches!(nal_type, 8 | 9)
}

/// H.265 leading pictures (RADL and RASL).
fn is_leading(nal_type: u8) -> bool {
    matches!(nal_type, 6..=9)
}

fn classify(codec: VideoCodec, head: &[u8]) -> NalClass {
    match codec {
        VideoCodec::H264 => {
            let Some(nal_type) = h264::nal_unit_type(head) else {
                return NalClass::default();
            };
            match nal_type {
                1..=5 => NalClass {
                    nal_type,
                    vcl: true,
                    // first_mb_in_slice == 0 is coded as a single 1 bit.
                    starts_access_unit: head.get(1).is_some_and(|b| b & 0x80 != 0),
                    keyframe: nal_type == h264::nal_type::IDR_SLICE,
                },
                6..=9 | 14..=18 => NalClass {
                    nal_type,
                    starts_access_unit: true,
                    ..NalClass::default()
                },
                _ => NalClass {
                    nal_type,
                    ..NalClass::default()
                },
            }
        }
        VideoCodec::H265 => {
            let (Some(nal_type), Some(layer)) =
                (h265::nal_unit_type(head), h265::nuh_layer_id(head))
            else {
                return NalClass::default();
            };
            if layer != 0 {
                // Enhancement layers belong to the base layer's access unit.
                return NalClass::default();
            }
            match nal_type {
                0..=31 => NalClass {
                    nal_type,
                    vcl: true,
                    // first_slice_segment_in_pic_flag
                    starts_access_unit: head.get(2).is_some_and(|b| b & 0x80 != 0),
                    keyframe: h265::is_irap(nal_type),
                },
                32..=35 | 39 | 41..=44 | 48..=55 => NalClass {
                    nal_type,
                    starts_access_unit: true,
                    ..NalClass::default()
                },
                _ => NalClass {
                    nal_type,
                    ..NalClass::default()
                },
            }
        }
    }
}

/// Parameter sets by id, most recently changed first.
#[derive(Default)]
struct ParameterSets {
    vps: Vec<(u32, Bytes)>,
    sps: Vec<(u32, Bytes)>,
    pps: Vec<(u32, Bytes)>,
}

impl ParameterSets {
    /// Stores a parameter set; returns whether anything changed.
    fn upsert(list: &mut Vec<(u32, Bytes)>, id: u32, nal: &[u8]) -> bool {
        match list.iter().position(|(existing, _)| *existing == id) {
            Some(i) if list[i].1.as_ref() == nal => false,
            found => {
                if let Some(i) = found {
                    list.remove(i);
                }
                list.insert(0, (id, Bytes::copy_from_slice(nal)));
                true
            }
        }
    }

    fn build_config(&self, codec: VideoCodec) -> Result<Option<VideoConfig>, MediaError> {
        let sps: Vec<&[u8]> = self.sps.iter().map(|(_, nal)| nal.as_ref()).collect();
        let pps: Vec<&[u8]> = self.pps.iter().map(|(_, nal)| nal.as_ref()).collect();
        if sps.is_empty() || pps.is_empty() {
            return Ok(None);
        }
        match codec {
            VideoCodec::H264 => h264::video_config(&sps, &pps).map(Some),
            VideoCodec::H265 => {
                let vps: Vec<&[u8]> = self.vps.iter().map(|(_, nal)| nal.as_ref()).collect();
                if vps.is_empty() {
                    return Ok(None);
                }
                h265::video_config(&vps, &sps, &pps).map(Some)
            }
        }
    }
}

/// Groups NAL units into access units and emits frames and configs.
struct AccessUnitAssembler {
    codec: VideoCodec,
    /// Length-prefixed NAL units of the current access unit.
    data: Vec<u8>,
    started: bool,
    has_vcl: bool,
    keyframe: bool,
    damaged: bool,
    /// `nal_unit_type` of the first slice of the current access unit.
    picture_type: Option<u8>,
    /// (PTS, DTS) of the current access unit.
    ts: Option<(Ticks90k, Ticks90k)>,
    /// Timestamps of the latest PES header, for the next access unit that starts.
    pending_ts: Option<(Ticks90k, Ticks90k)>,
    params: ParameterSets,
    params_changed: bool,
    config: Option<VideoConfig>,
    need_keyframe: bool,
    /// Decoding (re)started at an H.265 CRA or BLA picture: its RASL pictures reference
    /// pictures that were never received and are dropped (H.265 8.1.3).
    skip_rasl: bool,
    last_dts: Option<Ticks90k>,
    frame_ticks: Option<i64>,
}

impl AccessUnitAssembler {
    fn new(codec: VideoCodec) -> Self {
        Self {
            codec,
            data: Vec::new(),
            started: false,
            has_vcl: false,
            keyframe: false,
            damaged: false,
            picture_type: None,
            ts: None,
            pending_ts: None,
            params: ParameterSets::default(),
            params_changed: false,
            config: None,
            need_keyframe: true,
            skip_rasl: false,
            last_dts: None,
            frame_ticks: None,
        }
    }

    fn mark_damaged(&mut self) {
        if self.started {
            self.damaged = true;
        }
        self.need_keyframe = true;
    }

    fn on_nal(&mut self, event: NalEvent<'_>, sink: &mut Sink<'_>) {
        match event {
            NalEvent::Start(head) => self.on_nal_start(head, sink),
            NalEvent::Complete(nal) => self.on_nal_complete(nal, sink),
            NalEvent::Discarded => self.mark_damaged(),
        }
    }

    fn on_nal_start(&mut self, head: &[u8], sink: &mut Sink<'_>) {
        let class = classify(self.codec, head);
        if self.has_vcl && class.starts_access_unit {
            self.finish(sink);
        }
        if !self.started {
            self.started = true;
            self.ts = self.pending_ts.take();
        }
        if class.vcl {
            if !self.has_vcl
                && let Some(ts) = self.pending_ts.take()
            {
                // A PES packet began between the access unit's first NAL unit and its
                // first slice: its timestamps belong to this picture.
                self.ts = Some(ts);
            }
            self.picture_type.get_or_insert(class.nal_type);
            self.has_vcl = true;
            self.keyframe |= class.keyframe;
        }
    }

    fn on_nal_complete(&mut self, nal: &[u8], sink: &mut Sink<'_>) {
        let stored = match self.codec {
            VideoCodec::H264 => match h264::nal_unit_type(nal) {
                Some(h264::nal_type::SPS) => self.store_parameter_set(nal, sink),
                Some(h264::nal_type::PPS) => self.store_parameter_set(nal, sink),
                Some(
                    h264::nal_type::AUD
                    | h264::nal_type::FILLER_DATA
                    | h264::nal_type::SPS_EXTENSION,
                ) => true,
                _ => false,
            },
            VideoCodec::H265 => match h265::nal_unit_type(nal) {
                Some(h265::nal_type::VPS | h265::nal_type::SPS | h265::nal_type::PPS) => {
                    self.store_parameter_set(nal, sink)
                }
                Some(h265::nal_type::AUD | h265::nal_type::FILLER_DATA) => true,
                _ => false,
            },
        };
        if stored {
            return;
        }
        if self.data.len() + 4 + nal.len() > MAX_ACCESS_UNIT_BYTES {
            self.damaged = true;
            return;
        }
        // Bounded by MAX_ACCESS_UNIT_BYTES.
        self.data
            .extend_from_slice(&(nal.len() as u32).to_be_bytes());
        self.data.extend_from_slice(nal);
    }

    /// Records a parameter set; always returns true (it is consumed, not appended).
    fn store_parameter_set(&mut self, nal: &[u8], sink: &mut Sink<'_>) -> bool {
        let stored = match self.codec {
            VideoCodec::H264 => match h264::nal_unit_type(nal) {
                Some(h264::nal_type::SPS) => h264::Sps::parse(nal).map(|sps| {
                    ParameterSets::upsert(&mut self.params.sps, sps.seq_parameter_set_id, nal)
                }),
                _ => h264::Pps::parse(nal).map(|pps| {
                    ParameterSets::upsert(&mut self.params.pps, pps.pic_parameter_set_id, nal)
                }),
            },
            VideoCodec::H265 => match h265::nal_unit_type(nal) {
                Some(h265::nal_type::VPS) => h265::vps_id(nal)
                    .map(|id| ParameterSets::upsert(&mut self.params.vps, u32::from(id), nal)),
                Some(h265::nal_type::SPS) => h265::Sps::parse(nal).map(|sps| {
                    ParameterSets::upsert(&mut self.params.sps, sps.seq_parameter_set_id, nal)
                }),
                _ => h265::Pps::parse(nal).map(|pps| {
                    ParameterSets::upsert(&mut self.params.pps, pps.pic_parameter_set_id, nal)
                }),
            },
        };
        match stored {
            Ok(changed) => self.params_changed |= changed,
            Err(err) => {
                sink.stats.invalid_parameter_sets += 1;
                warn!(%err, "ignoring unparsable parameter set");
            }
        }
        true
    }

    fn refresh_config(&mut self, sink: &mut Sink<'_>) {
        self.params_changed = false;
        match self.params.build_config(self.codec) {
            Ok(Some(config)) => {
                if self.config.as_ref() != Some(&config) {
                    debug!(
                        codec = %config.codec_string,
                        width = config.width,
                        height = config.height,
                        "video config"
                    );
                    sink.out.push(MediaEvent::VideoConfig(config.clone()));
                    self.config = Some(config);
                    self.need_keyframe = true;
                }
            }
            Ok(None) => {}
            Err(err) => {
                sink.stats.invalid_parameter_sets += 1;
                warn!(%err, "ignoring unusable video parameter sets");
            }
        }
    }

    /// Completes the current access unit and emits it if it is usable.
    fn finish(&mut self, sink: &mut Sink<'_>) {
        if !self.started {
            return;
        }
        let data = std::mem::take(&mut self.data);
        let has_vcl = std::mem::take(&mut self.has_vcl);
        let keyframe = std::mem::take(&mut self.keyframe);
        let damaged = std::mem::take(&mut self.damaged);
        let picture_type = self.picture_type.take();
        let ts = self.ts.take();
        self.started = false;
        if !has_vcl {
            return;
        }
        let h265_picture = picture_type.filter(|_| self.codec == VideoCodec::H265);
        if let Some((_, dts)) = ts {
            if let Some(last) = self.last_dts {
                let delta = dts - last;
                if delta > 0 && delta <= i64::from(CLOCK_RATE) {
                    self.frame_ticks = Some(delta);
                }
            }
            self.last_dts = Some(dts);
        }

        let drop_reason = if damaged {
            self.need_keyframe = true;
            Some("damaged")
        } else {
            if self.params_changed || self.config.is_none() {
                self.refresh_config(sink);
            }
            if self.config.is_none() {
                Some("no parameter sets yet")
            } else if self.need_keyframe && !keyframe {
                Some("waiting for a keyframe")
            } else if self.skip_rasl && h265_picture.is_some_and(is_rasl) {
                Some("RASL picture of the first CRA")
            } else {
                None
            }
        };
        let ts = ts.or_else(|| {
            let dts = self.last_dts? + self.frame_ticks.unwrap_or(DEFAULT_FRAME_TICKS);
            self.last_dts = Some(dts);
            Some((dts, dts))
        });
        let drop_reason = drop_reason.or(ts.is_none().then_some("no timestamp"));
        if let Some(reason) = drop_reason {
            sink.stats.dropped_video_frames += 1;
            debug!(reason, "dropping video access unit");
            return;
        }
        let Some((pts, dts)) = ts else {
            return;
        };
        if self.need_keyframe {
            self.skip_rasl = h265_picture
                .is_some_and(|t| matches!(t, h265::nal_type::CRA_NUT | h265::nal_type::BLA_W_LP));
        } else if h265_picture.is_none_or(|t| !is_leading(t)) {
            // Leading pictures precede all trailing pictures in decoding order.
            self.skip_rasl = false;
        }
        self.need_keyframe = false;
        sink.stats.video_frames += 1;
        sink.out.push(MediaEvent::Video(VideoFrame {
            pts,
            dts,
            keyframe,
            data: Bytes::from(data),
        }));
    }
}

// ---------------------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------------------

/// Position on an audio timeline: a base PTS plus samples since, to timestamp frames
/// without their own PTS without accumulating rounding errors.
#[derive(Debug, Clone, Copy)]
struct AudioClock {
    base: Ticks90k,
    samples: u64,
    rate: u32,
}

impl AudioClock {
    fn now(&self) -> Ticks90k {
        // Lossless for any realistic stream length.
        self.base + (self.samples * u64::from(CLOCK_RATE) / u64::from(self.rate)) as i64
    }
}

struct AudioStream {
    pid: u16,
    codec: AudioCodec,
    continuity: ContinuityCounter,
    pes: PesReader,
    payload: Vec<u8>,
    overflow: bool,
    pes_pts: Option<Ticks90k>,
    clock: Option<AudioClock>,
    config: Option<AudioConfig>,
    /// Start of an ADTS frame that continues in the next PES packet.
    adts_carry: Vec<u8>,
}

impl AudioStream {
    fn new(pid: u16, codec: AudioCodec) -> Self {
        Self {
            pid,
            codec,
            continuity: ContinuityCounter::default(),
            pes: PesReader::default(),
            payload: Vec::new(),
            overflow: false,
            pes_pts: None,
            clock: None,
            config: None,
            adts_carry: Vec::new(),
        }
    }

    fn handle(
        &mut self,
        info: &PacketInfo<'_>,
        payload: &[u8],
        audio_rate: u32,
        sink: &mut Sink<'_>,
    ) {
        let loss = match self.continuity.check(info) {
            Continuity::Duplicate => return,
            Continuity::Gap => {
                sink.stats.continuity_errors += 1;
                debug!(pid = self.pid, "audio continuity error");
                true
            }
            Continuity::Ok => false,
        };
        let mut pes = std::mem::take(&mut self.pes);
        pes.feed(info.pusi, loss, payload, |event| {
            self.on_pes_event(event, audio_rate, sink);
        });
        self.pes = pes;
    }

    fn flush(&mut self, audio_rate: u32, sink: &mut Sink<'_>) {
        let mut pes = std::mem::take(&mut self.pes);
        pes.finish(|event| self.on_pes_event(event, audio_rate, sink));
        self.pes = pes;
        self.adts_carry.clear();
        self.continuity.reset();
    }

    fn on_pes_event(&mut self, event: PesEvent<'_>, audio_rate: u32, sink: &mut Sink<'_>) {
        match event {
            PesEvent::Loss => self.adts_carry.clear(),
            PesEvent::Start(header) => {
                self.payload.clear();
                self.overflow = false;
                self.pes_pts = header.pts.map(|pts| sink.clock.extend(pts));
            }
            PesEvent::Data(data) => {
                if self.payload.len() + data.len() > MAX_AUDIO_PES_BYTES {
                    self.overflow = true;
                } else {
                    self.payload.extend_from_slice(data);
                }
            }
            PesEvent::End { complete } => {
                if complete && !self.overflow {
                    match self.codec {
                        AudioCodec::PcmAlaw | AudioCodec::PcmMulaw => {
                            self.emit_g711(audio_rate, sink);
                        }
                        AudioCodec::Aac => self.emit_aac(sink),
                    }
                } else {
                    sink.stats.dropped_audio_frames += 1;
                    self.adts_carry.clear();
                }
                self.payload.clear();
                self.pes_pts = None;
            }
        }
    }

    fn push_config(&mut self, config: AudioConfig, sink: &mut Sink<'_>) {
        if self.config != Some(config) {
            debug!(?config, "audio config");
            sink.out.push(MediaEvent::AudioConfig(config));
            self.config = Some(config);
        }
    }

    /// The timestamp for a frame: the PES timestamp if this frame starts the packet,
    /// otherwise the running audio clock.
    fn frame_pts(&mut self, pes_pts: Option<Ticks90k>, rate: u32) -> Option<Ticks90k> {
        match (pes_pts, self.clock) {
            (Some(base), _) => {
                self.clock = Some(AudioClock {
                    base,
                    samples: 0,
                    rate,
                });
            }
            (None, Some(clock)) if clock.rate != rate => {
                self.clock = Some(AudioClock {
                    base: clock.now(),
                    samples: 0,
                    rate,
                });
            }
            _ => {}
        }
        self.clock.map(|clock| clock.now())
    }

    fn emit_g711(&mut self, rate: u32, sink: &mut Sink<'_>) {
        if self.payload.is_empty() {
            return;
        }
        let codec = self.codec;
        self.push_config(
            AudioConfig {
                codec,
                sample_rate: rate,
                channels: 1,
            },
            sink,
        );
        let Some(pts) = self.frame_pts(self.pes_pts, rate) else {
            sink.stats.dropped_audio_frames += 1;
            return;
        };
        if let Some(clock) = self.clock.as_mut() {
            clock.samples += self.payload.len() as u64;
        }
        sink.stats.audio_frames += 1;
        sink.out.push(MediaEvent::Audio(AudioFrame {
            pts,
            data: Bytes::from(std::mem::take(&mut self.payload)),
        }));
    }

    fn emit_aac(&mut self, sink: &mut Sink<'_>) {
        let mut data = std::mem::take(&mut self.adts_carry);
        let carried = data.len();
        data.extend_from_slice(&self.payload);
        let mut pes_pts = self.pes_pts;

        let mut frames = aac::adts_frames(&data);
        while let Some(frame) = frames.next() {
            let start = frames.offset() - frame.data.len();
            let header = frame.header;
            // The PES timestamp belongs to the first frame that starts in this packet.
            let pts = if start >= carried {
                self.frame_pts(pes_pts.take(), header.sample_rate)
            } else {
                self.frame_pts(None, header.sample_rate)
            };
            self.push_config(
                AudioConfig {
                    codec: AudioCodec::Aac,
                    sample_rate: header.sample_rate,
                    channels: aac::channel_count(header.channel_configuration),
                },
                sink,
            );
            let Some(pts) = pts else {
                sink.stats.dropped_audio_frames += 1;
                continue;
            };
            if let Some(clock) = self.clock.as_mut() {
                clock.samples += u64::from(header.samples());
            }
            sink.stats.audio_frames += 1;
            sink.out.push(MediaEvent::Audio(AudioFrame {
                pts,
                data: Bytes::copy_from_slice(frame.data),
            }));
        }
        let consumed = frames.offset();
        if data.len() - consumed <= MAX_ADTS_CARRY {
            data.drain(..consumed);
            self.adts_carry = data;
        }
    }
}

// ---------------------------------------------------------------------------------------
// PSI
// ---------------------------------------------------------------------------------------

/// Reassembles PSI sections of one PID.
#[derive(Default)]
struct SectionReader {
    buf: Vec<u8>,
    active: bool,
}

impl SectionReader {
    /// Feeds a packet payload; returns the sections it completes.
    fn push(&mut self, pusi: bool, payload: &[u8]) -> Vec<Vec<u8>> {
        let mut sections = Vec::new();
        let mut data = payload;
        if pusi {
            let Some((&pointer, rest)) = data.split_first() else {
                return sections;
            };
            let pointer = usize::from(pointer);
            if pointer > rest.len() {
                self.active = false;
                self.buf.clear();
                return sections;
            }
            if self.active {
                self.buf.extend_from_slice(&rest[..pointer]);
                self.take_sections(&mut sections);
            }
            self.buf.clear();
            self.active = true;
            data = &rest[pointer..];
        } else if !self.active {
            return sections;
        }
        self.buf.extend_from_slice(data);
        self.take_sections(&mut sections);
        sections
    }

    fn take_sections(&mut self, sections: &mut Vec<Vec<u8>>) {
        while self.active {
            match self.buf.first() {
                None => return,
                // Stuffing: no more sections in this unit.
                Some(0xFF) => {
                    self.active = false;
                    self.buf.clear();
                    return;
                }
                Some(_) => {}
            }
            if self.buf.len() < 3 {
                return;
            }
            let length = 3 + ((usize::from(self.buf[1] & 0x0F) << 8) | usize::from(self.buf[2]));
            if length > 4096 {
                self.active = false;
                self.buf.clear();
                return;
            }
            if self.buf.len() < length {
                return;
            }
            sections.push(self.buf.drain(..length).collect());
        }
    }
}

/// CRC-32/MPEG-2 as used by PSI sections: a valid section, CRC included, yields 0.
pub(crate) fn crc32_mpeg2(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Checks the common long-form section header; returns the section without its CRC.
fn section_body(section: &[u8], table_id: u8, min_len: usize) -> Option<&[u8]> {
    if section.len() < min_len || section[0] != table_id || section[1] & 0x80 == 0 {
        return None;
    }
    if section[5] & 0x01 == 0 {
        return None; // current_next_indicator: not applicable yet
    }
    if crc32_mpeg2(section) != 0 {
        // Accepted anyway: a lenient reader beats losing the stream to a firmware quirk.
        debug!(table_id, "PSI section CRC mismatch");
    }
    Some(&section[..section.len() - 4])
}

/// Parses a PAT section into (program_number, PMT PID) pairs, skipping the network PID.
fn parse_pat(section: &[u8]) -> Option<Vec<(u16, u16)>> {
    let body = section_body(section, 0x00, 12)?;
    let (entries, _) = body[8..].as_chunks::<4>();
    Some(
        entries
            .iter()
            .filter_map(|&[p0, p1, pid0, pid1]| {
                let program_number = u16::from_be_bytes([p0, p1]);
                let pid = u16::from_be_bytes([pid0, pid1]) & 0x1FFF;
                (program_number != 0).then_some((program_number, pid))
            })
            .collect(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PmtStream {
    stream_type: u8,
    pid: u16,
}

/// Parses a PMT section into its program number and elementary streams.
fn parse_pmt(section: &[u8]) -> Option<(u16, Vec<PmtStream>)> {
    let body = section_body(section, 0x02, 16)?;
    let program_number = u16::from_be_bytes([body[3], body[4]]);
    let program_info_length = usize::from(u16::from_be_bytes([body[10], body[11]]) & 0x0FFF);
    let mut pos = 12 + program_info_length;
    let mut streams = Vec::new();
    while pos + 5 <= body.len() {
        let stream_type = body[pos];
        let pid = u16::from_be_bytes([body[pos + 1], body[pos + 2]]) & 0x1FFF;
        let es_info_length =
            usize::from(u16::from_be_bytes([body[pos + 3], body[pos + 4]]) & 0x0FFF);
        streams.push(PmtStream { stream_type, pid });
        pos += 5 + es_info_length;
    }
    Some((program_number, streams))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_reference() {
        assert_eq!(crc32_mpeg2(b"123456789"), 0x0376_E6E7);
        let mut data = b"123456789".to_vec();
        data.extend_from_slice(&0x0376_E6E7u32.to_be_bytes());
        assert_eq!(crc32_mpeg2(&data), 0);
    }

    #[test]
    fn extends_timestamps_across_wraparound() {
        let wrap = TimestampExtender::WRAP as u64;
        let mut clock = TimestampExtender::new();
        assert_eq!(clock.extend(wrap - 3000), (wrap - 3000) as i64);
        assert_eq!(clock.extend(wrap - 10), (wrap - 10) as i64);
        assert_eq!(clock.extend(2990), (wrap + 2990) as i64);
        // A small step backwards (e.g. audio slightly behind video) across the wrap.
        assert_eq!(clock.extend(wrap - 5), (wrap - 5) as i64);
        assert_eq!(clock.extend(5990), (wrap + 5990) as i64);
        // Second wrap.
        for step in 1..=4u64 {
            let value = clock.extend((step * wrap / 4 + 5990) % wrap);
            assert_eq!(value, (wrap + step * wrap / 4 + 5990) as i64);
        }
        assert!(clock.extend(0) > 2 * wrap as i64 - 1);
    }

    #[test]
    fn extender_goes_negative_before_zero() {
        let mut clock = TimestampExtender::new();
        assert_eq!(clock.extend(100), 100);
        let wrap = TimestampExtender::WRAP as u64;
        assert_eq!(clock.extend(wrap - 100), -100);
    }

    fn pes_header_bytes(pts: Option<u64>, dts: Option<u64>, length: u16) -> Vec<u8> {
        fn ts(prefix: u8, t: u64) -> [u8; 5] {
            [
                (prefix << 4) | (((t >> 30) as u8 & 0x07) << 1) | 1,
                (t >> 22) as u8,
                (((t >> 15) as u8) << 1) | 1,
                (t >> 7) as u8,
                ((t as u8) << 1) | 1,
            ]
        }
        let mut h = vec![0, 0, 1, 0xE0];
        h.extend_from_slice(&length.to_be_bytes());
        let flags = match (pts, dts) {
            (Some(_), Some(_)) => 0xC0,
            (Some(_), None) => 0x80,
            _ => 0,
        };
        let mut extra = Vec::new();
        if let Some(pts) = pts {
            extra.extend_from_slice(&ts(if dts.is_some() { 3 } else { 2 }, pts));
        }
        if let Some(dts) = dts {
            extra.extend_from_slice(&ts(1, dts));
        }
        h.extend_from_slice(&[0x80, flags, extra.len() as u8]);
        h.extend_from_slice(&extra);
        h
    }

    #[test]
    fn parses_pes_headers() {
        let max = (1u64 << 33) - 1;
        let h = pes_header_bytes(Some(max), Some(12_345), 0);
        assert_eq!(pes_header_len(&h), h.len());
        let parsed = parse_pes_header(&h).unwrap();
        assert_eq!(parsed.pts, Some(max));
        assert_eq!(parsed.dts, Some(12_345));
        assert_eq!(parsed.packet_length, 0);

        let h = pes_header_bytes(Some(900_000), None, 100);
        let parsed = parse_pes_header(&h).unwrap();
        assert_eq!(
            (parsed.pts, parsed.dts, parsed.packet_length),
            (Some(900_000), None, 100)
        );

        assert!(parse_pes_header(&[0, 0, 2, 0xE0, 0, 0]).is_none());
        // Padding stream: no optional header.
        assert_eq!(pes_header_len(&[0, 0, 1, 0xBE, 0, 4]), 6);
    }

    #[test]
    fn pes_reader_handles_split_headers_and_lengths() {
        let mut payload = pes_header_bytes(Some(90_000), None, 0);
        let header_len = payload.len();
        payload.extend_from_slice(b"abcdefgh");
        let bounded_len = (payload.len() - 6) as u16;
        payload[4..6].copy_from_slice(&bounded_len.to_be_bytes());

        for split in 1..payload.len() {
            let mut reader = PesReader::default();
            let mut data = Vec::new();
            let mut starts = 0;
            let mut ends = Vec::new();
            let mut on = |event: PesEvent<'_>| match event {
                PesEvent::Start(h) => {
                    starts += 1;
                    assert_eq!(h.pts, Some(90_000));
                }
                PesEvent::Data(d) => data.extend_from_slice(d),
                PesEvent::End { complete } => ends.push(complete),
                PesEvent::Loss => panic!("no loss expected"),
            };
            reader.feed(true, false, &payload[..split], &mut on);
            reader.feed(false, false, &payload[split..], &mut on);
            // Stuffing after the declared length is ignored.
            reader.feed(false, false, b"zz", &mut on);
            assert_eq!(starts, 1, "split at {split}");
            assert_eq!(data, b"abcdefgh", "split at {split}");
            assert_eq!(ends, vec![true], "split at {split} (header {header_len})");
        }
    }

    #[test]
    fn pes_reader_reports_incomplete_packets() {
        let mut payload = pes_header_bytes(Some(90_000), None, 0);
        payload.extend_from_slice(b"abc");
        let declared = (payload.len() - 6 + 10) as u16; // 10 bytes never arrive
        payload[4..6].copy_from_slice(&declared.to_be_bytes());
        let mut ends = Vec::new();
        let mut reader = PesReader::default();
        reader.feed(true, false, &payload, |e| {
            if let PesEvent::End { complete } = e {
                ends.push(complete);
            }
        });
        reader.feed(true, false, &payload, |e| {
            if let PesEvent::End { complete } = e {
                ends.push(complete);
            }
        });
        assert_eq!(ends, vec![false]);
        reader.finish(|e| {
            if let PesEvent::End { complete } = e {
                ends.push(complete);
            }
        });
        assert_eq!(ends, vec![false, false]);
    }

    fn packet(pid: u16, cc: u8, fill: u8) -> [u8; PACKET_SIZE] {
        let mut p = [fill; PACKET_SIZE];
        p[0] = SYNC_BYTE;
        p[1] = (pid >> 8) as u8;
        p[2] = pid as u8;
        p[3] = 0x10 | cc;
        p
    }

    fn info(packet: &[u8]) -> PacketInfo<'_> {
        PacketInfo {
            packet,
            pusi: false,
            has_payload: true,
            continuity_counter: packet[3] & 0x0F,
            discontinuity: false,
        }
    }

    #[test]
    fn continuity_counter_detects_gaps_and_duplicates() {
        let mut cc = ContinuityCounter::default();
        let p0 = packet(68, 15, 1);
        let p1 = packet(68, 0, 2);
        assert_eq!(cc.check(&info(&p0)), Continuity::Ok);
        assert_eq!(cc.check(&info(&p1)), Continuity::Ok);
        assert_eq!(cc.check(&info(&p1)), Continuity::Duplicate);
        let p3 = packet(68, 2, 3);
        assert_eq!(cc.check(&info(&p3)), Continuity::Gap);
        let p4 = packet(68, 3, 4);
        assert_eq!(cc.check(&info(&p4)), Continuity::Ok);
        // Same counter, different content: not a duplicate.
        let p4b = packet(68, 3, 5);
        assert_eq!(cc.check(&info(&p4b)), Continuity::Gap);
        // Discontinuity indicator resets expectations.
        let p9 = packet(68, 9, 6);
        let mut i = info(&p9);
        i.discontinuity = true;
        assert_eq!(cc.check(&i), Continuity::Ok);
        // Adaptation-only packets do not advance the counter.
        let af = packet(68, 9, 7);
        let mut i = info(&af);
        i.has_payload = false;
        assert_eq!(cc.check(&i), Continuity::Ok);
        assert_eq!(cc.check(&info(&packet(68, 10, 8))), Continuity::Ok);
    }

    #[test]
    fn continuity_counter_gives_up_on_unmaintained_counters() {
        let mut cc = ContinuityCounter::default();
        let mut gaps = 0;
        for fill in 0..20u8 {
            if cc.check(&info(&packet(68, 0, fill))) == Continuity::Gap {
                gaps += 1;
            }
        }
        assert_eq!(gaps, u32::from(CC_UNRELIABLE_AFTER));
    }

    #[test]
    fn section_reader_reassembles_split_sections() {
        let section: Vec<u8> = {
            let mut s = vec![0x02, 0xB0, 0x00, 0, 1, 0xC1, 0, 0];
            s.extend(std::iter::repeat_n(0xAA, 300));
            let len = (s.len() - 3 + 4) as u16;
            s[1] = 0xB0 | (len >> 8) as u8;
            s[2] = len as u8;
            let crc = crc32_mpeg2(&s);
            s.extend_from_slice(&crc.to_be_bytes());
            s
        };
        let mut reader = SectionReader::default();
        let mut first = vec![0u8]; // pointer_field
        first.extend_from_slice(&section[..150]);
        assert!(reader.push(true, &first).is_empty());
        assert!(reader.push(false, &section[150..300]).is_empty());
        let mut last = section[300..].to_vec();
        last.extend(std::iter::repeat_n(0xFF, 20));
        let sections = reader.push(false, &last);
        assert_eq!(sections, vec![section.clone()]);

        // Two sections in one payload, then stuffing.
        let mut reader = SectionReader::default();
        let mut both = vec![0u8];
        both.extend_from_slice(&section);
        both.extend_from_slice(&section);
        both.push(0xFF);
        assert_eq!(reader.push(true, &both).len(), 2);
    }

    #[test]
    fn annex_b_scanner_matches_whole_buffer_split() {
        let stream: Vec<u8> = [
            &[0, 0, 0, 1, 0x09, 0xF0][..],
            &[0, 0, 0, 1, 0x67, 0x64, 0x00, 0x1F, 0xAC][..],
            &[0, 0, 1, 0x68, 0xEE, 0x3C, 0x80][..],
            &[0, 0, 1, 0x65, 0x88, 0x84, 0x00, 0x00, 0x03, 0x01, 0x02][..],
            &[0, 0, 0, 0, 1, 0x41, 0x9A, 0x00][..],
        ]
        .concat();
        let expected: Vec<Vec<u8>> = h264::nal_units(&stream).map(<[u8]>::to_vec).collect();
        assert_eq!(expected.len(), 5);
        for chunk in 1..=stream.len() {
            let mut scanner = AnnexBScanner::default();
            let mut starts = Vec::new();
            let mut nals = Vec::new();
            let mut on = |event: NalEvent<'_>| match event {
                NalEvent::Start(head) => starts.push(head[0]),
                NalEvent::Complete(nal) => nals.push(nal.to_vec()),
                NalEvent::Discarded => panic!("nothing to discard"),
            };
            for piece in stream.chunks(chunk) {
                scanner.push(piece, &mut on);
            }
            scanner.flush(&mut on);
            assert_eq!(nals, expected, "chunk size {chunk}");
            assert_eq!(
                starts,
                vec![0x09, 0x67, 0x68, 0x65, 0x41],
                "chunk size {chunk}"
            );
        }
    }

    #[test]
    fn classifies_access_unit_boundaries() {
        let h264 = |head: &[u8]| classify(VideoCodec::H264, head);
        assert!(h264(&[0x65, 0x88]).vcl && h264(&[0x65, 0x88]).starts_access_unit);
        assert!(h264(&[0x65, 0x88]).keyframe);
        assert!(!h264(&[0x41, 0x40]).starts_access_unit); // first_mb_in_slice != 0
        assert!(h264(&[0x09, 0xF0]).starts_access_unit && !h264(&[0x09]).vcl);
        assert!(h264(&[0x06, 0x05]).starts_access_unit);
        assert!(!h264(&[0x0C, 0xFF]).starts_access_unit); // filler

        let h265 = |head: &[u8]| classify(VideoCodec::H265, head);
        let idr = h265(&[0x26, 0x01, 0xAF]);
        assert!(idr.vcl && idr.keyframe && idr.starts_access_unit);
        assert!(!h265(&[0x02, 0x01, 0x40]).starts_access_unit); // not the first segment
        assert!(h265(&[0x40, 0x01, 0x0C]).starts_access_unit); // VPS
        assert!(!h265(&[0x50, 0x01, 0x00]).starts_access_unit); // suffix SEI
        assert!(!h265(&[0x26, 0x09, 0xAF]).starts_access_unit); // layer 1
    }

    /// Feeds access units (lists of NAL units with a PTS) to an assembler.
    fn assemble(codec: VideoCodec, units: &[(i64, Vec<Vec<u8>>)]) -> (Vec<MediaEvent>, DemuxStats) {
        let mut assembler = AccessUnitAssembler::new(codec);
        let mut clock = TimestampExtender::new();
        let mut stats = DemuxStats::default();
        let mut out = Vec::new();
        let mut sink = Sink {
            clock: &mut clock,
            stats: &mut stats,
            out: &mut out,
        };
        for (pts, nals) in units {
            assembler.pending_ts = Some((*pts, *pts));
            for nal in nals {
                assembler.on_nal(NalEvent::Start(nal), &mut sink);
                assembler.on_nal(NalEvent::Complete(nal), &mut sink);
            }
        }
        assembler.finish(&mut sink);
        (out, stats)
    }

    #[test]
    fn drops_rasl_pictures_after_a_starting_cra() {
        use crate::media::h264::tests::hex;
        use crate::media::h265::tests::{PPS_640X360, SPS_640X360, VPS_640X360};
        // Two-byte H.265 NAL headers for a type, then first_slice_segment_in_pic_flag.
        let slice = |nal_type: u8| vec![nal_type << 1, 0x01, 0x80, 0x11];
        let (cra, rasl, radl, trail) = (21, 8, 6, 1);
        let mut first = vec![hex(VPS_640X360), hex(SPS_640X360), hex(PPS_640X360)];
        first.push(slice(cra));
        let units = vec![
            (0, first),
            (1, vec![slice(rasl)]),
            (2, vec![slice(rasl)]),
            (3, vec![slice(radl)]),
            (4, vec![slice(trail)]),
            (5, vec![slice(cra)]),
            (6, vec![slice(rasl)]), // decodable: the earlier pictures were received
            (7, vec![slice(trail)]),
        ];
        let (events, stats) = assemble(VideoCodec::H265, &units);
        let pts: Vec<i64> = events
            .iter()
            .filter_map(|e| match e {
                MediaEvent::Video(frame) => Some(frame.pts),
                _ => None,
            })
            .collect();
        assert_eq!(pts, vec![0, 3, 4, 5, 6, 7]);
        assert_eq!(stats.dropped_video_frames, 2);
        assert!(matches!(events[0], MediaEvent::VideoConfig(_)));
    }

    #[test]
    fn multi_slice_pictures_form_one_access_unit() {
        use crate::media::h264::tests::{PPS_HIGH_640X360, SPS_HIGH_640X360, hex};
        // IDR slices: first_mb_in_slice = 0 (bit 1) and then non-zero (bits 010).
        let first = vec![0x65, 0x88, 0x01];
        let second = vec![0x65, 0x40, 0x01];
        let sei = vec![0x06, 0x05, 0x01];
        let units = vec![
            (
                0,
                vec![
                    hex(SPS_HIGH_640X360),
                    hex(PPS_HIGH_640X360),
                    sei.clone(),
                    first.clone(),
                    second.clone(),
                ],
            ),
            (3000, vec![vec![0x41, 0x9A, 0x01], vec![0x41, 0x40, 0x02]]),
        ];
        let (events, _) = assemble(VideoCodec::H264, &units);
        let frames: Vec<&VideoFrame> = events
            .iter()
            .filter_map(|e| match e {
                MediaEvent::Video(frame) => Some(frame),
                _ => None,
            })
            .collect();
        assert_eq!(frames.len(), 2);
        let nals: Vec<&[u8]> = frames[0].nal_units().collect();
        assert_eq!(nals, vec![&sei[..], &first[..], &second[..]]);
        assert!(frames[0].keyframe && !frames[1].keyframe);
        assert_eq!(frames[1].nal_units().count(), 2);
    }

    #[test]
    fn parameter_sets_track_changes() {
        let mut list = Vec::new();
        assert!(ParameterSets::upsert(&mut list, 0, &[1, 2]));
        assert!(!ParameterSets::upsert(&mut list, 0, &[1, 2]));
        assert!(ParameterSets::upsert(&mut list, 1, &[3]));
        assert_eq!(list[0].0, 1);
        assert!(ParameterSets::upsert(&mut list, 0, &[9]));
        assert_eq!(list[0], (0, Bytes::from_static(&[9])));
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn resyncs_after_garbage() {
        let mut demuxer = TsDemuxer::new();
        let mut events = Vec::new();
        let mut data = vec![0x47, 1, 2, 3];
        let null = {
            let mut p = [0xFFu8; PACKET_SIZE];
            p[..4].copy_from_slice(&[0x47, 0x1F, 0xFF, 0x10]);
            p
        };
        for _ in 0..3 {
            data.extend_from_slice(&null);
        }
        data.extend_from_slice(&[0x12; 50]);
        data.extend_from_slice(&null);
        data.extend_from_slice(&null);
        demuxer.push(&data, &mut events).unwrap();
        demuxer.flush(&mut events).unwrap();
        assert_eq!(demuxer.stats().packets, 5);
        assert_eq!(demuxer.stats().skipped_bytes, 4 + 50);
        assert_eq!(demuxer.stats().sync_losses, 1);
        assert!(events.is_empty());
    }

    #[test]
    fn reports_lost_sync_once() {
        let mut demuxer = TsDemuxer::new();
        let mut events = Vec::new();
        let garbage = vec![0u8; 256 * 1024];
        let mut errors = 0;
        for _ in 0..12 {
            if let Err(MediaError::LostSync { skipped }) = demuxer.push(&garbage, &mut events) {
                assert!(skipped >= LOST_SYNC_LIMIT);
                errors += 1;
            }
        }
        assert_eq!(errors, 1);
    }
}
