//! Camera previews for the Home cards and player posters: the latest frame the player showed
//! for each camera, saved by the UI as a small JPEG in the cache directory.
//!
//! Served through the thumbnail scheme at `/preview/<camera>/<saved_at_ms>`. The timestamp in
//! the path changes with every save, so the webview never shows a stale cached copy.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{ApiError, ApiResult};
use crate::thumbnails::SCHEME;

/// Larger bodies are refused; a 640-pixel-wide JPEG is around 50 KB.
const MAX_BYTES: usize = 2 * 1024 * 1024;

pub struct Previews {
    dir: PathBuf,
    /// Camera id → when its preview was saved, in milliseconds since the Unix epoch.
    saved: Mutex<HashMap<String, i64>>,
}

impl Previews {
    /// Indexes the previews already on disk.
    pub fn new(dir: PathBuf) -> Self {
        let mut saved = HashMap::new();
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let Some(id) = camera_id_of(&entry.path()) else {
                continue;
            };
            let modified = entry.metadata().and_then(|m| m.modified()).ok();
            if let Some(ms) = modified
                .and_then(|t| jiff::Timestamp::try_from(t).ok())
                .map(|t| t.as_millisecond())
            {
                saved.insert(id, ms);
            }
        }
        Self {
            dir,
            saved: Mutex::new(saved),
        }
    }

    /// When the camera's preview was saved, if it has one.
    pub fn saved_at(&self, camera_id: &str) -> Option<i64> {
        self.saved
            .lock()
            .expect("previews lock")
            .get(camera_id)
            .copied()
    }

    /// Stores `jpeg` as the camera's preview and returns when it was saved.
    pub fn save(&self, camera_id: &str, jpeg: &[u8]) -> ApiResult<i64> {
        if !is_camera_id(camera_id) {
            return Err(ApiError::invalid("invalid camera id"));
        }
        if jpeg.len() > MAX_BYTES {
            return Err(ApiError::invalid("the preview is too large"));
        }
        if !jpeg.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Err(ApiError::invalid("the preview is not a JPEG image"));
        }
        std::fs::create_dir_all(&self.dir).map_err(ApiError::internal)?;
        let path = self.path(camera_id);
        let part = path.with_extension("jpg.part");
        std::fs::write(&part, jpeg).map_err(ApiError::internal)?;
        std::fs::rename(&part, &path).map_err(ApiError::internal)?;
        let ms = jiff::Timestamp::now().as_millisecond();
        self.saved
            .lock()
            .expect("previews lock")
            .insert(camera_id.to_owned(), ms);
        Ok(ms)
    }

    pub fn read(&self, camera_id: &str) -> Option<Vec<u8>> {
        is_camera_id(camera_id)
            .then(|| std::fs::read(self.path(camera_id)).ok())
            .flatten()
    }

    pub fn remove(&self, camera_id: &str) {
        self.saved.lock().expect("previews lock").remove(camera_id);
        if is_camera_id(camera_id) {
            let _ = std::fs::remove_file(self.path(camera_id));
        }
    }

    fn path(&self, camera_id: &str) -> PathBuf {
        self.dir.join(format!("{camera_id}.jpg"))
    }
}

/// The URL the webview loads a preview from.
pub fn url(camera_id: &str, saved_at_ms: i64) -> String {
    if cfg!(windows) {
        format!("http://{SCHEME}.localhost/preview/{camera_id}/{saved_at_ms}")
    } else {
        format!("{SCHEME}://localhost/preview/{camera_id}/{saved_at_ms}")
    }
}

/// Parses the path part of a preview URL, `/preview/<camera_id>/<saved_at_ms>`, into the
/// camera id.
pub fn parse_path(path: &str) -> Option<String> {
    let mut parts = path.trim_start_matches('/').split('/');
    if parts.next()? != "preview" {
        return None;
    }
    let camera = parts.next().filter(|c| is_camera_id(c))?;
    parts.next()?.parse::<i64>().ok()?;
    parts.next().is_none().then(|| camera.to_owned())
}

/// Camera ids are short alphanumeric strings; anything else never touches the file system.
fn is_camera_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

fn camera_id_of(path: &Path) -> Option<String> {
    if path.extension()? != "jpg" {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    is_camera_id(stem).then(|| stem.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F'];

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("backsight-previews-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn save_read_and_remove() {
        let dir = temp_dir("roundtrip");
        let previews = Previews::new(dir.clone());
        assert_eq!(previews.saved_at("c1"), None);

        let at = previews.save("c1", JPEG).unwrap();
        assert_eq!(previews.saved_at("c1"), Some(at));
        assert_eq!(previews.read("c1").as_deref(), Some(JPEG));

        // A new instance finds it on disk.
        assert!(Previews::new(dir.clone()).saved_at("c1").is_some());

        previews.remove("c1");
        assert_eq!(previews.saved_at("c1"), None);
        assert_eq!(previews.read("c1"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_bad_input() {
        let dir = temp_dir("reject");
        let previews = Previews::new(dir.clone());
        assert!(previews.save("c1", b"\x89PNG\r\n").is_err());
        assert!(previews.save("../c1", JPEG).is_err());
        assert!(previews.save("c1", &vec![0xFF; MAX_BYTES + 1]).is_err());
        assert_eq!(previews.read("../c1"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn url_round_trip() {
        let url = url("c1", 1_790_000_000_123);
        let path = url.split("localhost").nth(1).unwrap();
        assert_eq!(parse_path(path).as_deref(), Some("c1"));
        assert_eq!(parse_path("/preview/c1"), None);
        assert_eq!(parse_path("/preview/c1/x"), None);
        assert_eq!(parse_path("/preview/c.1/1"), None);
        assert_eq!(parse_path("/c1/123"), None);
        assert_eq!(parse_path("/preview/c1/1/extra"), None);
    }
}
