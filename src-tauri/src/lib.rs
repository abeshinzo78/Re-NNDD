pub mod api;
pub mod commands;
pub mod downloader;
pub mod error;
pub mod library;
pub mod local_server;

use std::sync::Arc;

use tauri::Manager;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::api::auth::SessionStore;
use crate::library::db::LibraryHandle;
use crate::local_server::LocalServer;

fn init_tracing() {
    // Default verbosity: app + web bridge at debug, everything else at info.
    // Override via NNDD_LOG, e.g. `NNDD_LOG=trace` or `NNDD_LOG=warn`.
    let filter = EnvFilter::try_from_env("NNDD_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,nndd_next_lib=debug,web=debug"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    let session = Arc::new(SessionStore::default());

    if let Err(err) = tauri::Builder::default()
        .manage(Arc::clone(&session))
        .manage(commands::DownloadTasks::default())
        .plugin(tauri_plugin_shell::init())
        .setup(move |app| {
            // app_data_dir() は実行プロファイル（dev/prod）と OS で
            // 自動的に切り替わるので、ここで一度だけ解決して library.db を
            // 開く。マイグレーションは LibraryHandle::open 内で完走する。
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("app_data_dir resolution failed: {e}"))?;
            let db_path = data_dir.join("library.db");
            tracing::info!(path = %db_path.display(), "opening library db");
            let library = LibraryHandle::open(&db_path)
                .map_err(|e| format!("library db init failed: {e}"))?;

            // Restore persisted session from the settings table.
            {
                let conn = library.blocking_lock();
                session.load_from_db(&conn);
            }

            app.manage(library);

            // ローカル HTTP サーバを起動（DL 済み動画の Range 配信用）
            let videos_root = data_dir.join("videos");
            std::fs::create_dir_all(&videos_root).map_err(|e| format!("create videos dir: {e}"))?;
            let port =
                local_server::start(videos_root).map_err(|e| format!("local server start: {e}"))?;
            app.manage(LocalServer { port });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_version,
            commands::web_log,
            commands::search_videos_online,
            commands::save_session_cookie,
            commands::clear_session_cookie,
            commands::session_cookie_status,
            commands::login_password,
            commands::prepare_playback,
            commands::fetch_video_comments,
            commands::issue_hls_url,
            commands::fetch_hls_resource,
            commands::fetch_user_videos,
            commands::enqueue_download,
            commands::list_downloads,
            commands::cancel_download,
            commands::clear_finished_downloads,
            commands::start_download,
            commands::list_library_videos,
            commands::prepare_local_playback,
            commands::query_library_videos,
            commands::get_library_stats,
            commands::list_library_tags,
            commands::list_library_resolutions,
            commands::remux_local_video,
            commands::read_local_file,
            commands::delete_library_video,
            commands::cleanup_storage,
            commands::local_video_url,
            commands::local_audio_url,
            commands::local_thumbnail_url,
            commands::get_settings,
            commands::set_setting,
            commands::delete_setting,
            commands::get_app_info,
        ])
        .run(tauri::generate_context!())
    {
        tracing::error!(error = %err, "tauri runtime failed");
        std::process::exit(1);
    }
}
