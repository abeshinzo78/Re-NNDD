//! プラグインがフロントから `plugin_invoke(action, payload)` で呼び出せる
//! 限定 API のディスパッチャ。
//!
//! 設計原則:
//! - 全パスは `Result<Value, DispatchError>` を返し panic しない
//! - action に対する required permission を **明示マップ** で管理する
//!   (prefix split による誤判定を避ける)
//! - `permissions[]` に含まれない action は permission denied
//! - `settings.get/set` は key 先頭が `plugin:<id>:` でない場合も拒否

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

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
    // ----- URL/host バリデーション (SSRF 防御) -----
    let parsed = url::Url::parse(&req.url).map_err(|e| DispatchError::InvalidPayload {
        action: "net.fetch".into(),
        message: format!("url parse: {e}"),
    })?;
    if parsed.scheme() != "https" {
        return Err(DispatchError::InvalidPayload {
            action: "net.fetch".into(),
            message: "url must use https scheme".into(),
        });
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| DispatchError::InvalidPayload {
            action: "net.fetch".into(),
            message: "url has no host".into(),
        })?;
    if let Some(reason) = host_is_disallowed(host) {
        return Err(DispatchError::InvalidPayload {
            action: "net.fetch".into(),
            message: format!("host blocked by SSRF guard ({reason})"),
        });
    }
    // ----- DNS を事前解決し、解決後の IP を再検証 (Codex #5 P1: host が
    //   public 名でも DNS で 127.0.0.1 を返すような rebinding 攻撃を遮断) -----
    let port = parsed.port_or_known_default().unwrap_or(443);
    let lookup_target = format!("{host}:{port}");
    let resolved: Vec<std::net::SocketAddr> = tokio::net::lookup_host(&lookup_target)
        .await
        .map_err(|e| DispatchError::Upstream(format!("dns resolve {host}: {e}")))?
        .collect();
    if resolved.is_empty() {
        return Err(DispatchError::Upstream(format!(
            "dns: no addresses for {host}"
        )));
    }
    for addr in &resolved {
        if ip_is_private(addr.ip()) {
            return Err(DispatchError::InvalidPayload {
                action: "net.fetch".into(),
                message: format!(
                    "host {host} resolves to private/loopback IP {} (SSRF guard)",
                    addr.ip()
                ),
            });
        }
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
    // ----- reqwest クライアント (timeout + redirect なし + UA 固定 + 事前解決 IP 固定) -----
    let mut client_builder = reqwest::Client::builder()
        .user_agent("Re-NNDD-plugin/0.1")
        .timeout(NET_FETCH_TIMEOUT)
        .connect_timeout(NET_FETCH_CONNECT_TIMEOUT)
        // redirect を自動追従させると次ホップで SSRF ガードを迂回されるため
        // 拒否。プラグインが必要なら 3xx を見て手動で再 fetch する設計。
        .redirect(reqwest::redirect::Policy::none());
    // 事前解決した IP を pin し、reqwest が再 DNS を引かないようにする
    // (DNS rebinding window をさらに狭める; Codex #5 P1)。
    for addr in &resolved {
        client_builder = client_builder.resolve(host, *addr);
    }
    let client = client_builder
        .build()
        .map_err(|e| DispatchError::Upstream(format!("reqwest build: {e}")))?;
    let mut builder = client.request(method, parsed.as_str());
    // ----- ヘッダはホワイトリスト方式で受け付ける -----
    if let Some(h) = req.headers {
        for (k, v) in h {
            if !is_safe_request_header(&k) {
                return Err(DispatchError::InvalidPayload {
                    action: "net.fetch".into(),
                    message: format!("disallowed header: {k}"),
                });
            }
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
    // ----- body は chunked で読みつつ累積サイズを上限でガード -----
    let mut resp = resp;
    let mut acc: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if acc.len() + chunk.len() > NET_FETCH_MAX_BODY_BYTES {
                    return Err(DispatchError::Upstream(format!(
                        "response body exceeded {} bytes",
                        NET_FETCH_MAX_BODY_BYTES
                    )));
                }
                acc.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => return Err(DispatchError::Upstream(format!("read body: {e}"))),
        }
    }
    let body_b64 = BASE64.encode(&acc);
    Ok(json!({
        "status": status,
        "headers": Value::Object(headers),
        "bodyBase64": body_b64
    }))
}

