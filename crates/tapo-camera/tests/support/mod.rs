//! Test-only helpers: a small MPEG-TS muxer that writes Tapo-style transport streams from
//! elementary streams, a minimal PES extractor, and fixture access.
//!
//! This code is deliberately independent of the demuxer under test.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub const PACKET_SIZE: usize = 188;

/// PIDs and program layout of a Tapo camera stream (as documented by go2rtc).
pub const TAPO_PMT_PID: u16 = 18;
pub const TAPO_VIDEO_PID: u16 = 68;
pub const TAPO_AUDIO_PID: u16 = 69;
pub const TAPO_PROGRAM: u16 = 1;

pub const STREAM_TYPE_H264: u8 = 0x1B;
pub const STREAM_TYPE_H265: u8 = 0x24;
pub const STREAM_TYPE_PCMA_TAPO: u8 = 0x90;
pub const STREAM_TYPE_PCMU_TAPO: u8 = 0x91;

pub const WRAP: u64 = 1 << 33;

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

pub fn fixture(name: &str) -> Vec<u8> {
    let path = fixtures_dir().join(name);
    std::fs::read(&path).unwrap_or_else(|err| panic!("reading {}: {err}", path.display()))
}

/// CRC-32/MPEG-2 over a PSI section.
pub fn crc32_mpeg2(data: &[u8]) -> u32 {
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

/// Writes transport stream packets for one program.
pub struct TsMuxer {
    out: Vec<u8>,
    counters: Vec<u8>,
    pmt_pid: u16,
    program: u16,
    pcr_pid: u16,
    streams: Vec<(u8, u16)>,
}

/// How to packetize one PES packet.
#[derive(Debug, Clone, Copy, Default)]
pub struct PesOptions {
    pub pts: Option<u64>,
    pub dts: Option<u64>,
    /// Write PES_packet_length (otherwise 0, "unbounded", as video usually is).
    pub bounded: bool,
    /// PCR base to put in the first packet's adaptation field.
    pub pcr: Option<u64>,
    /// Set random_access_indicator in the first packet.
    pub random_access: bool,
}

impl TsMuxer {
    pub fn new(pmt_pid: u16, program: u16, pcr_pid: u16, streams: &[(u8, u16)]) -> Self {
        Self {
            out: Vec::new(),
            counters: vec![0; 8192],
            pmt_pid,
            program,
            pcr_pid,
            streams: streams.to_vec(),
        }
    }

    /// A muxer with the Tapo PID layout.
    pub fn tapo(video_stream_type: u8, audio_stream_type: Option<u8>) -> Self {
        let mut streams = vec![(video_stream_type, TAPO_VIDEO_PID)];
        if let Some(audio) = audio_stream_type {
            streams.push((audio, TAPO_AUDIO_PID));
        }
        Self::new(TAPO_PMT_PID, TAPO_PROGRAM, TAPO_VIDEO_PID, &streams)
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.out
    }

    pub fn len(&self) -> usize {
        self.out.len()
    }

    fn next_counter(&mut self, pid: u16) -> u8 {
        let counter = &mut self.counters[usize::from(pid)];
        let value = *counter;
        *counter = (*counter + 1) & 0x0F;
        value
    }

    fn write_section(&mut self, pid: u16, mut section: Vec<u8>) {
        let crc = crc32_mpeg2(&section);
        section.extend_from_slice(&crc.to_be_bytes());
        let mut payload = vec![0u8]; // pointer_field
        payload.extend_from_slice(&section);
        assert!(
            payload.len() <= 184,
            "test muxer writes single-packet sections"
        );
        payload.resize(184, 0xFF);
        let cc = self.next_counter(pid);
        self.out
            .extend_from_slice(&[0x47, 0x40 | (pid >> 8) as u8, pid as u8, 0x10 | cc]);
        self.out.extend_from_slice(&payload);
    }

    /// Writes a PAT and a PMT.
    pub fn write_tables(&mut self) {
        let mut pat = vec![0x00, 0xB0, 0x00, 0x00, 0x01, 0xC1, 0x00, 0x00];
        pat.extend_from_slice(&self.program.to_be_bytes());
        pat.extend_from_slice(&(0xE000 | self.pmt_pid).to_be_bytes());
        let length = (pat.len() - 3 + 4) as u16;
        pat[1] = 0xB0 | (length >> 8) as u8;
        pat[2] = length as u8;
        self.write_section(0, pat);

        let mut pmt = vec![0x02, 0xB0, 0x00];
        pmt.extend_from_slice(&self.program.to_be_bytes());
        pmt.extend_from_slice(&[0xC1, 0x00, 0x00]);
        pmt.extend_from_slice(&(0xE000 | self.pcr_pid).to_be_bytes());
        pmt.extend_from_slice(&[0xF0, 0x00]); // program_info_length = 0
        for &(stream_type, pid) in &self.streams {
            pmt.push(stream_type);
            pmt.extend_from_slice(&(0xE000 | pid).to_be_bytes());
            pmt.extend_from_slice(&[0xF0, 0x00]); // ES_info_length = 0
        }
        let length = (pmt.len() - 3 + 4) as u16;
        pmt[1] = 0xB0 | (length >> 8) as u8;
        pmt[2] = length as u8;
        let pmt_pid = self.pmt_pid;
        self.write_section(pmt_pid, pmt);
    }

    /// Writes one PES packet.
    pub fn write_pes(&mut self, pid: u16, stream_id: u8, payload: &[u8], options: PesOptions) {
        let pes = pes_packet(stream_id, payload, &options);
        let mut first_af = Vec::new();
        if options.pcr.is_some() || options.random_access {
            let mut flags = 0u8;
            if options.random_access {
                flags |= 0x40;
            }
            if options.pcr.is_some() {
                flags |= 0x10;
            }
            first_af.push(flags);
            if let Some(pcr) = options.pcr {
                let base = pcr % WRAP;
                first_af.extend_from_slice(&[
                    (base >> 25) as u8,
                    (base >> 17) as u8,
                    (base >> 9) as u8,
                    (base >> 1) as u8,
                    (((base & 1) as u8) << 7) | 0x7E,
                    0x00,
                ]);
            }
        }
        self.write_packets(pid, &pes, first_af);
    }

    fn write_packets(&mut self, pid: u16, mut data: &[u8], first_af: Vec<u8>) {
        let mut first = true;
        let mut af = first_af;
        while first || !data.is_empty() {
            let af_bytes = if af.is_empty() { 0 } else { 1 + af.len() };
            let room = 184 - af_bytes;
            let take = data.len().min(room);
            let stuffing = room - take;
            let mut adaptation: Vec<u8> = Vec::new();
            if !af.is_empty() || stuffing > 0 {
                let mut content = std::mem::take(&mut af);
                if stuffing > 0 {
                    if content.is_empty() {
                        // The length byte itself takes one of the stuffing bytes.
                        if stuffing > 1 {
                            content.push(0x00);
                            content.extend(std::iter::repeat_n(0xFF, stuffing - 2));
                        }
                    } else {
                        content.extend(std::iter::repeat_n(0xFF, stuffing));
                    }
                }
                adaptation.push(content.len() as u8);
                adaptation.extend_from_slice(&content);
            }
            let afc = if adaptation.is_empty() { 0x10 } else { 0x30 };
            let pusi = if first { 0x40 } else { 0x00 };
            let cc = self.next_counter(pid);
            self.out
                .extend_from_slice(&[0x47, pusi | (pid >> 8) as u8, pid as u8, afc | cc]);
            self.out.extend_from_slice(&adaptation);
            self.out.extend_from_slice(&data[..take]);
            debug_assert_eq!(self.out.len() % PACKET_SIZE, 0);
            data = &data[take..];
            first = false;
        }
    }
}

fn timestamp_bytes(prefix: u8, t: u64) -> [u8; 5] {
    let t = t % WRAP;
    [
        (prefix << 4) | ((((t >> 30) & 0x07) as u8) << 1) | 1,
        (t >> 22) as u8,
        ((((t >> 15) & 0x7F) as u8) << 1) | 1,
        (t >> 7) as u8,
        (((t & 0x7F) as u8) << 1) | 1,
    ]
}

/// Builds a PES packet (header and payload).
pub fn pes_packet(stream_id: u8, payload: &[u8], options: &PesOptions) -> Vec<u8> {
    let mut fields = Vec::new();
    let flags = match (options.pts, options.dts) {
        (Some(pts), Some(dts)) => {
            fields.extend_from_slice(&timestamp_bytes(3, pts));
            fields.extend_from_slice(&timestamp_bytes(1, dts));
            0xC0
        }
        (Some(pts), None) => {
            fields.extend_from_slice(&timestamp_bytes(2, pts));
            0x80
        }
        _ => 0x00,
    };
    let mut pes = vec![0, 0, 1, stream_id, 0, 0, 0x84, flags, fields.len() as u8];
    pes.extend_from_slice(&fields);
    pes.extend_from_slice(payload);
    if options.bounded {
        let length = u16::try_from(pes.len() - 6).expect("bounded PES fits in 16 bits");
        pes[4..6].copy_from_slice(&length.to_be_bytes());
    }
    pes
}

/// Concatenates the PES payloads of one PID (no continuity checks): the elementary
/// stream as the muxer received it.
pub fn extract_es(ts: &[u8], pid: u16) -> Vec<u8> {
    let mut es = Vec::new();
    let mut header_left = 0usize;
    for packet in ts.as_chunks::<PACKET_SIZE>().0 {
        assert_eq!(packet[0], 0x47);
        let packet_pid = (u16::from(packet[1] & 0x1F) << 8) | u16::from(packet[2]);
        if packet_pid != pid {
            continue;
        }
        let pusi = packet[1] & 0x40 != 0;
        let afc = (packet[3] >> 4) & 0x03;
        let mut start = 4;
        if afc & 0x02 != 0 {
            start += 1 + usize::from(packet[4]);
        }
        if afc & 0x01 == 0 || start >= PACKET_SIZE {
            continue;
        }
        let mut payload = &packet[start..];
        if pusi {
            assert_eq!(&payload[..3], &[0, 0, 1], "PES start code");
            header_left = 9 + usize::from(payload[8]);
        }
        let skip = header_left.min(payload.len());
        payload = &payload[skip..];
        header_left -= skip;
        es.extend_from_slice(payload);
    }
    es
}

/// Positions of three-byte start codes.
fn start_codes(data: &[u8]) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            positions.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    positions
}

