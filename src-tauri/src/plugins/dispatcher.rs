//! プラグインがフロントから `plugin_invoke(action, payload)` で呼び出せる
//! 限定 API のディスパッチャ。
//!
//! 設計原則:
//! - 全パスは `Result<Value, DispatchError>` を返し panic しない
//! - action に対する required permission を **明示マップ** で管理する
//!   (prefix split による誤判定を避ける)
//! - `permissions[]` に含まれない action は permission denied
//! - `settings.get/set` は key 先頭が `plugin.<id>.` でない場合も拒否

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::library::db::LibraryHandle;
use crate::library::query::{self as lib_query, LibraryQuery};
use crate::library::settings as lib_settings;
use crate::plugins::runtime::PluginRuntime;

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("unknown plugin: {0}")]
    UnknownPlugin(String),
    #[error("plugin disabled: {0}")]
    Disabled(String),
    #[error("unknown action: {0}")]
    UnknownAction(String),
    #[error("permission denied: action {action} requires {permission}")]
    PermissionDenied { action: String, permission: String },
    #[error("invalid payload for {action}: {message}")]
    InvalidPayload { action: String, message: String },
    #[error("upstream error: {0}")]
    Upstream(String),
}

impl DispatchError {
    pub fn into_string(self) -> String {
        self.to_string()
    }
}

/// `action` に対して必要な permission 名を返す。
/// `None` の場合は未知の action で、無条件に拒否する。
fn required_permission(action: &str) -> Option<&'static str> {
    match action {
        "net.fetch" => Some("net.fetch"),
        "library.list" => Some("library.read"),
        "settings.get" => Some("settings.read"),
        "settings.set" => Some("settings.write"),
        "notify.toast" => Some("notify"),
        _ => None,
    }
}

pub async fn dispatch(
    app: &AppHandle,
    runtime: &Arc<PluginRuntime>,
    library: &Arc<LibraryHandle>,
    plugin_id: &str,
    action: &str,
    payload: Value,
) -> Result<Value, DispatchError> {
    let entry = runtime
        .get(plugin_id)
        .ok_or_else(|| DispatchError::UnknownPlugin(plugin_id.to_string()))?;
    if !entry.enabled {
        return Err(DispatchError::Disabled(plugin_id.to_string()));
    }
    let perm = required_permission(action)
        .ok_or_else(|| DispatchError::UnknownAction(action.to_string()))?;
    if !entry.manifest.permissions.iter().any(|p| p == perm) {
        return Err(DispatchError::PermissionDenied {
            action: action.to_string(),
            permission: perm.to_string(),
        });
    }
    match action {
        "net.fetch" => handle_net_fetch(payload).await,
        "library.list" => handle_library_list(library, payload).await,
        "settings.get" => handle_settings_get(library, plugin_id, payload).await,
        "settings.set" => handle_settings_set(library, plugin_id, payload).await,
        "notify.toast" => handle_notify_toast(app, plugin_id, payload),
        _ => Err(DispatchError::UnknownAction(action.to_string())),
    }
}

// ------------------ net.fetch ------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NetFetchReq {
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    body: Option<String>,
}

async fn handle_net_fetch(payload: Value) -> Result<Value, DispatchError> {
    let req: NetFetchReq =
        serde_json::from_value(payload).map_err(|e| DispatchError::InvalidPayload {
            action: "net.fetch".into(),
            message: e.to_string(),
        })?;
    if !req.url.starts_with("https://") {
        return Err(DispatchError::InvalidPayload {
            action: "net.fetch".into(),
            message: "url must start with https://".into(),
        });
    }
    let method = req.method.as_deref().unwrap_or("GET").to_uppercase();
    let method = match method.as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "HEAD" => reqwest::Method::HEAD,
        other => {
            return Err(DispatchError::InvalidPayload {
                action: "net.fetch".into(),
                message: format!("unsupported method: {other}"),
            })
        }
    };
    let client = reqwest::Client::builder()
        .user_agent("Re-NNDD-plugin/0.1")
        .build()
        .map_err(|e| DispatchError::Upstream(format!("reqwest build: {e}")))?;
    let mut builder = client.request(method, &req.url);
    if let Some(h) = req.headers {
        for (k, v) in h {
            builder = builder.header(k, v);
        }
    }
    if let Some(body) = req.body {
        builder = builder.body(body);
    }
    let resp = builder
        .send()
        .await
        .map_err(|e| DispatchError::Upstream(format!("request: {e}")))?;
    let status = resp.status().as_u16();
    let mut headers = serde_json::Map::new();
    for (k, v) in resp.headers().iter() {
        if let Ok(s) = v.to_str() {
            headers.insert(k.as_str().to_string(), Value::String(s.to_string()));
        }
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| DispatchError::Upstream(format!("read body: {e}")))?;
    let body_b64 = BASE64.encode(&bytes);
    Ok(json!({
        "status": status,
        "headers": Value::Object(headers),
        "bodyBase64": body_b64
    }))
}