// ----- net.fetch ガード ヘルパ -----

const NET_FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const NET_FETCH_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// レスポンス body の累積サイズ上限 (10 MiB)。これを超えた時点で stream を
/// 打ち切る。zip インストーラの 50/200 MiB に比して厳しめなのは、メモリ
/// 上にまるごと base64 化して返すための保守的な値。
const NET_FETCH_MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// host が SSRF 危険レンジに該当するなら拒否理由を返す。`None` 通過。
/// IP literal だけでなく文字列 "localhost" 等もここで弾く。
/// 注: DNS 解決後に private IP に着地するケース (DNS rebinding) は
/// reqwest 内部の解決を取れないため完全には防げない — best-effort。
fn host_is_disallowed(host: &str) -> Option<&'static str> {
    let lowered = host.to_ascii_lowercase();
    // 名前ベース blocklist
    let blocked_names = ["localhost"];
    if blocked_names.iter().any(|b| &lowered == b) {
        return Some("localhost literal");
    }
    if lowered.ends_with(".localhost") || lowered.ends_with(".local") {
        return Some("local TLD");
    }
    // IP literal の解析: URL ホストは IPv6 だと `[::1]` 表記。url crate は
    // 既に `[]` を剥がした文字列で host_str を返してくれる。
    if let Ok(ip) = lowered.parse::<IpAddr>() {
        if ip_is_private(ip) {
            return Some("private/loopback IP");
        }
    }
    None
}

fn ip_is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            // is_global は unstable なので手書きで判定する。
            v4.is_loopback()       // 127.0.0.0/8
                || v4.is_private()     // 10/8, 172.16/12, 192.168/16
                || v4.is_link_local()  // 169.254/16
                || v4.is_broadcast()
                || v4.is_unspecified() // 0.0.0.0
                || v4.is_documentation()
                || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 0x40 // 100.64/10 (CGNAT)
        }
        IpAddr::V6(v6) => {
            // 主要な private/loopback レンジを手書きで判定
            v6.is_loopback()     // ::1
                || v6.is_unspecified() // ::
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 (ULA)
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 (link-local)
                // IPv4-mapped IPv6 (::ffff:0:0/96) を IPv4 として解析し直す
                || v6.to_ipv4_mapped().map(|v4| ip_is_private(IpAddr::V4(v4))).unwrap_or(false)
        }
    }
}

/// プラグインが指定可能なリクエストヘッダのホワイトリスト。
/// 値による spoofing リスクのある framing / 認証系は弾く。
fn is_safe_request_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "accept"
            | "accept-language"
            | "accept-encoding"
            | "cache-control"
            | "content-type"
            | "if-match"
            | "if-none-match"
            | "if-modified-since"
            | "if-unmodified-since"
            | "user-agent"
            | "range"
            | "referer"
    )
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
    // offset は i64 → u32。`as u32` だけだと巨大値で wrap-around するので
    // 範囲を u32::MAX に clamp してから cast (lower-severity だが
    // pagination 無限ループ防止)。
    let offset = req.offset.unwrap_or(0).clamp(0, u32::MAX as i64) as u32;
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

