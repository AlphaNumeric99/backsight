//! A fake Tapo camera for integration tests and UI development.
//!
//! It implements the camera side of the protocols `tapo-camera` speaks: the HTTPS control
//! API with the secure-connection login (including lockout after failed logins), and the
//! port-8800 media server that streams a fixture MPEG-TS file for live view, playback
//! and downloads, and serves a thumbnail for detection recordings.
//!
//! Recordings and detection events are generated deterministically for the last two
//! weeks, in the machine's local time zone.

mod control;
mod crypto;
mod data;
mod media;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub use data::Fixture;

/// How the fake camera behaves.
#[derive(Debug, Clone)]
pub struct MockOptions {
    pub ip: IpAddr,
    /// 443 on real cameras; 0 picks a free port.
    pub control_port: u16,
    /// 8800 on real cameras; 0 picks a free port.
    pub media_port: u16,
    /// The "TP-Link account password" the camera accepts.
    pub password: String,
    pub name: String,
    pub model: String,
    pub fixture: Arc<Fixture>,
}

impl MockOptions {
    pub fn new(password: impl Into<String>) -> Self {
        Self {
            ip: IpAddr::from([127, 0, 0, 1]),
            control_port: 0,
            media_port: 0,
            password: password.into(),
            name: "Mock Camera".into(),
            model: "C200".into(),
            fixture: Arc::new(Fixture::builtin()),
        }
    }
}

/// A running fake camera. Stops when dropped.
pub struct MockCamera {
    pub control_addr: SocketAddr,
    pub media_addr: SocketAddr,
    tasks: Vec<JoinHandle<()>>,
}

impl MockCamera {
    pub async fn start(options: MockOptions) -> anyhow::Result<Self> {
        let options = Arc::new(options);
        let control = TcpListener::bind((options.ip, options.control_port)).await?;
        let media = TcpListener::bind((options.ip, options.media_port)).await?;
        let control_addr = control.local_addr()?;
        let media_addr = media.local_addr()?;

        let tasks = vec![
            tokio::spawn(control::serve(control, options.clone())),
            tokio::spawn(media::serve(media, options)),
        ];
        Ok(Self {
            control_addr,
            media_addr,
            tasks,
        })
    }
}

impl Drop for MockCamera {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
