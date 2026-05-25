//! プラグイン関連の Tauri command。`lib.rs` の `invoke_handler` から
//! 一括で登録される。
//!
//! - `plugin_list_installed` / `plugin_get_manifest`: 一覧 + 単体取得
//! - `plugin_install_from_zip`: ZIP からインストール (path 受け取り)
//! - `plugin_uninstall`: 削除 (DB + ファイル)
//! - `plugin_set_enabled`: 有効/無効切替 (DB + runtime)
//! - `plugin_invoke`: dispatcher 入口
//!
//! 全コマンドが `Result<_, AppError>` を返し panic しない。

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, Result};
use crate::library::db::LibraryHandle;
use crate::library::now_unix_secs;
use crate::plugins::installer;
use crate::plugins::manifest::PluginManifest;
use crate::plugins::registry;
use crate::plugins::runtime::PluginRuntime;
use crate::plugins::DispatchError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub plugin_id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub description: Option<String>,
    pub author: Option<String>,
    pub homepage: Option<String>,
    pub entry: String,
    pub permissions: Vec<String>,
    /// `$APPDATA/plugins/<id>/<entry>` の **絶対パス**。フロントは
    /// `convertFileSrc(...)` で asset:// に変換して動的 `import()` する。
    pub entry_abs_path: String,
    pub installed_at: i64,
    pub updated_at: i64,
}

fn plugins_root(app: &AppHandle) -> Result<std::path::PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("app_data_dir: {e}")))?
        .join("plugins");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| AppError::Other(format!("create plugins root: {e}")))?;
    }
    Ok(dir)
}

fn row_to_info(
    plugin_id: &str,
    manifest: &PluginManifest,
    enabled: bool,
    plugins_root: &std::path::Path,
    installed_at: i64,
    updated_at: i64,
) -> PluginInfo {
    let entry_abs = plugins_root
        .join(plugin_id)
        .join(&manifest.entry)
        .to_string_lossy()
        .to_string();
    PluginInfo {
        plugin_id: plugin_id.to_string(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        enabled,
        description: manifest.description.clone(),
        author: manifest.author.clone(),
        homepage: manifest.homepage.clone(),
        entry: manifest.entry.clone(),
        permissions: manifest.permissions.clone(),
        entry_abs_path: entry_abs,
        installed_at,
        updated_at,
    }
}

#[tauri::command]
pub async fn plugin_list_installed(
    app: AppHandle,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<Vec<PluginInfo>> {
    let root = plugins_root(&app)?;
    let conn = library.lock().await;
    let rows = registry::list_all(&conn).map_err(AppError::from)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        match PluginManifest::parse_and_validate(&row.manifest_json, None) {
            Ok(m) => out.push(row_to_info(
                &row.plugin_id,
                &m,
                row.enabled,
                &root,
                row.installed_at,
                row.updated_at,
            )),
            Err(e) => {
                tracing::warn!(plugin_id = %row.plugin_id, error = %e, "skipping plugin row");
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn plugin_get_manifest(
    id: String,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<Option<PluginManifest>> {
    let conn = library.lock().await;
    let row = registry::get(&conn, &id).map_err(AppError::from)?;
    Ok(row.and_then(|r| PluginManifest::parse_and_validate(&r.manifest_json, None).ok()))
}

#[tauri::command]
pub async fn plugin_install_from_zip(
    path: String,
    replace: bool,
    app: AppHandle,
    library: State<'_, Arc<LibraryHandle>>,
    runtime: State<'_, Arc<PluginRuntime>>,
) -> Result<PluginInfo> {
    let root = plugins_root(&app)?;
    let app_version = env!("CARGO_PKG_VERSION");
    let zip_path = std::path::PathBuf::from(&path);
    let install_res = installer::install_from_zip_path(&root, &zip_path, replace, app_version)
        .await
        .map_err(|e| AppError::Other(format!("install failed: {e}")))?;

    let manifest_json = serde_json::to_string(&install_res.manifest)
        .map_err(|e| AppError::Other(format!("serialize manifest: {e}")))?;
    let now = now_unix_secs();
    let previous_enabled = {
        let conn = library.lock().await;
        // 上書き (replace=true) の場合は既存行の enabled フラグを保持したい。
        // registry::upsert の SQL は ON CONFLICT 時に enabled を触らないので
        // DB 側は既存値が残るが、in-memory runtime と返値 PluginInfo は
        // ここで明示的に保持しないと split-brain になる (Codex review #1)。
        let prev = registry::get(&conn, &install_res.manifest.id)
            .map_err(AppError::from)?
            .map(|r| r.enabled)
            .unwrap_or(false);
        registry::upsert(
            &conn,
            &install_res.manifest.id,
            &install_res.manifest.version,
            &manifest_json,
            now,
        )
        .map_err(AppError::from)?;
        prev
    };
    // in-memory にも反映 (既存行の enabled を維持)。
    runtime.upsert(
        &install_res.manifest.id,
        install_res.manifest.clone(),
        previous_enabled,
    );

    // インストール直後の Info を返す。再ロード不要 (DB と一致しているはず)。
    Ok(row_to_info(
        &install_res.manifest.id,
        &install_res.manifest,
        previous_enabled,
        &root,
        now,
        now,
    ))
}

#[tauri::command]
pub async fn plugin_uninstall(
    id: String,
    app: AppHandle,
    library: State<'_, Arc<LibraryHandle>>,
    runtime: State<'_, Arc<PluginRuntime>>,
) -> Result<()> {
    let root = plugins_root(&app)?;
    // 1) ファイル先 (UnsafePath なら拒否される — install 時の id を信頼するが二重防御)
    installer::uninstall(&root, &id).map_err(|e| AppError::Other(format!("uninstall fs: {e}")))?;
    // 2) DB
    {
        let conn = library.lock().await;
        registry::delete(&conn, &id).map_err(AppError::from)?;
    }
    // 3) in-memory
    runtime.remove(&id);
    Ok(())
}

#[tauri::command]
pub async fn plugin_set_enabled(
    id: String,
    enabled: bool,
    library: State<'_, Arc<LibraryHandle>>,
    runtime: State<'_, Arc<PluginRuntime>>,
) -> Result<()> {
    let now = now_unix_secs();
    {
        let conn = library.lock().await;
        let changed = registry::set_enabled(&conn, &id, enabled, now).map_err(AppError::from)?;
        if changed == 0 {
            return Err(AppError::Other(format!("plugin not found: {id}")));
        }
    }
    runtime.set_enabled(&id, enabled);
    Ok(())
}

#[tauri::command]
pub async fn plugin_invoke(
    plugin_id: String,
    action: String,
    payload: serde_json::Value,
    app: AppHandle,
    library: State<'_, Arc<LibraryHandle>>,
    runtime: State<'_, Arc<PluginRuntime>>,
) -> Result<serde_json::Value> {
    let library_handle = library.inner().clone();
    let runtime_handle = runtime.inner().clone();
    crate::plugins::dispatcher::dispatch(
        &app,
        &runtime_handle,
        &library_handle,
        &plugin_id,
        &action,
        payload,
    )
    .await
    .map_err(|e: DispatchError| {
        tracing::warn!(plugin = %plugin_id, action = %action, error = %e, "plugin dispatch failed");
        AppError::Other(e.into_string())
    })
}
