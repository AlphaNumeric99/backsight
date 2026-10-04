mod audio_aac;
mod cameras;
mod commands;
mod db;
mod error;
mod exports;
mod model;
mod previews;
mod qubo;
mod recordings;
mod secrets;
mod streams;
mod thumbnails;
mod wire;

use std::sync::Arc;

use tauri::Manager;
use tauri::http::{Response, StatusCode, header};
use tauri::webview::PageLoadEvent;

use crate::cameras::CameraManager;
use crate::commands::AppState;
use crate::db::Db;
use crate::qubo::Qubo;
use crate::thumbnails::Thumbnails;

/// The `thumb` scheme: detection thumbnails at `/<camera>/<start>`, from the cache or the
/// camera, and camera previews at `/preview/<camera>/<saved_at>`.
fn thumbnail_protocol(
    ctx: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let path = request.uri().path().to_owned();
    tauri::async_runtime::spawn(async move {
        let not_found = || {
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Vec::new())
                .expect("response")
        };
        let Some(state) = app.try_state::<AppState>() else {
            return responder.respond(not_found());
        };
        let jpeg = if let Some(camera_id) = previews::parse_path(&path) {
            state.cameras.preview(&camera_id)
        } else if let Some((camera_id, start)) = thumbnails::parse_path(&path) {
            state.thumbnails.get(&camera_id, start).await
        } else {
            None
        };
        match jpeg {
            // Both URL kinds name one immutable picture (previews carry their save time).
            Some(jpeg) => responder.respond(
                Response::builder()
                    .header(header::CONTENT_TYPE, "image/jpeg")
                    .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
                    .body(jpeg)
                    .expect("response"),
            ),
            None => responder.respond(not_found()),
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("BACKSIGHT_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol(thumbnails::SCHEME, thumbnail_protocol)
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db = Arc::new(Db::open(&data_dir.join("backsight.db"))?);
            let cache_dir = app.path().app_cache_dir()?;
            let previews = previews::Previews::new(cache_dir.join("previews"));
            let qubo = Arc::new(Qubo::new()?);
            let cameras =
                CameraManager::start(app.handle().clone(), db.clone(), previews, qubo.clone())
                    .map_err(|e| e.message)?;
            let default_export_dir = commands::default_export_dir(app.handle());
            let thumbnails = Arc::new(Thumbnails::new(
                cache_dir.join("thumbnails"),
                cameras.clone(),
            ));
            let streams = Arc::new(streams::Streams::new(cameras.clone(), qubo.clone()));
            let exports = Arc::new(exports::Exports::new(db.clone(), cameras.clone()));
            app.manage(AppState {
                db,
                cameras,
                thumbnails,
                streams,
                exports,
                default_export_dir,
            });
            Ok(())
        })
        .on_page_load(|webview, payload| {
            // Not raised for route (hash) changes, only when a new document loads.
            if payload.event() == PageLoadEvent::Started
                && let Some(state) = webview.try_state::<AppState>()
            {
                state.streams.close_webview(webview.label());
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_cameras,
            commands::get_camera,
            commands::discover,
            commands::add_camera,
            commands::list_qubo_devices,
            commands::add_qubo_camera,
            commands::update_camera,
            commands::remove_camera,
            commands::list_groups,
            commands::save_groups,
            commands::get_days_with_recordings,
            commands::get_day_index,
            commands::get_settings,
            commands::update_settings,
            commands::list_exports,
            commands::save_snapshot,
            commands::save_preview,
            commands::reveal_export,
            commands::open_stream,
            commands::close_stream,
            commands::start_export,
            commands::cancel_export,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