// ------------------ library.list ------------------

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct LibraryListReq {
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn handle_library_list(
    library: &Arc<LibraryHandle>,
    payload: Value,
) -> Result<Value, DispatchError> {
    let req: LibraryListReq = if payload.is_null() {
        LibraryListReq::default()
    } else {
        serde_json::from_value(payload).map_err(|e| DispatchError::InvalidPayload {
            action: "library.list".into(),
            message: e.to_string(),
        })?
    };
    let limit = req.limit.unwrap_or(50).clamp(1, 200) as u32;
    let offset = req.offset.unwrap_or(0).max(0) as u32;
    let q = LibraryQuery {
        q: None,
        tags: None,
        tags_any: None,
        uploader_id: None,
        min_duration: None,
        max_duration: None,
        resolution: None,
        is_short: None,
        sort_by: Some("downloaded_at".into()),
        sort_order: Some("desc".into()),
        offset: Some(offset),
        limit: Some(limit),
    };
    let conn = library.lock().await;
    let res =
        lib_query::query_videos(&conn, &q).map_err(|e| DispatchError::Upstream(e.to_string()))?;
    let items: Vec<Value> = res
        .items
        .into_iter()
        .map(|v| {
            json!({
                "videoId": v.id,
                "title": v.title,
                "durationSec": v.duration_sec,
                "postedAt": v.posted_at,
                "downloadedAt": v.downloaded_at,
                "uploaderId": v.uploader_id,
                "uploaderName": v.uploader_name,
                "thumbnailUrl": v.thumbnail_url,
                "tags": v.tags,
            })
        })
        .collect();
    Ok(json!({
        "items": items,
        "totalCount": res.total_count,
        "offset": res.offset,
        "limit": res.limit,
    }))
}

// ------------------ settings.get / settings.set ------------------

#[derive(Deserialize)]
struct SettingsGetReq {
    key: String,
}

#[derive(Deserialize)]
struct SettingsSetReq {
    key: String,
    value: String,
}

fn plugin_settings_prefix(plugin_id: &str) -> String {
    format!("plugin.{plugin_id}.")
}

async fn handle_settings_get(
    library: &Arc<LibraryHandle>,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, DispatchError> {
    let req: SettingsGetReq =
        serde_json::from_value(payload).map_err(|e| DispatchError::InvalidPayload {
            action: "settings.get".into(),
            message: e.to_string(),
        })?;
    let prefix = plugin_settings_prefix(plugin_id);
    if !req.key.starts_with(&prefix) {
        return Err(DispatchError::PermissionDenied {
            action: "settings.get".into(),
            permission: format!("key must start with {prefix}"),
        });
    }
    let conn = library.lock().await;
    let v =
        lib_settings::get(&conn, &req.key).map_err(|e| DispatchError::Upstream(e.to_string()))?;
    Ok(match v {
        Some(s) => Value::String(s),
        None => Value::Null,
    })
}

async fn handle_settings_set(
    library: &Arc<LibraryHandle>,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, DispatchError> {
    let req: SettingsSetReq =
        serde_json::from_value(payload).map_err(|e| DispatchError::InvalidPayload {
            action: "settings.set".into(),
            message: e.to_string(),
        })?;
    let prefix = plugin_settings_prefix(plugin_id);
    if !req.key.starts_with(&prefix) {
        return Err(DispatchError::PermissionDenied {
            action: "settings.set".into(),
            permission: format!("key must start with {prefix}"),
        });
    }
    let conn = library.lock().await;
    lib_settings::set(&conn, &req.key, &req.value)
        .map_err(|e| DispatchError::Upstream(e.to_string()))?;
    Ok(Value::Null)
}

// ------------------ notify.toast ------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NotifyToastReq {
    message: String,
    #[serde(default)]
    kind: Option<String>,
}

fn handle_notify_toast(
    app: &AppHandle,
    plugin_id: &str,
    payload: Value,
) -> Result<Value, DispatchError> {
    let req: NotifyToastReq =
        serde_json::from_value(payload).map_err(|e| DispatchError::InvalidPayload {
            action: "notify.toast".into(),
            message: e.to_string(),
        })?;
    let kind = req.kind.unwrap_or_else(|| "info".to_string());
    // ホストの host.ts は `nndd:plugin:event` を 1 本だけ listen している。
    // ここで独立チャンネル名を使うとフロントに届かない (Codex review #5) ため、
    // 共通の emit_event ヘルパ経由で `notify:toast` イベントとして配信する。
    crate::plugins::emit_event(
        app,
        "notify:toast",
        json!({
            "pluginId": plugin_id,
            "message": req.message,
            "kind": kind,
        }),
    );
    Ok(Value::Null)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    //! 個別 handler の検査のみ。`dispatch()` 全体は `AppHandle` を要するため
    //! Tauri 統合テストに任せる。permission チェックと InvalidPayload 系は
    //! ここで十分カバーできる。
    use super::*;

    fn mk_library() -> Arc<LibraryHandle> {
        LibraryHandle::open_memory().unwrap()
    }

    #[test]
    fn unknown_action_returns_none_required_permission() {
        assert!(required_permission("totally.unknown").is_none());
    }

    #[test]
    fn permission_map_is_consistent() {
        let allowed: std::collections::HashSet<&'static str> =
            crate::plugins::manifest::ALLOWED_PERMISSIONS
                .iter()
                .copied()
                .collect();
        for action in &[
            "net.fetch",
            "library.list",
            "settings.get",
            "settings.set",
            "notify.toast",
        ] {
            let perm = required_permission(action).unwrap();
            assert!(allowed.contains(perm), "{perm} not in ALLOWED_PERMISSIONS");
        }
    }

    #[tokio::test]
    async fn settings_get_rejects_non_plugin_key() {
        let library = mk_library();
        let err = handle_settings_get(
            &library,
            "com.example.test",
            json!({"key": "playback.autoplay"}),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DispatchError::PermissionDenied { .. }));
    }

    #[tokio::test]
    async fn settings_set_rejects_non_plugin_key() {
        let library = mk_library();
        let err = handle_settings_set(
            &library,
            "com.example.test",
            json!({"key": "playback.autoplay", "value": "evil"}),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DispatchError::PermissionDenied { .. }));
    }

    #[tokio::test]
    async fn settings_set_then_get_round_trip() {
        let library = mk_library();
        handle_settings_set(
            &library,
            "com.example.test",
            json!({"key": "plugin.com.example.test.k", "value": "v"}),
        )
        .await
        .unwrap();
        let got = handle_settings_get(
            &library,
            "com.example.test",
            json!({"key": "plugin.com.example.test.k"}),
        )
        .await
        .unwrap();
        assert_eq!(got, Value::String("v".into()));
    }

    #[tokio::test]
    async fn net_fetch_requires_https() {
        let err = handle_net_fetch(json!({"url": "http://example.com"}))
            .await
            .unwrap_err();
        assert!(matches!(err, DispatchError::InvalidPayload { .. }));
    }

    #[tokio::test]
    async fn net_fetch_rejects_unknown_method() {
        let err = handle_net_fetch(json!({"url": "https://example.com", "method": "PATCH"}))
            .await
            .unwrap_err();
        assert!(matches!(err, DispatchError::InvalidPayload { .. }));
    }

    #[tokio::test]
    async fn library_list_works_on_empty_db() {
        let library = mk_library();
        let v = handle_library_list(&library, Value::Null).await.unwrap();
        let items = v.get("items").unwrap().as_array().unwrap();
        assert!(items.is_empty());
    }
}