/// Splits an Annex B stream into NAL units (without start codes or trailing zeros).
pub fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let codes = start_codes(data);
    let mut nals = Vec::new();
    for (i, &code) in codes.iter().enumerate() {
        let start = code + 3;
        let mut end = codes.get(i + 1).copied().unwrap_or(data.len());
        while end > start && data[end - 1] == 0 {
            end -= 1;
        }
        if end > start {
            nals.push(&data[start..end]);
        }
    }
    nals
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    H265,
}

impl Codec {
    pub fn nal_type(self, nal: &[u8]) -> u8 {
        match self {
            Codec::H264 => nal[0] & 0x1F,
            Codec::H265 => (nal[0] >> 1) & 0x3F,
        }
    }

    pub fn is_aud(self, nal: &[u8]) -> bool {
        self.nal_type(nal) == if self == Codec::H264 { 9 } else { 35 }
    }

    pub fn is_parameter_set(self, nal: &[u8]) -> bool {
        match self {
            Codec::H264 => matches!(self.nal_type(nal), 7 | 8),
            Codec::H265 => matches!(self.nal_type(nal), 32..=34),
        }
    }

    pub fn is_keyframe_slice(self, nal: &[u8]) -> bool {
        match self {
            Codec::H264 => self.nal_type(nal) == 5,
            Codec::H265 => (16..=23).contains(&self.nal_type(nal)),
        }
    }

