//! A minimal RTSP client, just enough for the Qubo cloud's live relay.
//!
//! The cloud hands out signed `rtsps://` URLs (RTSP over TLS, port 443) that relay the
//! camera through Wowza. Only what live view needs is implemented: `DESCRIBE` (parse
//! the SDP for the video track), `SETUP` with interleaved RTP-over-TCP (no extra UDP
//! ports through NATs and firewalls), and `PLAY`. Audio tracks are ignored for now.
//!
//! The camera's audio is AAC in RTP, which the player pipeline doesn't decode yet
//! (Tapo audio is G.711); Qubo live view is video-only until that changes.

use std::collections::HashMap;
use std::time::Duration;

use bytes::{Buf, Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::TlsStream;

use crate::error::{ApiError, ApiResult};

const MAX_REPLY_BYTES: usize = 1024 * 1024;

/// One received RTP packet.
#[derive(Debug, Clone)]
pub struct RtpPacket {
    /// The RTP timestamp, in the stream's clock (90 kHz for H.264).
    pub timestamp: u32,
    /// The marker bit: the last packet of an access unit.
    pub marker: bool,
    /// The sequence number, for ordering diagnostics.
    pub sequence: u16,
    pub payload: Bytes,
}

/// The parts of the session's SDP the stream needs.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Sdp {
    /// The `a=control` URL of the first video track.
    pub video_control: Option<String>,
    /// `a=fmtp:… sprop-parameter-sets=` as raw NAL units, if the SDP carries them.
    pub sprop_parameter_sets: Vec<Vec<u8>>,
    pub codec: Option<String>,
}

/// A live RTSP session over TLS.
pub struct RtspSession<S = TlsStream<TcpStream>> {
    stream: S,
    /// Bytes read from the socket, waiting to be parsed into frames.
    buffer: BytesMut,
    cseq: u32,
    /// The server-assigned session id, needed by `PLAY`.
    session: Option<String>,
    pub parameter_sets: Vec<Vec<u8>>,
    pub codec: String,
    aggregate_url: String,
    video_channel: u8,
    keepalive_at: tokio::time::Instant,
}

impl<S> std::fmt::Debug for RtspSession<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtspSession")
            .field("cseq", &self.cseq)
            .finish_non_exhaustive()
    }
}

/// Splits `rtsps://host:port/path?query` into its pieces.
struct Url {
    /// The URL exactly as the cloud handed it to us; Wowza's tokens are bound to it.
    original: String,
    secure: bool,
    /// Host and port as the URL spelled them, for rebuilding request URLs.
    authority: String,
    host: String,
    port: u16,
    /// The path, without the query.
    path: String,
    /// The query string, without the `?`.
    query: Option<String>,
}

fn parse_url(url: &str) -> ApiResult<Url> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| ApiError::invalid("the stream URL has no scheme"))?;
    let (secure, default_port) = match scheme {
        "rtsps" => (true, 443),
        "rtsp" => (false, 554),
        _ => return Err(ApiError::invalid(format!("unsupported scheme {scheme:?}"))),
    };
    let (authority, path_and_query) = rest.split_once('/').unwrap_or((rest, ""));
    // An IPv6 literal in brackets keeps its colons; its port follows the bracket.
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let (host, rest) = stripped
            .split_once(']')
            .ok_or_else(|| ApiError::invalid("the stream URL has a malformed IPv6 host"))?;
        let port = rest
            .strip_prefix(':')
            .and_then(|p| p.parse().ok())
            .unwrap_or(default_port);
        (host.to_owned(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_owned(), p.parse().unwrap_or(default_port)),
            None => (authority.to_owned(), default_port),
        }
    };
    let (path, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p.to_owned(), Some(q.to_owned())),
        None => (path_and_query.to_owned(), None),
    };
    Ok(Url {
        original: url.to_owned(),
        secure,
        authority: authority.to_owned(),
        host,
        port,
        path,
        query,
    })
}

impl Url {
    /// The URL as handed to us.
    fn full(&self) -> &str {
        &self.original
    }

