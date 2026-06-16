pub mod api;
pub mod commands;
pub mod downloader;
pub mod error;
pub mod library;
pub mod local_server;
pub mod plugins;

use std::sync::Arc;

use tauri::Manager;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::api::auth::SessionStore;
use crate::library::db::LibraryHandle;
use crate::local_server::LocalServer;
use crate::plugins::runtime::PluginRuntime;

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

    let plugin_runtime: Arc<PluginRuntime> = Arc::new(PluginRuntime::default());

    if let Err(err) = tauri::Builder::default()
        .manage(Arc::clone(&session))
        .manage(commands::DownloadTasks::default())
        .manage(Arc::clone(&plugin_runtime))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
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

            app.manage(Arc::clone(&library));

            // ローカル HTTP サーバを起動（DL 済み動画の Range 配信用）
            let videos_root = data_dir.join("videos");
            std::fs::create_dir_all(&videos_root).map_err(|e| format!("create videos dir: {e}"))?;
            let port =
                local_server::start(videos_root).map_err(|e| format!("local server start: {e}"))?;
            app.manage(LocalServer { port });

            // プラグインランタイムを DB から **同期的に** 初期ロードする。
            // 起動直後に呼ばれる plugin_invoke が runtime 未 populate を見て
            // UnknownPlugin/Disabled エラーを返すレースを防ぐため、Tauri が
            // invoke を受け付け始める前にここで完了させる。失敗してもアプリ
            // 起動は止めない (プラグイン無しで通常起動を継続)。
            let plugins_dir = data_dir.join("plugins");
            if let Err(e) = std::fs::create_dir_all(&plugins_dir) {
                tracing::warn!(error = %e, "could not create plugins dir");
            }
            // 前回プロセスが install/uninstall 中にクラッシュした場合に残る
            // `<id>.tmp-<pid>-<n>` / `<id>.bak-<pid>-<n>` のゴミディレクトリを
            // best-effort で掃除する。これを放置すると、ディスク容量を圧迫し、
            // 次回 install 時の tmp 名衝突 → 古いゴミの remove_dir_all で
            // 巻き戻し race の原因にもなる。失敗はログのみ (続行可能)。
            crate::plugins::installer::cleanup_stale_dirs(&plugins_dir);
            // ランタイム初期ロード。DB を真値として PluginEntry を populate する。
            // 内部で warn ログを出すので戻り値はない。
            crate::plugins::bootstrap_blocking(&plugin_runtime, &library);
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
            commands::login_mfa,
            commands::prepare_playback,
            commands::fetch_video_comments,
            commands::issue_hls_url,
            commands::fetch_hls_resource,
            commands::fetch_user_videos,
            commands::fetch_series_videos,
            commands::fetch_user_mylists,
            commands::fetch_user_series_list,
            commands::fetch_mylist_videos,
            commands::enqueue_download,
            commands::list_downloads,
            commands::cancel_download,
            commands::clear_finished_downloads,
            commands::start_download,
            commands::list_library_videos,
            commands::prepare_local_playback,
            commands::list_comment_snapshots,
            commands::load_snapshot_comments,
            commands::delete_comment_snapshot,
            commands::update_snapshot_note,
            commands::refetch_video_comments,
            commands::export_video_with_comments,
            commands::query_library_videos,
            commands::get_library_stats,
            commands::list_library_tags,
            commands::list_library_resolutions,
            commands::search_library_comments,
            commands::list_library_uploaders,
            commands::list_playlists,
            commands::create_playlist,
            commands::update_playlist,
            commands::delete_playlist,
            commands::list_playlist_items,
            commands::add_playlist_item,
            commands::remove_playlist_item,
            commands::extract_video_frame,
            commands::extract_online_frame,
            commands::record_playback,
            commands::list_play_history,
            commands::delete_play_history_item,
            commands::remux_local_video,
            commands::read_local_file,
            commands::delete_library_video,
            commands::cleanup_storage,
            commands::local_video_url,
            commands::local_audio_url,
            commands::get_settings,
            commands::set_setting,
            commands::delete_setting,
            commands::get_app_info,
            commands::fetch_ranking_html,
            commands::search_short_ranking,
            commands::fetch_video_html,
            commands::fetch_related_videos,
            commands::resolve_thumbnail_url,
            // ----- プラグイン (フロント拡張機能) -----
            plugins::commands::plugin_list_installed,
            plugins::commands::plugin_get_manifest,
            plugins::commands::plugin_install_from_zip,
            plugins::commands::plugin_uninstall,
            plugins::commands::plugin_set_enabled,
            plugins::commands::plugin_invoke,
        ])
        .run(tauri::generate_context!())
    {
        tracing::error!(error = %err, "tauri runtime failed");
        std::process::exit(1);
    }
}