    pub fn stream_type(self) -> u8 {
        match self {
            Codec::H264 => STREAM_TYPE_H264,
            Codec::H265 => STREAM_TYPE_H265,
        }
    }
}

/// One access unit as a list of NAL units.
#[derive(Debug, Clone)]
pub struct AccessUnit {
    pub nals: Vec<Vec<u8>>,
}

impl AccessUnit {
    pub fn is_keyframe(&self, codec: Codec) -> bool {
        self.nals.iter().any(|nal| codec.is_keyframe_slice(nal))
    }

    /// Annex B bytes with four-byte start codes.
    pub fn to_annex_b(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in &self.nals {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(nal);
        }
        out
    }
}

/// Splits an elementary stream that has an AUD at the start of every access unit.
pub fn split_access_units(es: &[u8], codec: Codec) -> Vec<AccessUnit> {
    let mut units: Vec<AccessUnit> = Vec::new();
    for nal in nal_units(es) {
        if codec.is_aud(nal) || units.is_empty() {
            units.push(AccessUnit { nals: Vec::new() });
        }
        units.last_mut().unwrap().nals.push(nal.to_vec());
    }
    units
}

/// How to build a Tapo-style test stream.
#[derive(Debug, Clone, Copy)]
pub struct StreamOptions {
    pub codec: Codec,
    pub start_pts: u64,
    /// Ticks per video frame.
    pub frame_ticks: u64,
    pub strip_aud: bool,
    /// Also write DTS (equal to PTS).
    pub write_dts: bool,
    /// Audio stream type and bytes per audio PES (G.711: one byte per sample).
    pub audio: Option<(u8, usize)>,
    /// G.711 sample rate, for audio timestamps.
    pub audio_rate: u64,
}

