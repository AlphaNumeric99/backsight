//! H.264/H.265 out of RTP (RFC 6184 / RFC 7798), producing the player's media events.
//!
//! The stream's NAL units arrive as single-NAL packets, STAP-A aggregates or FU-A
//! fragments; this module reassembles them, groups them into access units (an RTP
//! marker bit, or the start of the next picture, ends one) and emits
//! [`MediaEvent`]s exactly like the Tapo MPEG-TS demuxer does: parameter sets are
//! stripped from frames and carried by [`MediaEvent::VideoConfig`], so the wire
//! format and the player stay brand-agnostic.
//!
//! Packet loss is handled the way a live stream can afford: a sequence gap or a
//! broken fragment drops the damaged frame and everything until the next keyframe.

use bytes::{Buf, Bytes, BytesMut};

use tapo_camera::media::{MediaEvent, VideoCodec, VideoFrame};
use tapo_camera::media::{h264, h265};

/// One depacketized H.264 access unit at most; larger ones are treated as damaged.
const MAX_ACCESS_UNIT_BYTES: usize = 4 * 1024 * 1024;

/// An RTP timestamp extended to 64 bits across wraparounds (90 kHz, like MPEG).
#[derive(Debug, Default)]
struct RtpClock {
    last: Option<u32>,
    extended: i64,
}

impl RtpClock {
    fn extend(&mut self, timestamp: u32) -> i64 {
        if let Some(last) = self.last {
            // A signed delta keeps the count through the 32-bit wraparound.
            self.extended += i64::from(timestamp.wrapping_sub(last) as i32);
        } else {
            self.extended = i64::from(timestamp);
        }
        self.last = Some(timestamp);
        self.extended
    }
}

/// Video depacketizer: feed RTP payloads, collect [`MediaEvent`]s.
#[derive(Debug)]
pub struct VideoDepacketizer {
    codec: VideoCodec,
    vps: Vec<Bytes>,
    /// Length-prefixed NAL units of the access unit being assembled.
    data: BytesMut,
    /// Whether the access unit being assembled has picture data.
    started: bool,
    has_vcl: bool,
    keyframe: bool,
    /// The extended RTP timestamp of the access unit being assembled.
    timestamp: i64,
    /// FU-A fragment under reassembly.
    fragment: Option<Vec<u8>>,
    /// Most recent parameter sets, oldest first.
    sps: Vec<Bytes>,
    pps: Vec<Bytes>,
    params_changed: bool,
    configured: bool,
    /// Frames are dropped until a keyframe arrives (stream start or packet loss).
    need_keyframe: bool,
    /// The sequence number the next packet should carry.
    expected_sequence: Option<u16>,
    clock: RtpClock,
    /// Events completed inside `push`, waiting to be handed to the caller.
    pending: Vec<MediaEvent>,
}

impl Default for VideoDepacketizer {
    fn default() -> Self {
        Self {
            codec: VideoCodec::H264,
            vps: Vec::new(),
            data: BytesMut::new(),
            started: false,
            has_vcl: false,
            keyframe: false,
            timestamp: 0,
            fragment: None,
            sps: Vec::new(),
            pps: Vec::new(),
            params_changed: false,
            configured: false,
            need_keyframe: true,
            expected_sequence: None,
            clock: RtpClock::default(),
            pending: Vec::new(),
        }
    }
}

/// H.264 NAL types that never appear in an access unit on the wire (they are carried
/// by the configuration instead) or that carry no picture data.
fn is_stripped(nal_type: u8) -> bool {
    matches!(
        nal_type,
        h264::nal_type::SPS
            | h264::nal_type::PPS
            | h264::nal_type::AUD
            | h264::nal_type::FILLER_DATA
            | h264::nal_type::SPS_EXTENSION
    )
}

