//! Thumbnails of detection recordings.
//!
//! The camera keeps one JPEG per detection recording and serves it on the media port.
//! Thumbnails are fetched on demand when the UI loads a `thumb://` URL, one camera at a
//! time over a single reused media session, and cached on disk (including "this
//! recording has none", which otherwise costs a full timeout every time).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tapo_camera::stream::{MediaConfig, MediaSession, StreamPart, StreamRequest};
use tokio::sync::{mpsc, oneshot};

use crate::cameras::{CameraHandle, CameraManager};

/// URI scheme the webview loads thumbnails from.
pub const SCHEME: &str = "thumb";

/// How long to wait for a thumbnail before assuming there is none.
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
/// Close the camera session after this long without requests.
const IDLE_CLOSE: Duration = Duration::from_secs(5);

/// The URL for the thumbnail of the recording that starts at `camera_start`
/// (camera-clock seconds).
pub fn url(camera_id: &str, camera_start: i64) -> String {
    if cfg!(windows) {
        format!("http://{SCHEME}.localhost/{camera_id}/{camera_start}")
    } else {
        format!("{SCHEME}://localhost/{camera_id}/{camera_start}")
    }
}

/// Parses the path part of a thumbnail URL: `/<camera_id>/<camera_start>`.
pub fn parse_path(path: &str) -> Option<(String, i64)> {
    let mut parts = path.trim_start_matches('/').split('/');
    let camera = parts
        .next()
        .filter(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_alphanumeric()))?;
    let start = parts.next()?.parse().ok()?;
    parts.next().is_none().then(|| (camera.to_owned(), start))
}

struct Job {
    start: i64,
    reply: oneshot::Sender<Option<Vec<u8>>>,
}

pub struct Thumbnails {
    dir: PathBuf,
    cameras: Arc<CameraManager>,
    player_id: String,
    workers: Mutex<HashMap<String, mpsc::Sender<Job>>>,
}

enum Fetched {
    Image(Vec<u8>),
    /// The image arrived but the session didn't end cleanly; don't reuse it.
    Stale(Vec<u8>),
    /// The camera said there is no thumbnail for this time.
    None,
    /// Couldn't tell (timeout, error): try again later.
    Unknown,
}

impl Thumbnails {
    pub fn new(dir: PathBuf, cameras: Arc<CameraManager>) -> Self {
        let mut id = [0u8; 8];
        getrandom::fill(&mut id).expect("OS random number generator");
        Self {
            dir,
            cameras,
            player_id: format!("backsight-{}", hex::encode(id)),
            workers: Mutex::new(HashMap::new()),
        }
    }

    fn paths(&self, camera_id: &str, start: i64) -> (PathBuf, PathBuf) {
        let dir = self.dir.join(camera_id);
        (
            dir.join(format!("{start}.jpg")),
            dir.join(format!("{start}.none")),
        )
    }

    /// The JPEG for a recording, from the cache or the camera.
    pub async fn get(&self, camera_id: &str, start: i64) -> Option<Vec<u8>> {
        let (image, none) = self.paths(camera_id, start);
        if let Ok(bytes) = tokio::fs::read(&image).await {
            return Some(bytes);
        }
        if tokio::fs::try_exists(&none).await.unwrap_or(false) {
            return None;
        }
        let handle = self.cameras.get(camera_id).ok()?;
        let (reply, answer) = oneshot::channel();
        self.worker(&handle).send(Job { start, reply }).await.ok()?;
        answer.await.ok().flatten()
    }

    fn worker(&self, handle: &Arc<CameraHandle>) -> mpsc::Sender<Job> {
        let mut workers = self.workers.lock().expect("workers lock");
        if let Some(tx) = workers.get(&handle.id).filter(|tx| !tx.is_closed()) {
            return tx.clone();
        }
        let (tx, rx) = mpsc::channel(256);
        workers.insert(handle.id.clone(), tx.clone());
        tauri::async_runtime::spawn(run_worker(
            handle.clone(),
            rx,
            self.dir.join(&handle.id),
            self.player_id.clone(),
        ));
        tx
    }
}

