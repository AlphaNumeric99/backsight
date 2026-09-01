//! Finding Tapo devices on the local network.
//!
//! Uses TP-Link's UDP discovery protocol (port 20002). A device answers a discovery query
//! with its model, MAC address and — most usefully — which login scheme it speaks, all
//! without any credentials.
//!
//! The query format is adapted from the `tapo` crate by Mihai Dinculescu (MIT).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde_json::Value;
use tokio::net::UdpSocket;
use tokio::time::Instant;

/// UDP port devices listen on for discovery queries.
pub const DISCOVERY_PORT: u16 = 20002;

/// How often the query is re-sent while waiting for replies.
const RESEND_INTERVAL: Duration = Duration::from_millis(1000);

/// A throwaway RSA public key sent with every query. Devices use it to encrypt the
/// `encrypt_info` part of their reply, which we never read, so no private key exists.
const QUERY_RSA_PUBLIC_KEY: &str = "-----BEGIN RSA PUBLIC KEY-----
MIGJAoGBANiUhXhLQaof/r1e6nwWlwvTjmpKlwEyOW/stnsw+ogHdkARISFD4WNe
2gv+AVnI6j3/vq+O6vw9KaoXfAihSNXm90tkWZRK6o5bhfW63wWtusI6mPLnIMnJ
ua7JmlbxLV9KTFCdUJOhq5CKMz9KRGd067fXTihW4GPQf9Ji5cqtAgMBAAE=
-----END RSA PUBLIC KEY-----
";

/// What kind of Tapo device answered.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Camera,
    Doorbell,
    Hub,
    /// Any other device type, with the raw type string (e.g. `SMART.TAPOPLUG`).
    Other(String),
}

impl DeviceKind {
    fn from_device_type(device_type: &str) -> Self {
        match device_type {
            "SMART.IPCAMERA" => Self::Camera,
            "SMART.TAPODOORBELL" => Self::Doorbell,
            "SMART.TAPOHUB" => Self::Hub,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// Which login scheme a device advertises. Decides the transport used to talk to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoginScheme {
    /// `encrypt_type` 3: nonce-based "secure connection" over HTTPS (2023+ firmware).
    Secure,
    /// `encrypt_type` 4 / TPAP: SPAKE2+ login (2026 firmware).
    Tpap,
    /// Nothing advertised: older firmware, or the device didn't say.
    Unknown,
}

/// One device that replied to discovery.
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub ip: IpAddr,
    pub kind: DeviceKind,
    /// Raw type string, e.g. `SMART.IPCAMERA`.
    pub device_type: String,
    /// Model, e.g. `C210`.
    pub model: Option<String>,
    pub mac: Option<String>,
    pub device_id: Option<String>,
    /// `encrypt_type` values advertised by the device, e.g. `["3"]`.
    pub encrypt_types: Vec<String>,
    pub supports_https: Option<bool>,
    pub http_port: Option<u16>,
    /// Present when the device speaks TPAP; describes its parameters.
    pub tpap: Option<Value>,
    pub factory_default: Option<bool>,
    /// The full `result` object, for diagnostics.
    pub raw: Value,
}

impl DiscoveredDevice {
    /// The login scheme to try first, based on what the device advertised.
    pub fn login_scheme(&self) -> LoginScheme {
        if self.tpap.is_some() || self.encrypt_types.iter().any(|t| t == "4") {
            LoginScheme::Tpap
        } else if self.encrypt_types.iter().any(|t| t == "3") {
            LoginScheme::Secure
        } else {
            LoginScheme::Unknown
        }
    }
}

/// Builds a discovery query datagram.
///
/// Layout (big-endian): version `2`, message type `0`, op code `1`, payload length,
/// flags `0x11`, padding, a random serial, and a CRC32 of the whole datagram computed
/// with `0x5A6B7C8D` in the CRC field; followed by the JSON payload.
pub fn build_query(serial: u32) -> Vec<u8> {
    let payload = serde_json::json!({ "params": { "rsa_key": QUERY_RSA_PUBLIC_KEY } });
    let payload = serde_json::to_vec(&payload).expect("static JSON serializes");

    let mut query = Vec::with_capacity(16 + payload.len());
    query.push(2); // version
    query.push(0); // message type
    query.extend_from_slice(&1u16.to_be_bytes()); // op code
    query.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    query.push(0x11); // flags
    query.push(0); // padding
    query.extend_from_slice(&serial.to_be_bytes());
    query.extend_from_slice(&0x5A6B_7C8Du32.to_be_bytes());
    query.extend_from_slice(&payload);

    let crc = crc32fast::hash(&query);
    query[12..16].copy_from_slice(&crc.to_be_bytes());
    query
}

/// Parses a discovery reply. Returns `None` for anything that isn't a device reply.
pub fn parse_reply(ip: IpAddr, datagram: &[u8]) -> Option<DiscoveredDevice> {
    let json: Value = serde_json::from_slice(datagram.get(16..)?).ok()?;
    let result = json.get("result")?;
    let text = |key: &str| result.get(key).and_then(Value::as_str).map(str::to_owned);

    let device_type = text("device_type")?;
    let encrypt_types = match result.get("encrypt_type") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    };
    let scheme = result.get("mgt_encrypt_schm");

