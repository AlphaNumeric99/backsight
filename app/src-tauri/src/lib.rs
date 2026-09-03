#[allow(dead_code)] // used by exports, which land next
mod audio_aac;
mod cameras;
mod commands;
mod db;
mod error;
mod model;
mod recordings;
mod secrets;

use std::sync::Arc;

use tauri::Manager;

use crate::cameras::CameraManager;
use crate::commands::AppState;
use crate::db::Db;

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
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db = Arc::new(Db::open(&data_dir.join("backsight.db"))?);
            let cameras =
                CameraManager::start(app.handle().clone(), db.clone()).map_err(|e| e.message)?;
            let default_export_dir = commands::default_export_dir(app.handle());
            app.manage(AppState {
                db,
                cameras,
                default_export_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_cameras,
            commands::get_camera,
            commands::discover,
            commands::add_camera,
            commands::update_camera,
            commands::remove_camera,
            commands::list_groups,
            commands::save_groups,
            commands::get_days_with_recordings,
            commands::get_day_index,
            commands::get_settings,
            commands::update_settings,
            commands::list_exports,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