impl StreamOptions {
    pub fn tapo(codec: Codec) -> Self {
        Self {
            codec,
            start_pts: 900_000,
            frame_ticks: 6_000,
            strip_aud: false,
            write_dts: false,
            audio: Some((STREAM_TYPE_PCMA_TAPO, 320)),
            audio_rate: 8_000,
        }
    }
}

/// Muxes access units and G.711 audio into a Tapo-style transport stream: PMT PID 18,
/// video PID 68 (unbounded PES, one access unit each, PCR), audio PID 69 (bounded PES),
/// PAT/PMT before every keyframe, audio interleaved by timestamp.
pub fn tapo_stream(units: &[AccessUnit], audio: &[u8], options: &StreamOptions) -> Vec<u8> {
    let codec = options.codec;
    let mut mux = TsMuxer::tapo(codec.stream_type(), options.audio.map(|(t, _)| t));
    let audio_chunks: Vec<&[u8]> = match options.audio {
        Some((_, bytes)) => audio.chunks(bytes).collect(),
        None => Vec::new(),
    };
    let mut next_audio = 0usize;
    let mut audio_samples = 0u64;
    let mut tables_written = false;
    for (i, unit) in units.iter().enumerate() {
        let pts = options.start_pts + i as u64 * options.frame_ticks;
        let keyframe = unit.is_keyframe(codec);
        if keyframe || !tables_written {
            mux.write_tables();
            tables_written = true;
        }
        while next_audio < audio_chunks.len() {
            let audio_pts = options.start_pts + audio_samples * 90_000 / options.audio_rate;
            if audio_pts > pts {
                break;
            }
            let chunk = audio_chunks[next_audio];
            mux.write_pes(
                TAPO_AUDIO_PID,
                0xC0,
                chunk,
                PesOptions {
                    pts: Some(audio_pts),
                    bounded: true,
                    ..PesOptions::default()
                },
            );
            audio_samples += chunk.len() as u64;
            next_audio += 1;
        }
        let nals: Vec<Vec<u8>> = unit
            .nals
            .iter()
            .filter(|nal| !(options.strip_aud && codec.is_aud(nal)))
            .cloned()
            .collect();
        let payload = AccessUnit { nals }.to_annex_b();
        mux.write_pes(
            TAPO_VIDEO_PID,
            0xE0,
            &payload,
            PesOptions {
                pts: Some(pts),
                dts: options.write_dts.then_some(pts),
                bounded: false,
                pcr: Some(pts + WRAP - 9_000),
                random_access: keyframe,
            },
        );
    }
    // Remaining audio.
    while next_audio < audio_chunks.len() {
        let audio_pts = options.start_pts + audio_samples * 90_000 / options.audio_rate;
        let chunk = audio_chunks[next_audio];
        mux.write_pes(
            TAPO_AUDIO_PID,
            0xC0,
            chunk,
            PesOptions {
                pts: Some(audio_pts),
                bounded: true,
                ..PesOptions::default()
            },
        );
        audio_samples += chunk.len() as u64;
        next_audio += 1;
    }
    mux.into_bytes()
}

/// Deterministic xorshift64* generator for chunking tests.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `1..=max`.
    pub fn size(&mut self, max: usize) -> usize {
        1 + (self.next() % max as u64) as usize
    }
}

/// Whether a command-line tool can be run.
pub fn have_tool(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
}