async fn open_session(handle: &CameraHandle) -> Option<MediaSession> {
    let password = handle.cloud_password().ok()?;
    let config = MediaConfig::new(handle.record().host, password);
    match MediaSession::connect(&config).await {
        Ok(session) => Some(session),
        Err(err) => {
            tracing::debug!(camera = %handle.id, %err, "thumbnail session failed");
            None
        }
    }
}

/// Asks for one thumbnail and reads the camera's answer through its "finished"
/// notification, so the session is clean for the next request. (Returning as soon as the
/// JPEG arrived left that notification behind, and the next request mistook it for
/// "no thumbnail".)
async fn fetch(session: &mut MediaSession, request: &StreamRequest) -> Fetched {
    if session.start(request).await.is_err() {
        return Fetched::Unknown;
    }
    let deadline = tokio::time::Instant::now() + FETCH_TIMEOUT;
    let mut image = None;
    loop {
        match tokio::time::timeout_at(deadline, session.next_part()).await {
            Ok(Ok(Some(StreamPart::Other { content_type, data })))
                if content_type == "image/jpeg" =>
            {
                image = Some(data.to_vec());
            }
            Ok(Ok(Some(part))) if part.is_finished() => {
                return image.map_or(Fetched::None, Fetched::Image);
            }
            Ok(Ok(Some(_))) => {}
            // Got the picture but not the end marker: use it, but don't reuse the session.
            _ => return image.map_or(Fetched::Unknown, Fetched::Stale),
        }
    }
}

async fn save(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Err(err) = tokio::fs::write(path, bytes).await {
        tracing::debug!(%err, "could not cache thumbnail");
    }
}

async fn run_worker(
    handle: Arc<CameraHandle>,
    mut rx: mpsc::Receiver<Job>,
    dir: PathBuf,
    player_id: String,
) {
    let mut session: Option<MediaSession> = None;
    loop {
        let job = match tokio::time::timeout(IDLE_CLOSE, rx.recv()).await {
            Ok(Some(job)) => job,
            Ok(None) => return,
            Err(_) => {
                session = None;
                match rx.recv().await {
                    Some(job) => job,
                    None => return,
                }
            }
        };
        let (image_path, none_path) = (
            dir.join(format!("{}.jpg", job.start)),
            dir.join(format!("{}.none", job.start)),
        );
        if let Ok(bytes) = tokio::fs::read(&image_path).await {
            let _ = job.reply.send(Some(bytes));
            continue;
        }

        let Ok(client_id) = handle.user_id(false).await else {
            let _ = job.reply.send(None);
            continue;
        };
        if session.is_none() {
            session = open_session(&handle).await;
        }
        let Some(active) = session.as_mut() else {
            let _ = job.reply.send(None);
            continue;
        };
        let request = StreamRequest::Thumbnail {
            client_id,
            start: job.start,
            player_id: player_id.clone(),
        };
        match fetch(active, &request).await {
            Fetched::Image(bytes) => {
                save(&image_path, &bytes).await;
                let _ = job.reply.send(Some(bytes));
            }
            Fetched::Stale(bytes) => {
                session = None;
                save(&image_path, &bytes).await;
                let _ = job.reply.send(Some(bytes));
            }
            Fetched::None => {
                save(&none_path, b"").await;
                let _ = job.reply.send(None);
            }
            Fetched::Unknown => {
                // The camera may be stuck on this request; start fresh next time.
                session = None;
                let _ = job.reply.send(None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_round_trip() {
        let url = url("a1b2c3", 1_790_000_123);
        let path = url
            .split(".localhost")
            .last()
            .unwrap()
            .split("localhost")
            .last()
            .unwrap();
        assert_eq!(parse_path(path), Some(("a1b2c3".into(), 1_790_000_123)));
        assert_eq!(parse_path("/../etc/1"), None);
        assert_eq!(parse_path("/cam/notanumber"), None);
        assert_eq!(parse_path("/cam/1/extra"), None);
    }
}
