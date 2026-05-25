//! ユーザーインストール型プラグインの Rust 側ホスト。
//!
//! - `manifest`: `manifest.json` のパース/バリデーション
//! - `registry`: SQLite `plugins` テーブルへの CRUD
//! - `installer`: ZIP の展開と path traversal 防止
//! - `runtime`: プロセス内の有効プラグイン一覧 (DB のキャッシュ)
//! - `dispatcher`: `plugin_invoke(action, payload)` のルーティングと
//!   permission チェック
//!
//! 起動時 (`lib.rs::setup`) に `runtime::bootstrap(&runtime, &library)` で
//! DB から再構築する。各 Tauri command (`plugin_*`) は DB と runtime の両方
//! を同期で更新する。

pub mod commands;
pub mod dispatcher;
pub mod installer;
pub mod manifest;
pub mod registry;
pub mod runtime;

pub use dispatcher::{dispatch, DispatchError};
pub use installer::{install_from_zip_path, uninstall, InstallError, InstallResult};
pub use manifest::{PluginManifest, ALLOWED_PERMISSIONS};
pub use runtime::{bootstrap_blocking, PluginEntry, PluginRuntime};

/// Rust → Frontend のプラグインイベントを emit するヘルパ。
///
/// host (`src/lib/plugins/host.ts`) が `nndd:plugin:event` を 1 本だけ listen
/// しており、`{name, payload}` 形式の envelope を内部 event bus に再 emit する。
/// emit 失敗はログに留めて、呼出元の処理を **絶対に** 落とさない。
///
/// listener 0 件のときも no-op で完了するため、プラグイン無効時 (= host が
/// listen を張っていない場合) の挙動はプラグイン機構導入前と完全に同一。
pub fn emit_event(app: &tauri::AppHandle, name: &str, payload: serde_json::Value) {
    use tauri::Emitter;
    let envelope = serde_json::json!({
        "name": name,
        "payload": payload,
    });
    if let Err(e) = app.emit("nndd:plugin:event", envelope) {
        tracing::warn!(event = name, error = %e, "plugin event emit failed");
    }
}