/// プラグイン固有 settings キーの prefix。
/// 区切りを `:` にすることで、plugin_id 自体に `.` が含まれていても
/// 「他プラグインの prefix の dot-prefix になる」攻撃を防ぐ。plugin_id の
/// charset は `[a-z0-9._-]` なので `:` は確実に分離記号として効く
/// (PR レビュー: r3297346764 の cross-plugin access 問題)。
fn plugin_settings_prefix(plugin_id: &str) -> String {
    format!("plugin:{plugin_id}:")
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
            json!({"key": "plugin:com.example.test:k", "value": "v"}),
        )
        .await
        .unwrap();
        let got = handle_settings_get(
            &library,
            "com.example.test",
            json!({"key": "plugin:com.example.test:k"}),
        )
        .await
        .unwrap();
        assert_eq!(got, Value::String("v".into()));
    }

    #[tokio::test]
    async fn settings_dot_prefix_cross_plugin_access_blocked() {
        // plugin id "a" が plugin id "a.b" のキー "plugin:a.b:secret" を
        // 触れないことを確認 (Codex #1: dot-prefix で他プラグインの空間に
        // 侵入できる問題の回帰防止)。
        let library = mk_library();
        let err = handle_settings_get(&library, "a", json!({"key": "plugin:a.b:secret"}))
            .await
            .unwrap_err();
        assert!(matches!(err, DispatchError::PermissionDenied { .. }));
        let err = handle_settings_set(
            &library,
            "a",
            json!({"key": "plugin:a.b:secret", "value": "x"}),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DispatchError::PermissionDenied { .. }));
    }

    #[test]
    fn host_blocklist_blocks_loopback_and_private_v4() {
        assert!(host_is_disallowed("127.0.0.1").is_some());
        assert!(host_is_disallowed("10.0.0.1").is_some());
        assert!(host_is_disallowed("172.16.0.1").is_some());
        assert!(host_is_disallowed("172.31.0.1").is_some());
        assert!(host_is_disallowed("192.168.1.1").is_some());
        assert!(host_is_disallowed("169.254.169.254").is_some());
        assert!(host_is_disallowed("0.0.0.0").is_some());
        assert!(host_is_disallowed("100.64.0.1").is_some());
        assert!(host_is_disallowed("localhost").is_some());
        assert!(host_is_disallowed("foo.localhost").is_some());
        assert!(host_is_disallowed("bar.local").is_some());
        // 通常の public host は通る
        assert!(host_is_disallowed("api.example.com").is_none());
        assert!(host_is_disallowed("8.8.8.8").is_none());
    }

    #[test]
    fn host_blocklist_blocks_loopback_and_ula_v6() {
        assert!(host_is_disallowed("::1").is_some());
        assert!(host_is_disallowed("::").is_some());
        assert!(host_is_disallowed("fc00::1").is_some());
        assert!(host_is_disallowed("fd00::1").is_some());
        assert!(host_is_disallowed("fe80::1").is_some());
        // IPv4-mapped private は IPv4 として再検査されて弾かれる
        assert!(host_is_disallowed("::ffff:127.0.0.1").is_some());
        // 通常の IPv6 (Google DNS) は通る
        assert!(host_is_disallowed("2001:4860:4860::8888").is_none());
    }

    #[test]
    fn safe_request_header_allowlist_basics() {
        assert!(is_safe_request_header("Accept"));
        assert!(is_safe_request_header("content-type"));
        assert!(is_safe_request_header("User-Agent"));
        // 認証 / framing は拒否
        assert!(!is_safe_request_header("Host"));
        assert!(!is_safe_request_header("Authorization"));
        assert!(!is_safe_request_header("Cookie"));
        assert!(!is_safe_request_header("Content-Length"));
        assert!(!is_safe_request_header("X-Forwarded-For"));
    }

    #[tokio::test]
    async fn net_fetch_blocks_private_host() {
        let err = handle_net_fetch(json!({"url": "https://127.0.0.1/"}))
            .await
            .unwrap_err();
        // 期待: InvalidPayload(message に SSRF guard 関連の語)
        let matched = matches!(
            &err,
            DispatchError::InvalidPayload { message, .. }
                if message.contains("SSRF") || message.contains("blocked")
        );
        assert!(matched, "expected SSRF guard InvalidPayload, got {err:?}");
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
