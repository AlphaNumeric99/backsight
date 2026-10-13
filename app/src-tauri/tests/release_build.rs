//! Checks a release executable, if one has been built, for the frontend that has to be
//! inside it.
//!
//! A release build done without the Tauri CLI leaves the `tauri` crate's `custom-protocol`
//! feature off, which drops the embedded assets and points the window at the dev server;
//! the app then starts up on "localhost refused to connect". `build.rs` refuses to produce
//! such a build, and this is the check on the other end: whatever is in `target/release`
//! really does carry the UI.

use std::path::{Path, PathBuf};

/// Markers that only a build with the frontend embedded in it contains. `index.html` alone
/// is not one of them — the config, which names it, is embedded either way.
const EMBEDDED_FRONTEND: &[&str] = &["/assets/main-", "media.worker-"];

fn release_binary() -> Option<PathBuf> {
    // CARGO_MANIFEST_DIR is `app/src-tauri`; the workspace target directory is two up.
    let target = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)?
        .join("target/release");
    let binary = target.join(if cfg!(windows) {
        "backsight.exe"
    } else {
        "backsight"
    });
    binary.is_file().then_some(binary)
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[test]
fn the_release_executable_has_the_frontend_inside_it() {
    let Some(binary) = release_binary() else {
        // Nothing to check: this machine has only ever built in debug.
        return;
    };
    let bytes = std::fs::read(&binary).expect("read the release executable");

    for marker in EMBEDDED_FRONTEND {
        assert!(
            contains(&bytes, marker),
            "{} has no frontend embedded in it ({marker} is missing), so it would open on \
             \"localhost refused to connect\". It was built with plain cargo rather than \
             `npm run tauri build`; delete it and build it again.",
            binary.display(),
        );
    }
}