impl VideoDepacketizer {
    pub fn with_codec(mut self, codec: VideoCodec) -> Self {
        self.codec = codec;
        self
    }
    /// Seeds the parameter sets from the SDP's `sprop-parameter-sets`, so the
    /// configuration is ready before the first in-band parameter sets arrive.
    pub fn with_parameter_sets(mut self, sets: &[Vec<u8>]) -> Self {
        for nal in sets {
            if self.codec == VideoCodec::H265 {
                self.on_nal(nal);
                continue;
            }
            match h264::nal_unit_type(nal) {
                Some(h264::nal_type::SPS) => self.sps.push(Bytes::from(nal.clone())),
                Some(h264::nal_type::PPS) => self.pps.push(Bytes::from(nal.clone())),
                _ => {}
            }
        }
        self.params_changed = !self.sps.is_empty() && !self.pps.is_empty();
        self
    }

    /// Feeds one RTP packet; appends the events it completed to `events`.
    pub fn push(&mut self, packet: &crate::qubo::rtsp::RtpPacket, events: &mut Vec<MediaEvent>) {
        if let Some(expected) = self.expected_sequence
            && packet.sequence != expected
        {
            // A gap or a duplicate: everything up to the next keyframe is
            // undecodable anyway.
            tracing::debug!(
                expected,
                got = packet.sequence,
                "live stream RTP sequence gap"
            );
            self.damaged();
        }
        self.expected_sequence = Some(packet.sequence.wrapping_add(1));

        let timestamp = self.clock.extend(packet.timestamp);
        if self.started && self.timestamp != timestamp && self.fragment.is_none() {
            self.finish();
        }
        if !self.started {
            self.started = true;
            self.timestamp = timestamp;
        }

        if let Err(err) = self.depacketize(&packet.payload) {
            tracing::debug!(%err, "live stream H.264 depacketizer error");
            self.damaged();
        }
        if packet.marker {
            self.finish();
        }
        events.append(&mut self.pending);
    }