    Some(DiscoveredDevice {
        ip,
        kind: DeviceKind::from_device_type(&device_type),
        device_type,
        model: text("device_model"),
        mac: text("mac"),
        device_id: text("device_id"),
        encrypt_types,
        supports_https: scheme
            .and_then(|s| s.get("is_support_https"))
            .and_then(Value::as_bool),
        http_port: scheme
            .and_then(|s| s.get("http_port"))
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok()),
        tpap: result.get("tpap").filter(|v| !v.is_null()).cloned(),
        factory_default: result.get("factory_default").and_then(Value::as_bool),
        raw: result.clone(),
    })
}

/// Broadcasts discovery queries on every local IPv4 network and collects the replies
/// that arrive within `timeout`. Each device appears once.
pub async fn discover(timeout: Duration) -> std::io::Result<Vec<DiscoveredDevice>> {
    let mut targets = vec![Ipv4Addr::BROADCAST];
    targets.extend(interface_broadcasts());
    targets.dedup();
    collect(&targets, timeout, None).await
}

/// Queries a single address directly. Useful when broadcasts don't reach the device
/// (other subnet, VPN) or to identify a camera the user typed in.
pub async fn probe(ip: Ipv4Addr, timeout: Duration) -> std::io::Result<Option<DiscoveredDevice>> {
    let mut found = collect(&[ip], timeout, Some(IpAddr::V4(ip))).await?;
    Ok(found.pop())
}

async fn collect(
    targets: &[Ipv4Addr],
    timeout: Duration,
    stop_at: Option<IpAddr>,
) -> std::io::Result<Vec<DiscoveredDevice>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    socket.set_broadcast(true)?;
    let query = build_query(rand_serial());

    let deadline = Instant::now() + timeout;
    let mut next_send = Instant::now();
    let mut found: HashMap<IpAddr, DiscoveredDevice> = HashMap::new();
    let mut buf = vec![0u8; 4096];

    while Instant::now() < deadline {
        if Instant::now() >= next_send {
            for target in targets {
                // Unreachable networks are normal on multi-homed machines; keep going.
                if let Err(err) = socket.send_to(&query, (*target, DISCOVERY_PORT)).await {
                    tracing::debug!(%target, %err, "discovery send failed");
                }
            }
            next_send = Instant::now() + RESEND_INTERVAL;
        }

        let wait = next_send
            .min(deadline)
            .saturating_duration_since(Instant::now());
        match tokio::time::timeout(wait, socket.recv_from(&mut buf)).await {
            Ok(Ok((len, SocketAddr::V4(from)))) => {
                let ip = IpAddr::V4(*from.ip());
                if let Some(device) = parse_reply(ip, &buf[..len]) {
                    tracing::debug!(%ip, device_type = %device.device_type, "discovered");
                    found.entry(ip).or_insert(device);
                    if stop_at == Some(ip) {
                        break;
                    }
                }
            }
            Ok(Ok(_)) => {}
            // Windows reports ICMP "port unreachable" from earlier sends as a receive
            // error (WSAECONNRESET); it says nothing about other devices.
            Ok(Err(err)) if err.kind() == std::io::ErrorKind::ConnectionReset => {}
            Ok(Err(err)) => return Err(err),
            Err(_elapsed) => {}
        }
    }

    let mut devices: Vec<_> = found.into_values().collect();
    devices.sort_by_key(|d| d.ip);
    Ok(devices)
}