    /// Resolves the SDP's `a=control` reference against this URL.
    ///
    /// Absolute control URLs are used as-is. Relative ones are joined onto the path,
    /// and the query is kept: Wowza's signed URLs must carry their token on every
    /// request.
    fn resolve_control(&self, control: &str) -> String {
        if control.starts_with("rtsp://") || control.starts_with("rtsps://") {
            return control.to_owned();
        }
        let scheme = if self.secure { "rtsps" } else { "rtsp" };
        let path = control
            .strip_prefix('/')
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{}/{}", self.path, control));
        let mut url = format!("{scheme}://{}/{}", self.authority, path);
        if let Some(query) = &self.query {
            url.push('?');
            url.push_str(query);
        }
        url
    }
}

impl RtspSession {
    /// Connects, then `DESCRIBE`s, `SETUP`s the video track and `PLAY`s.
    pub async fn connect(url: &str, timeout: Duration) -> ApiResult<Self> {
        let parsed = parse_url(url)?;
        let connect = async {
            let tcp = TcpStream::connect((parsed.host.as_str(), parsed.port))
                .await
                .map_err(|e| {
                    ApiError::new("offline", format!("the live stream is unreachable: {e}"))
                })?;
            tcp.set_nodelay(true).ok();
            if parsed.secure {
                let tls = TlsConnector::from(super::tls_config()?);
                let name = rustls::pki_types::ServerName::try_from(parsed.host.clone())
                    .map_err(|_| ApiError::invalid("the live stream URL has a bad host name"))?;
                let stream = tls.connect(name, tcp).await.map_err(|e| {
                    ApiError::new("offline", format!("live stream TLS failed: {e}"))
                })?;
                Ok(TlsStream::Client(stream))
            } else {
                Err(ApiError::internal(
                    "plain RTSP is not used by the Qubo cloud",
                ))
            }
        };
        let stream = tokio::time::timeout(timeout, connect).await.map_err(|_| {
            ApiError::new(
                "offline",
                "The Qubo live stream didn't answer in time. Try again.",
            )
        })??;
        let mut session = Self {
            stream,
            buffer: BytesMut::new(),
            cseq: 0,
            session: None,
            parameter_sets: Vec::new(),
            codec: String::new(),
            aggregate_url: url.to_owned(),
            video_channel: 0,
            keepalive_at: tokio::time::Instant::now() + Duration::from_secs(20),
        };

        let describe = session
            .request("DESCRIBE", parsed.full(), &[("Accept", "application/sdp")])
            .await?;
        if describe.0 != 200 {
            return Err(session_error("DESCRIBE", describe.0));
        }
        let sdp = parse_sdp(&describe.2);
        if !matches!(sdp.codec.as_deref(), Some("H264" | "H265")) {
            return Err(ApiError::new(
                "unsupported",
                "The Qubo relay must provide H.264 or H.265 video.",
            ));
        }
        let Some(control) = &sdp.video_control else {
            return Err(ApiError::internal("the Qubo stream has no video track"));
        };
        let base = describe
            .1
            .get("content-base")
            .map(|base| parse_url(base))
            .transpose()?
            .unwrap_or(parsed);
        let control = base.resolve_control(control);
        session.parameter_sets = sdp.sprop_parameter_sets;
        session.codec = sdp.codec.unwrap_or_default();

        let setup = session
            .request(
                "SETUP",
                &control,
                &[("Transport", "RTP/AVP/TCP;unicast;interleaved=0-1")],
            )
            .await?;
        if setup.0 != 200 {
            return Err(session_error("SETUP", setup.0));
        }
        session.session = setup
            .1
            .get("session")
            .map(|s| s.split(';').next().unwrap_or_default().trim().to_owned());
        if let Some(transport) = setup.1.get("transport") {
            session.video_channel = transport
                .split(';')
                .find_map(|part| part.trim().strip_prefix("interleaved="))
                .and_then(|channels| channels.split('-').next()?.parse().ok())
                .unwrap_or(0);
        }
        if session.session.is_none() {
            return Err(ApiError::internal(
                "The Qubo relay did not establish a session.",
            ));
        }

        let session_header = session.session.clone();
        let mut play_headers: Vec<(&str, &str)> = Vec::new();
        if let Some(session_id) = &session_header {
            play_headers.push(("Session", session_id.as_str()));
        }
        play_headers.push(("Range", "npt=0.000-"));
        let play = session.request("PLAY", url, &play_headers).await?;
        if play.0 != 200 {
            return Err(session_error("PLAY", play.0));
        }
        Ok(session)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> RtspSession<S> {
    /// Writes a request and reads its response: `(status, headers, body)`.
    async fn request(
        &mut self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
    ) -> ApiResult<(u16, HashMap<String, String>, String)> {
        self.cseq += 1;
        let mut request = format!("{method} {url} RTSP/1.0\r\nCSeq: {}\r\n", self.cseq);
        request.push_str("User-Agent: Backsight\r\n");
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        self.stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| ApiError::new("offline", stream_message(e)))?;
        let response = self.read_http_like().await?;
        if let Some(cseq) = response.1.get("cseq")
            && cseq.trim() != self.cseq.to_string()
        {
            return Err(ApiError::internal(format!(
                "the live stream replied with CSeq {cseq:?} instead of {}",
                self.cseq
            )));
        }
        Ok(response)
    }

    /// Reads a status line, headers and body from the socket.
    ///
    /// Also used mid-stream to skip responses the server sends on its own (for
    /// example keepalive acknowledgements).
    async fn read_http_like(&mut self) -> ApiResult<(u16, HashMap<String, String>, String)> {
        let head_end = loop {
            if let Some(i) = find_crlfcrlf(&self.buffer) {
                break i;
            }
            if self.buffer.len() > 64 * 1024 {
                return Err(ApiError::internal("RTSP reply headers are too large."));
            }
            if self.read_more().await? == 0 {
                return Err(ApiError::new("offline", "the live stream closed mid-reply"));
            }
        };
        let head = String::from_utf8_lossy(&self.buffer[..head_end]).into_owned();
        self.buffer.advance(head_end + 4);
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .and_then(|line| line.split(' ').nth(1))
            .and_then(|code| code.parse().ok())
            .ok_or_else(|| ApiError::internal("the live stream sent a malformed reply"))?;
        let mut headers = HashMap::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
            }
        }
        let mut body = String::new();
        if let Some(len) = headers
            .get("content-length")
            .and_then(|v| v.parse::<usize>().ok())
        {
            if len > MAX_REPLY_BYTES {
                return Err(ApiError::internal("RTSP reply body is too large."));
            }
            while self.buffer.len() < len {
                if self.read_more().await? == 0 {
                    return Err(ApiError::new(
                        "offline",
                        "The live stream closed mid-reply.",
                    ));
                }
            }
            let take = len.min(self.buffer.len());
            body = String::from_utf8_lossy(&self.buffer[..take]).into_owned();
            self.buffer.advance(take);
        }
        Ok((status, headers, body))
    }

    /// Reads more bytes from the socket into the buffer.
    async fn read_more(&mut self) -> ApiResult<usize> {
        let mut chunk = [0u8; 16 * 1024];
        let n = self
            .stream
            .read(&mut chunk)
            .await
            .map_err(|e| ApiError::new("offline", stream_message(e)))?;
        self.buffer.extend_from_slice(&chunk[..n]);
        Ok(n)
    }

    /// The next RTP packet of the video track, or `None` when the stream ended.
    ///
    /// RTCP and audio packets are skipped; server-initiated RTSP responses are
    /// consumed.
    pub async fn next_packet(&mut self) -> ApiResult<Option<RtpPacket>> {
        loop {
            if tokio::time::Instant::now() >= self.keepalive_at {
                self.cseq += 1;
                let request = format!(
                    "OPTIONS {} RTSP/1.0\r\nCSeq: {}\r\nSession: {}\r\n\r\n",
                    self.aggregate_url,
                    self.cseq,
                    self.session.as_deref().unwrap_or_default()
                );
                self.stream
                    .write_all(request.as_bytes())
                    .await
                    .map_err(|e| ApiError::new("offline", stream_message(e)))?;
                self.keepalive_at = tokio::time::Instant::now() + Duration::from_secs(20);
            }
            if self.buffer.first() == Some(&b'$') {
                if self.buffer.len() < 4 {
                    if self.read_more().await? == 0 {
                        return Ok(None);
                    }
                    continue;
                }
                let channel = self.buffer[1];
                let len = u16::from_be_bytes([self.buffer[2], self.buffer[3]]) as usize;
                if self.buffer.len() < 4 + len {
                    if self.read_more().await? == 0 {
                        return Ok(None);
                    }
                    continue;
                }
                self.buffer.advance(4);
                let data = self.buffer.split_to(len);
                if channel != self.video_channel {
                    continue;
                }
                return parse_rtp(data.freeze()).map(Some);
            }
            if self.buffer.first() == Some(&b'R') {
                // "RTSP/1.0 …": a response sent outside a request.
                let (_, _, _) = self.read_http_like().await?;
                continue;
            }
            // Nothing recognisable yet: read more.
            if !self.buffer.is_empty() {
                return Err(ApiError::internal(
                    "Unexpected data on the RTSP connection.",
                ));
            }
            if self.read_more().await? == 0 {
                return Ok(None);
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> RtspSession<S> {
    /// Ends the session with the server. The TCP close that follows also works, but
    /// an explicit `TEARDOWN` lets the cloud drop the relay promptly.
    pub async fn teardown(&mut self) {
        if let Some(session) = self.session.take() {
            self.cseq += 1;
            let request = format!(
                "TEARDOWN {} RTSP/1.0\r\nCSeq: {}\r\nSession: {session}\r\n\r\n",
                self.aggregate_url, self.cseq
            );
            if let Err(err) = self.stream.write_all(request.as_bytes()).await {
                tracing::debug!(%err, "live stream TEARDOWN failed");
            }
        }
    }
}

fn find_crlfcrlf(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|w| w == b"\r\n\r\n")
}

fn stream_message(err: std::io::Error) -> String {
    format!("the live stream connection failed: {err}")
}

fn session_error(step: &str, status: u16) -> ApiError {
    if status == 401 || status == 403 {
        ApiError::new(
            "auth_failed",
            "The Qubo cloud refused the live stream. Sign in again.",
        )
    } else {
        ApiError::internal(format!("{step} failed with status {status}"))
    }
}

/// Parses the RTP header and returns the payload.
fn parse_rtp(data: Bytes) -> ApiResult<RtpPacket> {
    let Some(&head) = data.first() else {
        return Err(ApiError::internal("empty RTP packet"));
    };
    if head >> 6 != 2 {
        return Err(ApiError::internal("the live stream is not RTP version 2"));
    }
    let padding = head & 0x20 != 0;
    let extension = head & 0x10 != 0;
    let csrc_count = (head & 0x0F) as usize;
    let Some(&second) = data.get(1) else {
        return Err(ApiError::internal("short RTP packet"));
    };
    let marker = second & 0x80 != 0;
    let mut offset = 12 + 4 * csrc_count;
    if extension {
        let Some(ext_len) = data
            .get(offset + 2..offset + 4)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
        else {
            return Err(ApiError::internal("short RTP extension header"));
        };
        offset += 4 + 4 * ext_len as usize;
    }
    let Some(sequence) = data.get(2..4).map(|b| u16::from_be_bytes([b[0], b[1]])) else {
        return Err(ApiError::internal("short RTP packet"));
    };
    let timestamp = u32::from_be_bytes(
        data.get(4..8)
            .map(|b| [b[0], b[1], b[2], b[3]])
            .ok_or_else(|| ApiError::internal("short RTP timestamp"))?,
    );
    if offset > data.len() {
        return Err(ApiError::internal("RTP header exceeds the packet"));
    }
    let mut payload = data.slice(offset..);
    if padding {
        let Some(&pad) = payload.last() else {
            return Err(ApiError::internal("RTP padding without length"));
        };
        let pad = pad as usize;
        if pad == 0 || pad > payload.len() {
            return Err(ApiError::internal("RTP padding exceeds the packet"));
        }
        payload.truncate(payload.len() - pad);
    }
    Ok(RtpPacket {
        timestamp,
        marker,
        sequence,
        payload,
    })
}

/// Reads the video track's control URL and parameter sets out of a session's SDP.
///
/// Only the first `m=video` section is used; `m=audio` sections are skipped.
pub fn parse_sdp(text: &str) -> Sdp {
    use base64::Engine as _;

    let mut sdp = Sdp::default();
    let mut in_video = false;
    let mut video_seen = false;
    for line in text.lines() {
        let (key, value) = match line.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        match key {
            "m" => {
                in_video = value.trim().starts_with("video") && !video_seen;
                if in_video {
                    video_seen = true;
                }
            }
            "a" if in_video => {
                if let Some(control) = value.strip_prefix("control:") {
                    sdp.video_control = Some(control.trim().to_owned());
                } else if let Some(rtpmap) = value.strip_prefix("rtpmap:") {
                    sdp.codec = rtpmap
                        .split_whitespace()
                        .nth(1)
                        .and_then(|format| format.split('/').next())
                        .map(str::to_owned);
                } else if let Some(fmtp) = value.strip_prefix("fmtp:") {
                    let fmtp = fmtp
                        .split_once(' ')
                        .map(|(_, params)| params)
                        .unwrap_or(fmtp);
                    for (name, value) in fmtp.split(';').filter_map(|pair| pair.split_once('=')) {
                        if matches!(
                            name.trim(),
                            "sprop-parameter-sets" | "sprop-vps" | "sprop-sps" | "sprop-pps"
                        ) {
                            sdp.sprop_parameter_sets.extend(
                                value
                                    .split(',')
                                    .filter(|s| !s.is_empty())
                                    .filter_map(|s| {
                                        base64::engine::general_purpose::STANDARD
                                            .decode(s.trim())
                                            .ok()
                                    })
                                    .collect::<Vec<_>>(),
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
    sdp
}

#[cfg(test)]
mod tests {
    use super::*;

    const SDP: &str = "v=0\r
o=- 123 1 IN IP4 10.0.0.1\r
s=QuboLive\r
t=0 0\r
m=video 0 RTP/AVP 96\r
c=IN IP4 0.0.0.0\r
a=rtpmap:96 H264/90000\r
a=fmtp:96 packetization-mode=1;profile-level-id=64001F;sprop-parameter-sets=Z2QAHqwrUFgLAA==,aP6gLA==\r
a=control:trackID=1\r
m=audio 0 RTP/AVP 97\r
a=rtpmap:97 MPEG4-GENERIC/16000/1\r
a=control:trackID=2\r
";

    #[test]
    fn sdp_finds_the_video_track() {
        let sdp = parse_sdp(SDP);
        assert_eq!(sdp.video_control.as_deref(), Some("trackID=1"));
        assert_eq!(sdp.sprop_parameter_sets.len(), 2);
        // Z2QAHqwrUFgLAA== decodes to an SPS: forbidden_zero(0), nal_ref_idc, type 7.
        assert_eq!(sdp.sprop_parameter_sets[0][0] & 0x1F, 7);
        // aP6gLA== decodes to a PPS: type 8.
        assert_eq!(sdp.sprop_parameter_sets[1][0] & 0x1F, 8);
    }

    #[test]
    fn sdp_without_video_is_empty() {
        let sdp = parse_sdp("v=0\r\nm=audio 0 RTP/AVP 97\r\na=control:trackID=2\r\n");
        assert_eq!(sdp, Sdp::default());
    }

    #[test]
    fn urls_parse() {
        let url =
            parse_url("rtsps://wowza.platform.quboworld.com:443/live/abc?wowzatoken=1").unwrap();
        assert!(url.secure);
        assert_eq!(url.host, "wowza.platform.quboworld.com");
        assert_eq!(url.port, 443);
        assert_eq!(url.path, "live/abc");
        assert_eq!(url.query.as_deref(), Some("wowzatoken=1"));
        // The URL is used exactly as handed to us.
        assert_eq!(
            url.full(),
            "rtsps://wowza.platform.quboworld.com:443/live/abc?wowzatoken=1"
        );
    }

    #[test]
    fn control_urls_resolve_with_the_token_kept() {
        let url = parse_url("rtsps://host:443/app/stream?wowzatoken=1").unwrap();
        assert_eq!(
            url.resolve_control("trackID=1"),
            "rtsps://host:443/app/stream/trackID=1?wowzatoken=1"
        );
        assert_eq!(
            url.resolve_control("/other/path"),
            "rtsps://host:443/other/path?wowzatoken=1"
        );
        assert_eq!(
            url.resolve_control("rtsps://other:1935/abs"),
            "rtsps://other:1935/abs"
        );
    }

    #[test]
    fn rtp_parses_with_the_marker_bit() {
        // Header: V=2, no padding, no extension, 0 CSRC; M=1, PT=96.
        let mut data = vec![0x80, 0xE0, 0x12, 0x34, 0, 0, 1, 0, 0, 0, 0, 9];
        data.extend_from_slice(b"payload");
        let packet = parse_rtp(Bytes::from(data)).unwrap();
        assert!(packet.marker);
        assert_eq!(packet.sequence, 0x1234);
        assert_eq!(packet.timestamp, 256);
        assert_eq!(packet.payload, Bytes::from_static(b"payload"));
    }

    #[test]
    fn rtp_skips_extensions_and_padding() {
        // V=2, P=1, X=1, 1 CSRC; M=0; then CSRC, extension (profile, len 1 word).
        let mut data = vec![
            0xB1, 0x60, 0, 1, 0, 0, 2, 0, 0, 0, 0, 9, 0x11, 0x22, 0x33, 0x44,
        ];
        // Extension: profile 2 bytes, length 1 (= 4 bytes), 4 bytes of data.
        data.extend_from_slice(&[0, 1, 0, 1, 0xAA, 0xBB, 0xCC, 0xDD]);
        data.extend_from_slice(b"abcd");
        // Three padding bytes, the last counting them.
        data.extend_from_slice(&[1, 2, 3]);
        let packet = parse_rtp(Bytes::from(data)).unwrap();
        assert!(!packet.marker);
        assert_eq!(packet.payload, Bytes::from_static(b"abcd"));
    }

    #[test]
    fn rtp_rejects_bad_versions() {
        assert!(
            parse_rtp(Bytes::from_static(&[
                0x40, 0xE0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 9
            ]))
            .is_err()
        );
    }

    fn session(stream: tokio::io::DuplexStream) -> RtspSession<tokio::io::DuplexStream> {
        RtspSession {
            stream,
            buffer: BytesMut::new(),
            cseq: 0,
            session: None,
            parameter_sets: Vec::new(),
            codec: "H264".into(),
            aggregate_url: "rtsps://test/live".into(),
            video_channel: 0,
            keepalive_at: tokio::time::Instant::now() + Duration::from_secs(20),
        }
    }

    #[tokio::test]
    async fn eof_after_a_partial_frame_ends_the_stream() {
        let (client, mut server) = tokio::io::duplex(64);
        server.write_all(&[b'$', 0, 0, 20, 0x80]).await.unwrap();
        server.shutdown().await.unwrap();
        let mut session = session(client);
        let packet = tokio::time::timeout(Duration::from_millis(100), session.next_packet())
            .await
            .unwrap()
            .unwrap();
        assert!(packet.is_none());
    }

    #[tokio::test]
    async fn rtcp_is_not_mistaken_for_video() {
        let (client, mut server) = tokio::io::duplex(128);
        server
            .write_all(&[b'$', 1, 0, 4, 0x80, 200, 0, 0])
            .await
            .unwrap();
        server
            .write_all(&[
                b'$', 0, 0, 13, 0x80, 0xe0, 0, 1, 0, 0, 0, 5, 0, 0, 0, 1, 0x65,
            ])
            .await
            .unwrap();
        let mut session = session(client);
        assert_eq!(session.next_packet().await.unwrap().unwrap().timestamp, 5);
    }
}