    /// Splits one RTP payload into NAL units.
    fn depacketize(&mut self, payload: &[u8]) -> Result<(), &'static str> {
        if self.codec == VideoCodec::H265 {
            return self.depacketize_h265(payload);
        }
        let Some(&head) = payload.first() else {
            return Err("empty RTP payload");
        };
        let nal_type = head & 0x1F;
        match nal_type {
            1..=23 => {
                // A single, complete NAL unit.
                self.on_nal(payload);
            }
            24 => {
                // STAP-A: a 2-byte size before each NAL unit.
                let mut rest = &payload[1..];
                while rest.has_remaining() {
                    if rest.len() < 2 {
                        return Err("short STAP-A entry");
                    }
                    let len = u16::from_be_bytes([rest[0], rest[1]]) as usize;
                    rest.advance(2);
                    if rest.len() < len || len == 0 {
                        return Err("STAP-A entry exceeds the packet");
                    }
                    let nal = &rest[..len];
                    self.on_nal(nal);
                    rest.advance(len);
                }
            }
            28 => {
                // FU-A: one NAL unit in fragments. The first byte of the RTP payload
                // carries the reconstruction's forbidden/reference bits and type; the
                // second is the fragment flags and the NAL type.
                let Some(&flags) = payload.get(1) else {
                    return Err("short FU-A header");
                };
                let start = flags & 0x80 != 0;
                let end = flags & 0x40 != 0;
                let body = &payload[2..];
                if start {
                    if let Some(fragment) = self.fragment.take() {
                        tracing::debug!(len = fragment.len(), "dropped unfinished FU-A");
                        self.damaged();
                    }
                    let mut nal = Vec::with_capacity(body.len() + 16);
                    nal.push((head & 0xE0) | (flags & 0x1F));
                    nal.extend_from_slice(body);
                    self.fragment = Some(nal);
                } else {
                    let Some(fragment) = self.fragment.as_mut() else {
                        // A continuation without a start: the stream began mid-frame.
                        return Err("FU-A continuation without a start");
                    };
                    if fragment.len() + body.len() > MAX_ACCESS_UNIT_BYTES {
                        return Err("FU-A exceeds the access-unit limit");
                    }
                    fragment.extend_from_slice(body);
                }
                if end {
                    let Some(fragment) = self.fragment.take() else {
                        return Err("FU-A end without a start");
                    };
                    if fragment.len() < 2 {
                        return Err("empty FU-A fragment");
                    }
                    // The reconstruction byte is already in place; the fragment body
                    // follows it.
                    self.on_nal(&fragment);
                }
            }
            // STAP-B / MTAP / FU-B are only used in interleaved mode, which live
            // camera streams don't use.
            _ => return Err("unsupported H.264 RTP aggregation"),
        }
        Ok(())
    }

    /// RFC 7798 single-NAL, aggregation and fragmentation units (no DONL).
    fn depacketize_h265(&mut self, payload: &[u8]) -> Result<(), &'static str> {
        if payload.len() < 2 {
            return Err("short H.265 payload");
        }
        match (payload[0] >> 1) & 63 {
            48 => {
                let mut rest = &payload[2..];
                while !rest.is_empty() {
                    if rest.len() < 2 {
                        return Err("short H.265 aggregate");
                    }
                    let len = u16::from_be_bytes([rest[0], rest[1]]) as usize;
                    rest = &rest[2..];
                    if len < 2 || len > rest.len() {
                        return Err("invalid H.265 aggregate length");
                    }
                    self.on_nal(&rest[..len]);
                    rest = &rest[len..];
                }
            }
            49 => {
                if payload.len() < 4 {
                    return Err("short H.265 fragment");
                }
                let flags = payload[2];
                if flags & 0x80 != 0 {
                    if self.fragment.is_some() {
                        self.damaged();
                    }
                    self.fragment =
                        Some(vec![(payload[0] & 0x81) | ((flags & 63) << 1), payload[1]]);
                }
                let Some(fragment) = self.fragment.as_mut() else {
                    return Err("H.265 fragment without a start");
                };
                if fragment.len() + payload.len() - 3 > MAX_ACCESS_UNIT_BYTES {
                    return Err("H.265 fragment exceeds limit");
                }
                fragment.extend_from_slice(&payload[3..]);
                if flags & 0x40 != 0 {
                    let nal = self.fragment.take().unwrap();
                    self.on_nal(&nal);
                }
            }
            50 => return Err("H.265 PACI is unsupported"),
            _ => self.on_nal(payload),
        }
        Ok(())
    }

    /// Adds one complete NAL unit to the access unit, or records it as a parameter
    /// set.
    fn on_nal(&mut self, nal: &[u8]) {
        if self.codec == VideoCodec::H265 {
            let Some(kind) = h265::nal_unit_type(nal) else {
                return;
            };
            if matches!(kind, 32..=34) {
                let list = match kind {
                    32 => &mut self.vps,
                    33 => &mut self.sps,
                    _ => &mut self.pps,
                };
                if !list.iter().any(|existing| existing.as_ref() == nal) {
                    list.clear();
                    list.push(Bytes::copy_from_slice(nal));
                    self.params_changed = true;
                    self.need_keyframe = true;
                }
                return;
            }
            if matches!(kind, 35 | 38) {
                return;
            }
            if kind <= 31 {
                self.has_vcl = true;
                self.keyframe |= h265::is_irap(kind);
            }
            if self.data.len() + nal.len() + 4 > MAX_ACCESS_UNIT_BYTES {
                self.damaged();
                return;
            }
            self.data
                .extend_from_slice(&(nal.len() as u32).to_be_bytes());
            self.data.extend_from_slice(nal);
            return;
        }
        let Some(nal_type) = h264::nal_unit_type(nal) else {
            return;
        };
        if is_stripped(nal_type) {
            if matches!(nal_type, h264::nal_type::SPS | h264::nal_type::PPS) {
                self.store_parameter_set(nal, nal_type);
            }
            return;
        }
        if h264::is_vcl(nal_type) {
            self.has_vcl = true;
            self.keyframe |= nal_type == h264::nal_type::IDR_SLICE;
        }
        if self.data.len() + 4 + nal.len() > MAX_ACCESS_UNIT_BYTES {
            self.damaged();
            return;
        }
        // Bounded by MAX_ACCESS_UNIT_BYTES.
        self.data
            .extend_from_slice(&(nal.len() as u32).to_be_bytes());
        self.data.extend_from_slice(nal);
    }

    /// Records an SPS or PPS; marks the configuration as changed.
    fn store_parameter_set(&mut self, nal: &[u8], nal_type: u8) {
        let list = if nal_type == h264::nal_type::SPS {
            &mut self.sps
        } else {
            &mut self.pps
        };
        match list.iter().position(|existing| existing == nal) {
            Some(_) => {}
            None => {
                // A live relay changes its active parameter set when resolution
                // changes; do not build the next configuration from an old SPS.
                list.clear();
                list.push(Bytes::copy_from_slice(nal));
                self.params_changed = true;
                self.need_keyframe = true;
            }
        }
    }

    /// Ends the access unit; emits the configuration (if it changed) and the frame.
    fn finish(&mut self) {
        if !self.has_vcl || self.data.is_empty() {
            self.reset_access_unit();
            return;
        }
        if self.keyframe && self.params_changed && !self.sps.is_empty() && !self.pps.is_empty() {
            let sps: Vec<&[u8]> = self.sps.iter().map(|n| n.as_ref()).collect();
            let pps: Vec<&[u8]> = self.pps.iter().map(|n| n.as_ref()).collect();
            let config = match self.codec {
                VideoCodec::H264 => h264::video_config(&sps, &pps),
                VideoCodec::H265 => {
                    let vps: Vec<&[u8]> = self.vps.iter().map(|n| n.as_ref()).collect();
                    h265::video_config(&vps, &sps, &pps)
                }
            };
            match config {
                Ok(config) => {
                    self.pending.push(MediaEvent::VideoConfig(config));
                    self.params_changed = false;
                    self.configured = true;
                }
                Err(err) => tracing::debug!(%err, "ignoring unparsable parameter sets"),
            }
        }
        let keyframe = self.keyframe;
        if keyframe && self.configured && !self.params_changed {
            self.need_keyframe = false;
        }
        if !self.need_keyframe {
            self.pending.push(MediaEvent::Video(VideoFrame {
                pts: self.timestamp,
                dts: self.timestamp,
                keyframe,
                data: self.data.split_to(self.data.len()).freeze(),
            }));
        } else {
            tracing::debug!("dropping a frame before the first keyframe");
        }
        self.reset_access_unit();
    }

    /// Drops the current frame and everything until the next keyframe.
    fn damaged(&mut self) {
        self.reset_access_unit();
        self.fragment = None;
        self.need_keyframe = true;
    }

    fn reset_access_unit(&mut self) {
        self.data.clear();
        self.started = false;
        self.has_vcl = false;
        self.keyframe = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qubo::rtsp::RtpPacket;
    use tapo_camera::media::VideoConfig;

    fn packet(sequence: u16, timestamp: u32, marker: bool, payload: &[u8]) -> RtpPacket {
        RtpPacket {
            timestamp,
            marker,
            sequence,
            payload: Bytes::copy_from_slice(payload),
        }
    }

    /// A real 640x360 High-profile SPS and its PPS (the same vectors the
    /// `tapo-camera` demuxer tests use).
    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
            .collect()
    }
    const SPS_640X360: &str = "67640016acb405017fcb8088000003000800000300f078b175";
    const PPS_640X360: &str = "68ef3cb0";
    fn sps() -> Vec<u8> {
        hex(SPS_640X360)
    }
    fn pps() -> Vec<u8> {
        hex(PPS_640X360)
    }

    fn idr() -> Vec<u8> {
        // nal_ref_idc 3, IDR slice, first_mb_in_slice == 0.
        vec![0x65, 0x88, 0x84, 0x21, 0xA0]
    }

    fn pframe() -> Vec<u8> {
        vec![0x41, 0x9A, 0x20, 0x1E, 0x50]
    }

    #[test]
    fn single_nal_packets_assemble_into_frames() {
        let mut depacketizer = VideoDepacketizer::default();
        let mut events = Vec::new();
        depacketizer.push(&packet(1, 90_000, false, &sps()), &mut events);
        depacketizer.push(&packet(2, 90_000, false, &pps()), &mut events);
        depacketizer.push(&packet(3, 90_000, true, &idr()), &mut events);
        // No marker on the PPS/SPS packet: the frame is finished by the IDR's marker.
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], MediaEvent::VideoConfig(_)));
        let MediaEvent::Video(frame) = &events[1] else {
            panic!();
        };
        assert!(frame.keyframe);
        assert_eq!(frame.pts, 90_000);
        assert_eq!(frame.data.len(), 4 + idr().len());
        // The parameter sets were stripped.
        assert_eq!(&frame.data[4..5], &[0x65]);
    }

    #[test]
    fn fu_a_fragments_reassemble() {
        let mut depacketizer = VideoDepacketizer::default().with_parameter_sets(&[sps(), pps()]);
        let mut events = Vec::new();
        let nal = idr();
        // The FU indicator's and FU header's bits rebuild the NAL's header byte, so
        // the fragments carry the NAL body from its second byte on.
        let body = &nal[1..];
        let mut first = vec![0x7C, 0x85];
        first.extend_from_slice(&body[..2]);
        depacketizer.push(&packet(1, 90_000, false, &first), &mut events);
        let mut mid = vec![0x7C, 0x05];
        mid.extend_from_slice(&body[2..3]);
        depacketizer.push(&packet(2, 90_000, false, &mid), &mut events);
        // End with the marker bit.
        let mut last = vec![0x7C, 0x45];
        last.extend_from_slice(&body[3..]);
        depacketizer.push(&packet(3, 90_000, true, &last), &mut events);
        assert_eq!(events.len(), 2);
        let MediaEvent::Video(frame) = &events[1] else {
            panic!();
        };
        assert!(frame.keyframe);
        assert_eq!(&frame.data[4..], &nal[..]);
    }

    #[test]
    fn stap_a_carries_parameter_sets() {
        let mut depacketizer = VideoDepacketizer::default();
        let mut events = Vec::new();
        let mut stap = vec![24u8];
        for nal in [sps(), pps()] {
            stap.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            stap.extend_from_slice(&nal);
        }
        depacketizer.push(&packet(1, 90_000, true, &stap), &mut events);
        // A STAP-A with only parameter sets completes no frame but seeds the config.
        assert!(events.is_empty());
        depacketizer.push(&packet(2, 180_000, true, &idr()), &mut events);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], MediaEvent::VideoConfig(_)));
    }

    #[test]
    fn frames_are_dropped_until_a_keyframe() {
        let mut depacketizer = VideoDepacketizer::default().with_parameter_sets(&[sps(), pps()]);
        let mut events = Vec::new();
        // The stream starts mid-GOP.
        depacketizer.push(&packet(10, 90_000, true, &pframe()), &mut events);
        depacketizer.push(&packet(11, 90_300, true, &pframe()), &mut events);
        assert!(events.is_empty());
        depacketizer.push(&packet(12, 90_600, true, &idr()), &mut events);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn a_sequence_gap_damages_until_the_next_keyframe() {
        let mut depacketizer = VideoDepacketizer::default().with_parameter_sets(&[sps(), pps()]);
        let mut events = Vec::new();
        depacketizer.push(&packet(1, 90_000, true, &idr()), &mut events);
        events.clear();
        // A gap of one packet.
        depacketizer.push(&packet(3, 90_300, true, &pframe()), &mut events);
        assert!(events.is_empty());
        depacketizer.push(&packet(4, 90_600, true, &pframe()), &mut events);
        assert!(events.is_empty());
        depacketizer.push(&packet(5, 90_900, true, &idr()), &mut events);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn sprop_parameter_sets_seed_the_configuration() {
        let mut depacketizer = VideoDepacketizer::default().with_parameter_sets(&[sps(), pps()]);
        let mut events = Vec::new();
        depacketizer.push(&packet(1, 90_000, true, &idr()), &mut events);
        assert_eq!(events.len(), 2);
        let MediaEvent::VideoConfig(config) = &events[0] else {
            panic!();
        };
        let VideoConfig { width, height, .. } = config;
        assert_eq!(*width, 640);
        assert_eq!(*height, 360);
    }

    #[test]
    fn the_clock_wraps_cleanly() {
        let mut clock = RtpClock {
            last: Some(0xFFFF_F000),
            extended: 0xFFFF_F000_i64,
        };
        assert_eq!(clock.extend(0x1000), 0x1_0000_1000);
        let mut clock = RtpClock::default();
        assert_eq!(clock.extend(1000), 1000);
        assert_eq!(clock.extend(1500), 1500);
        assert_eq!(clock.extend(500), 500);
    }

    #[test]
    fn the_marker_bit_end_of_frame_keeps_the_first_timestamp() {
        let mut depacketizer = VideoDepacketizer::default().with_parameter_sets(&[sps(), pps()]);
        let mut events = Vec::new();
        // Two FU-A fragments of the same frame, the timestamp of the first counts.
        let nal = idr();
        let body = &nal[1..];
        let mut first = vec![0x7C, 0x85];
        first.extend_from_slice(&body[..2]);
        depacketizer.push(&packet(1, 90_000, false, &first), &mut events);
        let mut last = vec![0x7C, 0x45];
        last.extend_from_slice(&body[2..]);
        depacketizer.push(&packet(2, 90_360, true, &last), &mut events);
        let MediaEvent::Video(frame) = &events[1] else {
            panic!();
        };
        assert_eq!(frame.pts, 90_000);
    }

    fn h265_sets() -> Vec<Vec<u8>> {
        // The x265 fixtures used by the shared media codec tests.
        [
            "40010c01ffff01600000030090000003000003003fba0240",
            "42010101600000030090000003000003003fa00502016965ba924caf0168080000030008000003007840",
            "4401c172b46240",
        ]
        .map(hex)
        .to_vec()
    }

    #[test]
    fn h265_fragments_preserve_both_nal_header_bytes() {
        let mut depacketizer = VideoDepacketizer::default()
            .with_codec(VideoCodec::H265)
            .with_parameter_sets(&h265_sets());
        let mut events = Vec::new();
        depacketizer.push(
            &packet(1, 90_000, false, &[0x62, 0x01, 0x93, 0x88, 0x21]),
            &mut events,
        );
        depacketizer.push(
            &packet(2, 90_000, true, &[0x62, 0x01, 0x53, 0xa0]),
            &mut events,
        );
        let MediaEvent::VideoConfig(config) = &events[0] else {
            panic!("no configuration");
        };
        assert_eq!((config.width, config.height), (640, 360));
        let MediaEvent::Video(frame) = &events[1] else {
            panic!("no video");
        };
        assert!(frame.keyframe);
        assert_eq!(&frame.data[4..], &[0x26, 0x01, 0x88, 0x21, 0xa0]);
    }

    #[test]
    fn h265_aggregation_strips_parameter_sets_from_frames() {
        let mut depacketizer = VideoDepacketizer::default().with_codec(VideoCodec::H265);
        let mut aggregate = vec![0x60, 1];
        let mut units = h265_sets();
        units.push(vec![0x26, 1, 0x88, 0x21]);
        for nal in units {
            aggregate.extend_from_slice(&(nal.len() as u16).to_be_bytes());
            aggregate.extend_from_slice(&nal);
        }
        let mut events = Vec::new();
        depacketizer.push(&packet(1, 90_000, true, &aggregate), &mut events);
        assert!(matches!(events[0], MediaEvent::VideoConfig(_)));
        let MediaEvent::Video(frame) = &events[1] else {
            panic!("no video");
        };
        assert_eq!(
            frame.nal_units().collect::<Vec<_>>(),
            vec![&[0x26, 1, 0x88, 0x21][..]]
        );
    }
}