/// Directed broadcast addresses of the machine's IPv4 interfaces (e.g. `192.168.1.255`).
fn interface_broadcasts() -> Vec<Ipv4Addr> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    interfaces
        .into_iter()
        .filter(|iface| !iface.is_loopback())
        .filter_map(|iface| match iface.addr {
            if_addrs::IfAddr::V4(v4) => v4.broadcast.or_else(|| {
                let ip = u32::from(v4.ip);
                let mask = u32::from(v4.netmask);
                (mask != 0).then(|| Ipv4Addr::from(ip | !mask))
            }),
            if_addrs::IfAddr::V6(_) => None,
        })
        .collect()
}

fn rand_serial() -> u32 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(Instant::now()) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_header_and_crc() {
        let query = build_query(0x0102_0304);
        assert_eq!(&query[..4], &[2, 0, 0, 1]);
        let payload_len = u16::from_be_bytes([query[4], query[5]]) as usize;
        assert_eq!(query.len(), 16 + payload_len);
        assert_eq!(query[6], 0x11);
        assert_eq!(&query[8..12], &[1, 2, 3, 4]);

        // The CRC covers the datagram with the placeholder in the CRC field.
        let mut check = query.clone();
        check[12..16].copy_from_slice(&0x5A6B_7C8Du32.to_be_bytes());
        assert_eq!(
            u32::from_be_bytes(query[12..16].try_into().unwrap()),
            crc32fast::hash(&check)
        );

        let json: Value = serde_json::from_slice(&query[16..]).unwrap();
        assert!(
            json["params"]["rsa_key"]
                .as_str()
                .unwrap()
                .starts_with("-----BEGIN RSA PUBLIC KEY-----")
        );
    }

    fn reply(result: Value) -> Vec<u8> {
        let mut datagram = vec![0u8; 16];
        datagram.extend(
            serde_json::to_vec(&serde_json::json!({ "error_code": 0, "result": result })).unwrap(),
        );
        datagram
    }

    #[test]
    fn parses_camera_reply() {
        let datagram = reply(serde_json::json!({
            "device_type": "SMART.IPCAMERA",
            "device_model": "C210",
            "mac": "AA-BB-CC-DD-EE-FF",
            "device_id": "abc",
            "encrypt_type": ["3"],
            "mgt_encrypt_schm": { "is_support_https": true, "http_port": 443 },
            "factory_default": false
        }));
        let ip: IpAddr = "192.168.1.20".parse().unwrap();
        let device = parse_reply(ip, &datagram).unwrap();
        assert_eq!(device.kind, DeviceKind::Camera);
        assert_eq!(device.model.as_deref(), Some("C210"));
        assert_eq!(device.http_port, Some(443));
        assert_eq!(device.supports_https, Some(true));
        assert_eq!(device.login_scheme(), LoginScheme::Secure);
    }

    #[test]
    fn detects_tpap() {
        let datagram = reply(serde_json::json!({
            "device_type": "SMART.IPCAMERA",
            "encrypt_type": ["4"],
            "tpap": { "pake": [2], "tls": 1, "port": 443 }
        }));
        let device = parse_reply("10.0.0.5".parse().unwrap(), &datagram).unwrap();
        assert_eq!(device.login_scheme(), LoginScheme::Tpap);
    }

    #[test]
    fn ignores_garbage() {
        assert!(parse_reply("10.0.0.5".parse().unwrap(), b"short").is_none());
        assert!(parse_reply("10.0.0.5".parse().unwrap(), &[0u8; 40]).is_none());
    }
}
