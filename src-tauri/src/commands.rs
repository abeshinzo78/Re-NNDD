//! Tauri invoke handlers. Thin glue between the frontend and the Rust modules;
//! domain logic lives under `api/`, `library/`, etc.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use reqwest::header;
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::watch;

use crate::api::auth::{login_with_password, LoginOutcome, SessionStore};
use crate::api::comment::{Comment, CommentApi, ThreadsClient};
use crate::api::search::{SearchApi, SnapshotSearchClient};
use crate::api::types::{SearchQuery, SearchResponse};
use crate::api::video::{
    quality_candidates, NiconicoWatchClient, NvCommentSetup, WatchApi, WatchOwner, WatchVideoMeta,
};
use crate::error::{AppError, Result};
use crate::library::db::LibraryHandle;
use crate::library::query::{self, LibraryQuery, LibraryStats, QueryResult};
use crate::library::queue::{self, DownloadQueueItem};
use crate::library::settings;
use crate::library::videos::{self, CommentRecord, IngestPayload, TagRecord, VideoRecord};
use crate::local_server::LocalServer;

#[derive(Clone, Default)]
pub struct DownloadTasks {
    inner: Arc<Mutex<HashMap<i64, watch::Sender<bool>>>>,
}

impl DownloadTasks {
    fn insert(&self, id: i64, tx: watch::Sender<bool>) {
        if let Ok(mut tasks) = self.inner.lock() {
            if let Some(old) = tasks.insert(id, tx) {
                let _ = old.send(true);
            }
        }
    }

    fn cancel(&self, id: i64) {
        if let Ok(mut tasks) = self.inner.lock() {
            if let Some(tx) = tasks.remove(&id) {
                let _ = tx.send(true);
            }
        }
    }

    fn remove(&self, id: i64) {
        if let Ok(mut tasks) = self.inner.lock() {
            tasks.remove(&id);
        }
    }
}

#[tauri::command]
pub fn get_app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// `video_id` が `app_data_dir/videos/{video_id}` のサブディレクトリ名として
/// 安全に使える形式かを検証する。
///
/// niconico の watch ID は `sm12345` `nm67890` `so11111` のように
/// 英数字 + ハイフン + アンダースコアだけ。`/`, `\`, `..`, NUL 等が混ざった
/// `video_id` を弾くことで、フロントエンド側に XSS が入っても
/// `delete_library_video("../../../")` のようなディレクトリトラバーサルで
/// 任意ディレクトリを破壊されないようにする。
fn validate_video_id(video_id: &str) -> std::result::Result<(), AppError> {
    if video_id.is_empty() || video_id.len() > 64 {
        return Err(AppError::Other(format!("invalid video_id: {video_id:?}")));
    }
    if !video_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AppError::Other(format!("invalid video_id: {video_id:?}")));
    }
    Ok(())
}

/// Forward a WebView console message to the Rust tracing pipeline.
/// Called from a `console.*` shim in the frontend so devs without the
/// WebKit inspector can still see browser-side logs in `/tmp/tauri-dev.log`.
#[tauri::command]
pub fn web_log(level: String, message: String) {
    match level.as_str() {
        "error" => tracing::error!(target: "web", "{message}"),
        "warn" => tracing::warn!(target: "web", "{message}"),
        "info" => tracing::info!(target: "web", "{message}"),
        "debug" | "log" => tracing::debug!(target: "web", "{message}"),
        _ => tracing::info!(target: "web", "{message}"),
    }
}

#[tauri::command]
pub async fn search_videos_online(query: SearchQuery) -> Result<SearchResponse> {
    let client = SnapshotSearchClient::new().map_err(AppError::from)?;
    let response = client.search(&query).await.map_err(AppError::from)?;
    Ok(response)
}

#[tauri::command]
pub async fn save_session_cookie(
    value: String,
    store: State<'_, Arc<SessionStore>>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<()> {
    let conn = library.lock().await;
    store.set_with_conn(value, &conn);
    Ok(())
}

#[tauri::command]
pub async fn clear_session_cookie(
    store: State<'_, Arc<SessionStore>>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<()> {
    let conn = library.lock().await;
    store.clear_with_conn(&conn);
    Ok(())
}

#[tauri::command]
pub fn session_cookie_status(store: State<'_, Arc<SessionStore>>) -> bool {
    store.is_set()
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoginResult {
    Success,
    Mfa { mfa_session: Option<String> },
    InvalidCredentials,
}

#[tauri::command]
pub async fn login_password(
    email: String,
    password: String,
    store: State<'_, Arc<SessionStore>>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<LoginResult> {
    let outcome = login_with_password(&email, &password)
        .await
        .map_err(AppError::from)?;
    match outcome {
        LoginOutcome::Success { user_session } => {
            let conn = library.lock().await;
            store.set_with_conn(user_session, &conn);
            Ok(LoginResult::Success)
        }
        LoginOutcome::Mfa { mfa_session } => Ok(LoginResult::Mfa { mfa_session }),
        LoginOutcome::InvalidCredentials => Ok(LoginResult::InvalidCredentials),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedQuality {
    pub video_track: String,
    pub audio_track: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPayload {
    pub video: WatchVideoMeta,
    pub owner: Option<WatchOwner>,
    pub hls_url: String,
    pub picked_quality: PickedQuality,
    pub all_qualities: Vec<PickedQuality>,
    /// NvComment setup — frontend passes this to `fetch_video_comments`.
    pub nv_comment: Option<NvCommentSetup>,
    /// JWT used to call `access-rights/hls`. Front-end keeps this so it can
    /// re-issue a signed HLS URL via [`issue_hls_url`] when the original
    /// expires (~30 s TTL).
    pub access_right_key: String,
    /// Echo back the video id so the frontend can call `issue_hls_url`
    /// without re-deriving it from the route.
    pub video_id: String,
}

/// Fast path: fetch watch page → HLS URL. Returns as soon as the video
/// can start playing. Comments are loaded separately via
/// [`fetch_video_comments`].
#[tauri::command]
pub async fn prepare_playback(
    video_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<PlaybackPayload> {
    let session = Arc::clone(&store);
    let watch = NiconicoWatchClient::new(Arc::clone(&session)).map_err(AppError::from)?;

    let page = watch
        .fetch_watch_page(&video_id)
        .await
        .map_err(AppError::from)?;

    let domand = page.domand.ok_or_else(|| {
        AppError::Other(
            "この動画は再生情報が取得できません（削除済み・プレミアム限定・要ログインなど）".into(),
        )
    })?;
    let candidates = quality_candidates(&domand);
    if candidates.is_empty() {
        return Err(AppError::Other(
            "利用可能な画質/音質トラックが見つかりません".into(),
        ));
    }

    let outputs: Vec<(String, String)> = candidates
        .iter()
        .map(|candidate| (candidate.video_track.clone(), candidate.audio_track.clone()))
        .collect();
    let all_qualities: Vec<PickedQuality> = candidates
        .iter()
        .map(|c| PickedQuality {
            video_track: c.video_track.clone(),
            audio_track: c.audio_track.clone(),
            label: c.label.clone(),
        })
        .collect();
    let picked = candidates
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Other("利用可能な画質/音質トラックが見つかりません".into()))?;

    let hls = watch
        .fetch_hls_outputs(
            &video_id,
            &domand.access_right_key,
            page.watch_track_id.as_deref(),
            &outputs,
        )
        .await
        .map_err(AppError::from)?;

    Ok(PlaybackPayload {
        video: page.video,
        owner: page.owner,
        hls_url: hls.content_url,
        picked_quality: PickedQuality {
            video_track: picked.video_track,
            audio_track: picked.audio_track,
            label: picked.label,
        },
        all_qualities,
        nv_comment: page.nv_comment,
        access_right_key: domand.access_right_key,
        video_id,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchCommentsPayload {
    pub nv_comment: NvCommentSetup,
}

#[tauri::command]
pub async fn fetch_video_comments(
    nv_comment: NvCommentSetup,
    store: State<'_, Arc<SessionStore>>,
) -> Result<Vec<Comment>> {
    let session = Arc::clone(&store);
    let client = ThreadsClient::new(session).map_err(AppError::from)?;
    let comments = client
        .fetch_comments(&nv_comment)
        .await
        .map_err(AppError::from)?;
    Ok(comments)
}

/// Issue a fresh HLS URL by re-running the watch-page → access-rights flow.
///
/// We can't just replay the original `accessRightKey` because niconico
/// invalidates it after the first `access-rights/hls` call (HTTP 400
/// INVALID_PARAMETER on retry). Each issuance therefore needs a fresh
/// watch page fetch — costs ~1 s but only fires when the prior signed
/// URL expires (~30 s TTL).
#[tauri::command]
pub async fn issue_hls_url(
    video_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<String> {
    let watch = NiconicoWatchClient::new(Arc::clone(&store)).map_err(AppError::from)?;
    let page = watch
        .fetch_watch_page(&video_id)
        .await
        .map_err(AppError::from)?;
    let domand = page.domand.ok_or_else(|| {
        AppError::Other(
            "この動画は再生情報が取得できません（削除済み・プレミアム限定・要ログインなど）".into(),
        )
    })?;
    let candidates = quality_candidates(&domand);
    if candidates.is_empty() {
        return Err(AppError::Other(
            "利用可能な画質/音質トラックが見つかりません".into(),
        ));
    }
    let outputs: Vec<(String, String)> = candidates
        .iter()
        .map(|candidate| (candidate.video_track.clone(), candidate.audio_track.clone()))
        .collect();
    let hls = watch
        .fetch_hls_outputs(
            &video_id,
            &domand.access_right_key,
            page.watch_track_id.as_deref(),
            &outputs,
        )
        .await
        .map_err(AppError::from)?;
    Ok(hls.content_url)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HlsResource {
    pub data_base64: String,
    pub content_type: Option<String>,
    pub status: u16,
}

/// Fetch a signed Domand HLS resource for hls.js inside the WebView.
///
/// Linux WebKit/Tauri can fail on direct cross-origin HLS fragment/key loads.
/// Keep this deliberately narrow: only signed Domand delivery/asset URLs are
/// accepted, so the command cannot become a general-purpose local HTTP proxy.
///
/// Domand fronts CloudFront with niconico-side checks that look at
/// `User-Agent` and `Referer`. Without a browser-like UA + a niconico
/// referer the CDN returns 403, even though the URL is signed.
#[tauri::command]
pub async fn fetch_hls_resource(
    url: String,
    range_start: Option<u64>,
    range_end: Option<u64>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<HlsResource> {
    validate_domand_url(&url)?;

    // No automatic gzip decoding: niconico/CloudFront serves segments as
    // raw binary and reqwest's gzip layer was truncating responses to a
    // single byte (likely tripping on a stray `Content-Encoding: gzip`
    // for non-gzipped data, which yields ENOENT-of-gzip-header very fast).
    // No `http1_only()` either — niconico's CDN expects HTTP/2 multiplexing
    // for asset.domand and gets weird with forced 1.1 keep-alive.
    let client = reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
        )
        .build()
        .map_err(crate::error::ApiError::from)?;

    let mut request = client
        .get(&url)
        .header(header::REFERER, "https://www.nicovideo.jp/")
        .header(header::ACCEPT, "*/*")
        .header(header::ACCEPT_LANGUAGE, "ja,en-US;q=0.9,en;q=0.8")
        // Modern Chrome sends these on every fetch. Some CDNs / Lambda@Edge
        // functions look at them as a cheap "is this a real browser?" hint.
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-site")
        .header("sec-fetch-dest", "empty")
        .header(
            "sec-ch-ua",
            "\"Chromium\";v=\"130\", \"Not?A_Brand\";v=\"99\"",
        )
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", "\"Linux\"");
    if let Some(cookie) = store.cookie_header() {
        request = request.header(header::COOKIE, cookie);
    }
    // hls.js's convention: `rangeEnd` is EXCLUSIVE (Python-slice style).
    // Convert to RFC 7233's inclusive form by subtracting 1, and skip the
    // header entirely when the range is empty / degenerate (e.g. 0-0 from
    // an internal hls.js probe). Without this guard CloudFront cheerfully
    // returns `Partial Content size=1` and segments parse to nothing.
    let effective_range = match (range_start, range_end) {
        (Some(start), Some(end)) if end > start => Some(format!("bytes={start}-{}", end - 1)),
        (Some(start), None) => Some(format!("bytes={start}-")),
        // {Some(0), Some(0)} or any empty range → treat as full fetch.
        _ => None,
    };
    if let Some(range) = effective_range.as_ref() {
        request = request.header(header::RANGE, range);
    }

    tracing::debug!(
        %url,
        range_start,
        range_end,
        ?effective_range,
        "fetch_hls_resource"
    );
    let response = request.send().await.map_err(crate::error::ApiError::from)?;
    let status = response.status();
    let response_headers = response.headers().clone();
    let content_type = response_headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(String::from);
    let content_length = response_headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    let content_encoding = response_headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let bytes = response
        .bytes()
        .await
        .map_err(crate::error::ApiError::from)?;
    if let Some(expected) = content_length {
        if (bytes.len() as u64) < expected {
            tracing::warn!(
                %url,
                got = bytes.len(),
                expected,
                ?content_encoding,
                "response body truncated vs Content-Length"
            );
        }
    }

    let head_hex: String = bytes.iter().take(16).map(|b| format!("{b:02x}")).collect();
    let kind = if bytes.len() == 16 {
        "aes-key"
    } else if url.contains("/init") || url.contains("init.cmfv") {
        "init-segment"
    } else if url.contains(".cmfv") || url.contains("/seg") {
        "media-segment"
    } else if url.contains(".m3u8") {
        "playlist"
    } else {
        "other"
    };
    tracing::debug!(
        %url,
        %status,
        size = bytes.len(),
        %head_hex,
        %kind,
        ?content_type,
        "HLS resource fetched"
    );

    if !status.is_success() {
        let preview = String::from_utf8_lossy(&bytes);
        let preview = preview.chars().take(400).collect::<String>();
        let cf_id = response_headers
            .get("x-amz-cf-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let cf_pop = response_headers
            .get("x-amz-cf-pop")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let server = response_headers
            .get(header::SERVER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        tracing::warn!(
            %url,
            %status,
            cf_id,
            cf_pop,
            server,
            body = %preview,
            "HLS resource fetch failed"
        );
        return Err(AppError::Other(format!(
            "HLS resource fetch failed ({status}): {url}"
        )));
    }

    Ok(HlsResource {
        data_base64: BASE64.encode(bytes),
        content_type,
        status: status.as_u16(),
    })
}

fn validate_domand_url(raw: &str) -> Result<()> {
    let url = url::Url::parse(raw).map_err(crate::error::ApiError::from)?;
    if url.scheme() != "https" {
        return Err(AppError::Other("HLS URL must use https".into()));
    }
    let Some(host) = url.host_str() else {
        return Err(AppError::Other("HLS URL is missing a host".into()));
    };
    if matches!(
        host,
        "delivery.domand.nicovideo.jp" | "asset.domand.nicovideo.jp"
    ) {
        Ok(())
    } else {
        tracing::warn!(%host, url = raw, "HLS URL rejected: host not in allowlist");
        Err(AppError::Other(format!("HLS host is not allowed: {host}")))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserVideoItem {
    pub content_id: String,
    pub title: String,
    pub thumbnail_url: Option<String>,
    pub length_seconds: Option<i64>,
    pub view_counter: Option<i64>,
    pub comment_counter: Option<i64>,
    pub mylist_counter: Option<i64>,
    pub start_time: Option<String>,
    pub user_id: Option<i64>,
    pub channel_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserVideosResponse {
    pub total_count: i64,
    pub items: Vec<UserVideoItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_raw: Option<String>,
}

#[tauri::command]
pub async fn fetch_user_videos(
    owner_kind: String,
    owner_id: String,
    page: u32,
    page_size: u32,
    sort_key: String,
    sort_order: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<UserVideosResponse> {
    if owner_id.is_empty()
        || owner_id.len() > 64
        || !owner_id.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(AppError::Other(format!("invalid owner_id: {owner_id:?}")));
    }
    let client = reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
        )
        .build()
        .map_err(crate::error::ApiError::from)?;

    let url = if owner_kind == "channel" {
        format!(
            "https://nvapi.nicovideo.jp/v1/channels/{}/videos?page={}&pageSize={}&sortKey={}&sortOrder={}",
            owner_id, page, page_size, sort_key, sort_order
        )
    } else {
        format!(
            "https://nvapi.nicovideo.jp/v2/users/{}/videos?page={}&pageSize={}&sortKey={}&sortOrder={}",
            owner_id, page, page_size, sort_key, sort_order
        )
    };

    let mut req = client
        .get(&url)
        .header("X-Frontend-Id", "6")
        .header("X-Frontend-Version", "0")
        .header(header::REFERER, "https://www.nicovideo.jp/")
        .header(header::ACCEPT, "application/json");

    if let Some(cookie) = store.cookie_header() {
        req = req.header(header::COOKIE, cookie);
    }

    let resp = req.send().await.map_err(crate::error::ApiError::from)?;
    let status = resp.status();
    let body = resp.text().await.map_err(crate::error::ApiError::from)?;

    if !status.is_success() {
        let preview: String = body.chars().take(200).collect();
        tracing::warn!(%url, %status, body = %preview, "user videos API error");
        return Err(AppError::Other(format!(
            "ユーザー動画 API エラー ({status}): {preview}"
        )));
    }

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(crate::error::ApiError::from)?;

    let preview: String = body.chars().take(500).collect();
    tracing::info!(%url, %status, body = %preview, "user videos API response");

    let total_count = json["data"]["totalCount"]
        .as_i64()
        .or_else(|| json["meta"]["totalCount"].as_i64())
        .unwrap_or(0);

    let items_val = json["data"]["items"]
        .as_array()
        .or_else(|| json["data"]["videosList"]["items"].as_array())
        .or_else(|| json["data"]["videos"].as_array())
        .or_else(|| json["data"]["videoList"]["items"].as_array())
        .cloned()
        .unwrap_or_default();

    let mut items = Vec::with_capacity(items_val.len());
    for raw_item in items_val {
        // NV API wraps video data under "essential" key
        let v = if raw_item["essential"].is_object() {
            &raw_item["essential"]
        } else {
            &raw_item
        };
        let id = v["id"]
            .as_str()
            .or_else(|| v["contentId"].as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let thumb = v["thumbnail"]["url"]
            .as_str()
            .or_else(|| v["thumbnail"]["listingUrl"].as_str())
            .or_else(|| v["thumbnailUrl"].as_str())
            .map(String::from);
        let dur = v["duration"]
            .as_i64()
            .or_else(|| v["lengthSeconds"].as_i64());
        let views = v["count"]["view"]
            .as_i64()
            .or_else(|| v["viewCounter"].as_i64());
        let coms = v["count"]["comment"]
            .as_i64()
            .or_else(|| v["commentCounter"].as_i64());
        let mys = v["count"]["mylist"]
            .as_i64()
            .or_else(|| v["mylistCounter"].as_i64());
        let start = v["registeredAt"]
            .as_str()
            .or_else(|| v["startTime"].as_str())
            .map(String::from);
        let parse_id = |value: &serde_json::Value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok()))
        };
        let uid = parse_id(&v["owner"]["id"]).or_else(|| parse_id(&v["userId"]));
        let cid = if v["owner"]["ownerType"].as_str() == Some("channel")
            || v["owner"]["type"].as_str() == Some("channel")
        {
            parse_id(&v["owner"]["id"])
        } else {
            None
        };
        let title = v["title"].as_str().unwrap_or("(無題)").to_string();
        items.push(UserVideoItem {
            content_id: id,
            title,
            thumbnail_url: thumb,
            length_seconds: dur,
            view_counter: views,
            comment_counter: coms,
            mylist_counter: mys,
            start_time: start,
            user_id: uid,
            channel_id: cid,
        });
    }

    Ok(UserVideosResponse {
        total_count,
        items,
        debug_raw: Some(body.chars().take(2000).collect()),
    })
}

// =================== ダウンロードキュー ===================
//
// 段階1: キュー基盤の CRUD。
// 段階2: `start_download` で実 DL を起動（映像 variant のみを fragmented MP4
// として保存）。音声マージは段階3 以降。

#[tauri::command]
pub async fn enqueue_download(
    video_id: String,
    scheduled_at: Option<i64>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<DownloadQueueItem> {
    validate_video_id(&video_id)?;
    let conn = library.lock().await;
    let item = queue::enqueue(&conn, &video_id, scheduled_at).map_err(AppError::from)?;
    Ok(item)
}

#[tauri::command]
pub async fn list_downloads(
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<Vec<DownloadQueueItem>> {
    let conn = library.lock().await;
    let items = queue::list_all(&conn).map_err(AppError::from)?;
    Ok(items)
}

#[tauri::command]
pub async fn cancel_download(
    id: i64,
    library: State<'_, Arc<LibraryHandle>>,
    tasks: State<'_, DownloadTasks>,
) -> Result<bool> {
    tasks.cancel(id);
    let conn = library.lock().await;
    let removed = queue::cancel(&conn, id).map_err(AppError::from)?;
    Ok(removed > 0)
}

#[tauri::command]
pub async fn clear_finished_downloads(library: State<'_, Arc<LibraryHandle>>) -> Result<usize> {
    let conn = library.lock().await;
    let removed = queue::clear_finished(&conn).map_err(AppError::from)?;
    Ok(removed)
}

/// キューの `id` のジョブを「裏で」走らせる。
///
/// すぐ返って、進捗は `download_queue.progress` の更新で UI に届く。UI 側は
/// `list_downloads` を低頻度ポーリングしている前提。
///
/// - 既に `downloading` の行は再起動しない（多重起動防止）
/// - 出力先: `app_data_dir/videos/{video_id}/video.mp4`
/// - 段階2 仕様により暗号化セグメントは未対応（来 stage 4）
#[tauri::command]
pub async fn start_download(
    id: i64,
    session: State<'_, Arc<crate::api::auth::SessionStore>>,
    library: State<'_, Arc<LibraryHandle>>,
    tasks: State<'_, DownloadTasks>,
    app: tauri::AppHandle,
) -> Result<()> {
    use tauri::Manager;
    let video_id = {
        let conn = library.lock().await;
        let item = queue::get_by_id(&conn, id)
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::Other(format!("queue id {id} not found")))?;
        if item.status == "downloading" {
            return Err(AppError::Other("既に DL 中です".into()));
        }
        // enqueue_download を経由しない経路（旧バージョンが入れた行など）で
        // 不正な ID が DB に入っていた場合に備えて、状態を `downloading` に
        // する前に弾く。後で弾くと、行が `downloading` のまま固まって
        // 「既に DL 中です」で永久に再起動できなくなる（キューデッドロック）。
        validate_video_id(&item.video_id)?;
        queue::mark_status(&conn, id, "downloading").map_err(AppError::from)?;
        // 進捗を 0 に戻す（再試行ケース）
        let _ = queue::update_progress(&conn, id, 0.0);
        item.video_id
    };

    let (cancel_tx, cancel_rx) = watch::channel(false);
    tasks.insert(id, cancel_tx);

    let session = Arc::clone(&session);
    let library = Arc::clone(&library);
    let tasks = tasks.inner().clone();
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("app_data_dir: {e}")))?;
    let app_for_task = app.clone();

    tokio::spawn(async move {
        let result = run_one_download(
            &app_for_task,
            &session,
            &video_id,
            &app_data_dir,
            &library,
            id,
            cancel_rx,
        )
        .await;
        let conn = library.lock().await;
        match result {
            Ok(()) => {
                if let Err(e) = queue::mark_status(&conn, id, "done") {
                    tracing::error!(error = %e, queue_id = id, "failed to mark done");
                }
            }
            Err(e) => {
                let msg = e.to_string();
                let was_canceled = matches!(e, crate::error::ApiError::DownloadCanceled);
                if was_canceled {
                    let _ = tokio::fs::remove_dir_all(app_data_dir.join("videos").join(&video_id))
                        .await;
                }
                tracing::warn!(error = %msg, queue_id = id, video = %video_id, "download failed");
                if let Err(e2) = queue::mark_error(&conn, id, &msg) {
                    tracing::error!(error = %e2, queue_id = id, "failed to mark error");
                }
            }
        }
        tasks.remove(id);
    });

    Ok(())
}

async fn run_one_download(
    app: &tauri::AppHandle,
    session: &Arc<crate::api::auth::SessionStore>,
    video_id: &str,
    app_data_dir: &std::path::Path,
    library: &Arc<LibraryHandle>,
    queue_id: i64,
    cancel: watch::Receiver<bool>,
) -> std::result::Result<(), crate::error::ApiError> {
    use crate::api::comment::CommentApi;
    use crate::api::comment::ThreadsClient;
    use crate::downloader::ytdlp;
    use crate::error::ApiError;

    // 1) yt-dlp に丸投げ。video.mp4 + サムネ + 説明 + info.json を出力。
    //    自前 HLS+AES+ffmpeg より遥かに堅い（niconico 仕様変更追従、
    //    まともな単一 mp4 出力で WebKit が素直に再生できる）。
    let video_dir = app_data_dir.join("videos").join(video_id);
    tokio::fs::create_dir_all(&video_dir).await?;
    let url = format!("https://www.nicovideo.jp/watch/{video_id}");
    let cookie = session.cookie_header();

    let library_for_progress = Arc::clone(library);
    let queue_id_copy = queue_id;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<f64>();
    let progress_handle = tokio::spawn(async move {
        while let Some(pct) = rx.recv().await {
            let conn = library_for_progress.lock().await;
            let _ = queue::update_progress(&conn, queue_id_copy, pct);
        }
    });
    let result = ytdlp::download_with_cancel(
        Some(app),
        &url,
        &video_dir,
        cookie,
        move |p| {
            let _ = tx.send(p);
        },
        cancel,
    )
    .await?;
    let _ = progress_handle.await;

    {
        let conn = library.lock().await;
        if queue::get_by_id(&conn, queue_id)
            .map_err(|e| ApiError::Downloader(format!("queue lookup failed: {e}")))?
            .is_none()
        {
            return Err(ApiError::DownloadCanceled);
        }
    }

    // 2) 出力ファイルを我々の慣例の名前にリネーム。
    // yt-dlp の `video.info.json` は 1-2MB あり、欲しい情報は DB の
    // raw_meta_json に既に保存されるので、ディスクには残さない。
    let final_video_path = video_dir.join("video.mp4"); // yt-dlp が直接ここに出している
    let thumb_path = video_dir.join("thumbnail.jpg");
    let desc_path = video_dir.join("description.txt");
    if let Some(yt_thumb) = result.thumbnail_path.as_deref() {
        let _ = tokio::fs::rename(yt_thumb, &thumb_path).await;
    }
    if let Some(yt_desc) = result.description_path.as_deref() {
        let _ = tokio::fs::rename(yt_desc, &desc_path).await;
    }
    if !final_video_path.exists() {
        return Err(ApiError::Downloader(format!(
            "yt-dlp 完了後に {} が見つかりません",
            final_video_path.display()
        )));
    }
    // info.json は DB に取り込んだ後すぐ削除（後段で読む info_json は
    // yt-dlp の戻り値を使うのでファイル不要）
    let _ = tokio::fs::remove_file(&result.info_path).await;
    // 旧バージョン由来の遺物が残ってたらまとめて掃除しておく
    cleanup_legacy_sidecars(&video_dir).await;

    // 3) コメント取得は yt-dlp に頼らず自前 threads API。
    //    タイミング (vpos_ms) を含む正確な dump が要るため。
    //    watch page 取得 → nv-comment setup → fetch
    let watch = NiconicoWatchClient::new(Arc::clone(session))?;
    let page = watch.fetch_watch_page(video_id).await.ok();
    let comments_dto = if let Some(p) = page.as_ref().and_then(|p| p.nv_comment.as_ref()) {
        let cclient = ThreadsClient::new(Arc::clone(session))?;
        cclient.fetch_comments(p).await.unwrap_or_default()
    } else {
        Vec::new()
    };

    // 4) ライブラリ取り込み用の VideoRecord を組み立てる。
    //    yt-dlp の info.json にも全部入っているが、watch page で取れたなら
    //    そちらを優先（タグや一部メタが充実している）。
    let info = &result.info_json;
    // yt-dlp info の width × height を "1280x720" に。両方取れなければ None。
    let resolution: Option<String> = match (info["width"].as_i64(), info["height"].as_i64()) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some(format!("{w}x{h}")),
        _ => info["resolution"].as_str().map(String::from),
    };
    let video_record = if let Some(p) = page.as_ref() {
        let raw_meta_json = serde_json::to_string(&p.video).ok();
        VideoRecord {
            id: video_id.to_string(),
            title: p.video.title.clone(),
            description: Some(p.video.description.clone()),
            uploader_id: p.owner.as_ref().and_then(|o| o.id.clone()),
            uploader_name: p.owner.as_ref().and_then(|o| o.nickname.clone()),
            uploader_type: p.owner.as_ref().map(|o| o.kind.clone()),
            category: p
                .video
                .tags
                .iter()
                .find(|t| t.is_category)
                .map(|t| t.name.clone()),
            duration_sec: p.video.duration,
            posted_at: p
                .video
                .registered_at
                .as_deref()
                .and_then(parse_iso8601_to_unix),
            view_count: p.video.view_count,
            comment_count: p.video.comment_count,
            mylist_count: p.video.mylist_count,
            thumbnail_url: p.video.thumbnail_url.clone(),
            video_path: Some(format!("videos/{video_id}/video.mp4")),
            raw_meta_json,
            resolution: resolution.clone(),
        }
    } else {
        // watch page が取れなかったケース（fallback）。yt-dlp info.json から組む。
        VideoRecord {
            id: video_id.to_string(),
            title: info["title"].as_str().unwrap_or(video_id).to_string(),
            description: info["description"].as_str().map(String::from),
            uploader_id: info["uploader_id"]
                .as_str()
                .map(String::from)
                .or_else(|| info["channel_id"].as_str().map(String::from)),
            uploader_name: info["uploader"]
                .as_str()
                .map(String::from)
                .or_else(|| info["channel"].as_str().map(String::from)),
            uploader_type: if info["channel_id"].is_string() {
                Some("channel".into())
            } else {
                Some("user".into())
            },
            category: info["categories"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(String::from),
            duration_sec: info["duration"].as_f64().map(|d| d as i64).unwrap_or(0),
            posted_at: info["timestamp"]
                .as_i64()
                .or_else(|| info["release_timestamp"].as_i64())
                .or_else(|| info["upload_date"].as_str().and_then(yt_dlp_date_to_unix)),
            view_count: info["view_count"].as_i64(),
            comment_count: info["comment_count"].as_i64(),
            mylist_count: None,
            thumbnail_url: info["thumbnail"].as_str().map(String::from),
            video_path: Some(format!("videos/{video_id}/video.mp4")),
            raw_meta_json: serde_json::to_string(info).ok(),
            resolution: resolution.clone(),
        }
    };
    let tag_records: Vec<TagRecord> = if let Some(p) = page.as_ref() {
        p.video
            .tags
            .iter()
            .map(|t| TagRecord {
                name: t.name.clone(),
                is_locked: t.is_locked,
            })
            .collect()
    } else {
        info["tags"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|name| TagRecord {
                        name: name.to_string(),
                        is_locked: false,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let comment_records: Vec<CommentRecord> = comments_dto
        .iter()
        .map(|c| CommentRecord {
            no: c.no,
            vpos_ms: c.vpos_ms,
            content: c.content.clone(),
            mail: if c.mail.is_empty() {
                None
            } else {
                Some(c.mail.clone())
            },
            user_hash: c.user_id.clone(),
            is_owner: c.is_owner,
            posted_at: c.posted_at.as_deref().and_then(parse_iso8601_to_unix),
        })
        .collect();

    {
        let mut guard = library.lock().await;
        if queue::get_by_id(&guard, queue_id)
            .map_err(|e| ApiError::Downloader(format!("queue lookup failed: {e}")))?
            .is_none()
        {
            return Err(ApiError::DownloadCanceled);
        }
        videos::ingest_downloaded(
            &mut guard,
            &IngestPayload {
                video: &video_record,
                tags: &tag_records,
                comments: &comment_records,
            },
        )
        .map_err(|e| ApiError::Downloader(format!("library ingest failed: {e}")))?;
    }

    tracing::info!(
        video_id = %video_id,
        comments = comment_records.len(),
        "yt-dlp download finished"
    );
    Ok(())
}

/// 旧バージョンで作られた重い sidecar (video.info.json / meta.json /
/// audio.mp4 / *.track.mp4) があったら削除する。video.mp4 / thumbnail.jpg /
/// description.txt は残す。
async fn cleanup_legacy_sidecars(video_dir: &std::path::Path) {
    for name in [
        "video.info.json",
        "meta.json",
        "audio.mp4",
        "video.track.mp4",
        "audio.track.mp4",
    ] {
        let p = video_dir.join(name);
        if p.exists() {
            if let Err(e) = tokio::fs::remove_file(&p).await {
                tracing::debug!(error = %e, file = %p.display(), "legacy sidecar cleanup");
            }
        }
    }
}

/// yt-dlp の `upload_date` フィールド (YYYYMMDD) を unix epoch (UTC) に。
fn yt_dlp_date_to_unix(yyyymmdd: &str) -> Option<i64> {
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
    let date = NaiveDate::parse_from_str(yyyymmdd, "%Y%m%d").ok()?;
    let dt = NaiveDateTime::new(date, NaiveTime::from_hms_opt(0, 0, 0)?);
    Some(Utc.from_utc_datetime(&dt).timestamp())
}

/// "2024-01-02T03:04:05+09:00" や "2024-01-02T03:04:05Z" を unix epoch (秒) に。
/// 失敗時は None。
fn parse_iso8601_to_unix(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

/// 既存 DL 物の重い sidecar (旧 yt-dlp info.json 等) を一括掃除する。
/// 各動画ディレクトリで video.mp4 / thumbnail.jpg / description.txt 以外を消す。
/// 戻り値は削除したファイルの合計バイト数。
#[tauri::command]
pub async fn cleanup_storage(app: tauri::AppHandle) -> Result<u64> {
    use tauri::Manager;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("app_data_dir: {e}")))?;
    let videos_root = app_data_dir.join("videos");
    if !videos_root.exists() {
        return Ok(0);
    }

    let keep = [
        "video.mp4",
        "thumbnail.jpg",
        "description.txt",
        ".cookies.txt",
    ];
    let mut total_bytes: u64 = 0;
    let mut entries = tokio::fs::read_dir(&videos_root)
        .await
        .map_err(|e| AppError::Other(format!("read videos dir: {e}")))?;
    while let Ok(Some(dir)) = entries.next_entry().await {
        let path = dir.path();
        if !path.is_dir() {
            continue;
        }
        let mut sub = match tokio::fs::read_dir(&path).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        while let Ok(Some(file)) = sub.next_entry().await {
            let fp = file.path();
            let Some(name) = fp.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if keep.contains(&name) {
                continue;
            }
            let size = file.metadata().await.map(|m| m.len()).unwrap_or(0);
            if let Err(e) = tokio::fs::remove_file(&fp).await {
                tracing::debug!(error = %e, file = %fp.display(), "cleanup remove failed");
            } else {
                total_bytes += size;
            }
        }
    }
    Ok(total_bytes)
}

/// ライブラリから 1 動画分を完全削除する。
/// - DB: videos / tags / comment_snapshots / comments / play_history
/// - ディスク: `app_data_dir/videos/{video_id}/` ディレクトリ丸ごと
#[tauri::command]
pub async fn delete_library_video(
    video_id: String,
    library: State<'_, Arc<LibraryHandle>>,
    app: tauri::AppHandle,
) -> Result<()> {
    use tauri::Manager;
    validate_video_id(&video_id)?;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("app_data_dir: {e}")))?;

    {
        let conn = library.lock().await;
        // foreign key cascade で tags/comment_snapshots/comments/play_history は
        // 自動的に消える（schema 上 ON DELETE CASCADE）。
        conn.execute(
            "DELETE FROM videos WHERE id = ?1",
            rusqlite::params![video_id],
        )
        .map_err(|e| AppError::Other(format!("delete videos: {e}")))?;
    }

    let dir = app_data_dir.join("videos").join(&video_id);
    if dir.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
            tracing::warn!(error = %e, dir = %dir.display(), "failed to remove video dir");
        }
    }
    Ok(())
}

/// 内蔵 HTTP サーバ経由のローカル動画 URL を返す。
/// `<video src=...>` にこれを渡すと Range/206 が効いて WebKitGTK でも
/// 後方シークが正しく動く（Blob URL では NG）。
#[tauri::command]
pub fn local_video_url(video_id: String, server: State<'_, LocalServer>) -> Result<String> {
    validate_video_id(&video_id)?;
    Ok(format!(
        "http://127.0.0.1:{}/v/{}/video.mp4",
        server.port, video_id
    ))
}

#[tauri::command]
pub fn local_audio_url(video_id: String, server: State<'_, LocalServer>) -> Result<String> {
    validate_video_id(&video_id)?;
    Ok(format!(
        "http://127.0.0.1:{}/v/{}/audio.mp4",
        server.port, video_id
    ))
}

/// ローカルファイルの中身をバイナリとして JS 側へ返す。
/// `<video>` で `asset://` が使えない WebKitGTK 環境向けに、Blob URL 経由で
/// 再生するためのフォールバック。
///
/// セキュリティ: `app_data_dir` 配下のファイルしか返さない。
#[tauri::command]
pub async fn read_local_file(path: String, app: tauri::AppHandle) -> Result<tauri::ipc::Response> {
    use tauri::Manager;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("app_data_dir: {e}")))?;
    let abs = std::path::PathBuf::from(&path);
    let canonical = abs
        .canonicalize()
        .map_err(|e| AppError::Other(format!("canonicalize {}: {e}", abs.display())))?;
    let canonical_root = app_data_dir
        .canonicalize()
        .map_err(|e| AppError::Other(format!("canonicalize app_data_dir: {e}")))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(AppError::Other(format!(
            "path {} is outside app_data_dir",
            canonical.display()
        )));
    }
    let bytes = tokio::fs::read(&canonical)
        .await
        .map_err(|e| AppError::Other(format!("read {}: {e}", canonical.display())))?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// 既存の `videos/{id}/video.mp4` (+ `audio.mp4`) を ffmpeg で remux し直す。
/// 旧バージョンで DL した CMAF 単独ファイルを `<video>` 互換にしたい時に使う。
#[tauri::command]
pub async fn remux_local_video(video_id: String, app: tauri::AppHandle) -> Result<String> {
    use tauri::Manager;
    validate_video_id(&video_id)?;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("app_data_dir: {e}")))?;
    let dir = app_data_dir.join("videos").join(&video_id);
    let video_path = dir.join("video.mp4");
    let audio_path = dir.join("audio.mp4");
    if !video_path.exists() {
        return Err(AppError::Other(format!(
            "video.mp4 が見つからない: {}",
            video_path.display()
        )));
    }

    // 入力を一旦 .src.mp4 に退避してから ffmpeg で video.mp4 へ書き戻す。
    let src_video = dir.join(".src-video.mp4");
    let src_audio = dir.join(".src-audio.mp4");
    tokio::fs::rename(&video_path, &src_video)
        .await
        .map_err(|e| AppError::Other(format!("rename video.mp4: {e}")))?;
    let audio_arg = if audio_path.exists() {
        tokio::fs::rename(&audio_path, &src_audio)
            .await
            .map_err(|e| AppError::Other(format!("rename audio.mp4: {e}")))?;
        Some(src_audio.as_path())
    } else {
        None
    };

    let outcome =
        crate::downloader::ffmpeg::remux(Some(&app), &src_video, audio_arg, &video_path).await?;
    match outcome {
        crate::downloader::ffmpeg::MuxOutcome::Success => {
            let _ = tokio::fs::remove_file(&src_video).await;
            let _ = tokio::fs::remove_file(&src_audio).await;
            Ok(format!("{} を remux しました", video_id))
        }
        crate::downloader::ffmpeg::MuxOutcome::FfmpegNotFound => {
            // 退避を戻す
            let _ = tokio::fs::rename(&src_video, &video_path).await;
            if audio_arg.is_some() {
                let _ = tokio::fs::rename(&src_audio, &audio_path).await;
            }
            Err(AppError::Other(
                "ffmpeg が PATH に見つかりません。インストールしてから再実行してください。".into(),
            ))
        }
        crate::downloader::ffmpeg::MuxOutcome::FfmpegFailed { stderr } => {
            let _ = tokio::fs::rename(&src_video, &video_path).await;
            if audio_arg.is_some() {
                let _ = tokio::fs::rename(&src_audio, &audio_path).await;
            }
            Err(AppError::Other(format!(
                "ffmpeg 失敗:\n{}",
                stderr.lines().take(20).collect::<Vec<_>>().join("\n")
            )))
        }
    }
}

// =================== ライブラリ閲覧 ===================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryVideoItem {
    pub id: String,
    pub title: String,
    pub duration_sec: i64,
    pub uploader_id: Option<String>,
    pub uploader_name: Option<String>,
    pub view_count: Option<i64>,
    pub posted_at: Option<i64>,
    pub downloaded_at: Option<i64>,
    /// "1280x720" 形式
    pub resolution: Option<String>,
    /// リモート URL (オリジナル)
    pub thumbnail_url: Option<String>,
    /// 絶対パス。フロント側で `convertFileSrc` を通して `<img>` に渡す。
    pub local_thumbnail_path: Option<String>,
    /// 絶対パス。フロント側で `convertFileSrc` を通して `<video>` に渡す。
    pub local_video_path: Option<String>,
    pub tags: Vec<String>,
}

/// ダウンロード済みの動画一覧（`videos.video_path IS NOT NULL` かつ
/// 実ファイルが存在するもの）。ファイルが消えていた行は静かに除外する。
#[tauri::command]
pub async fn list_library_videos(
    library: State<'_, Arc<LibraryHandle>>,
    app: tauri::AppHandle,
) -> Result<Vec<LibraryVideoItem>> {
    use tauri::Manager;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("app_data_dir: {e}")))?;

    let conn = library.lock().await;
    let mut stmt = conn
        .prepare(
            "SELECT id, title, duration_sec, uploader_id, uploader_name, \
                    view_count, posted_at, downloaded_at, thumbnail_url, video_path, resolution \
             FROM videos \
             WHERE video_path IS NOT NULL \
             ORDER BY downloaded_at DESC, id DESC",
        )
        .map_err(|e| AppError::Other(format!("prepare videos: {e}")))?;
    let mut items: Vec<LibraryVideoItem> = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let video_path: Option<String> = row.get(9)?;
            let resolution: Option<String> = row.get(10)?;
            let local_video_abs = video_path
                .as_deref()
                .map(|p| app_data_dir.join(p).to_string_lossy().into_owned());
            let local_thumb_abs = {
                let p = app_data_dir.join("videos").join(&id).join("thumbnail.jpg");
                if p.exists() {
                    Some(p.to_string_lossy().into_owned())
                } else {
                    None
                }
            };
            Ok(LibraryVideoItem {
                id,
                title: row.get(1)?,
                duration_sec: row.get(2)?,
                uploader_id: row.get(3)?,
                uploader_name: row.get(4)?,
                view_count: row.get(5)?,
                posted_at: row.get(6)?,
                downloaded_at: row.get(7)?,
                resolution,
                thumbnail_url: row.get(8)?,
                local_thumbnail_path: local_thumb_abs,
                local_video_path: local_video_abs,
                tags: Vec::new(),
            })
        })
        .map_err(|e| AppError::Other(format!("query videos: {e}")))?
        .filter_map(|r| r.ok())
        // ファイルが消えてる行はライブラリから見せない（DB は残す。
        // delete_library_video で明示的に消した時のみ DB も clear する）
        .filter(|item| {
            item.local_video_path
                .as_deref()
                .map(|p| std::path::Path::new(p).exists())
                .unwrap_or(false)
        })
        .collect();

    // タグを 1 クエリで埋める（N+1 を避ける）
    let ids: Vec<&str> = items.iter().map(|v| v.id.as_str()).collect();
    if !ids.is_empty() {
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT video_id, name FROM tags WHERE video_id IN ({placeholders}) \
             ORDER BY video_id, name"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| AppError::Other(format!("prepare tags: {e}")))?;
        let mut by_video: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        let rows = stmt
            .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| AppError::Other(format!("query tags: {e}")))?;
        for r in rows.flatten() {
            by_video.entry(r.0).or_default().push(r.1);
        }
        for item in items.iter_mut() {
            if let Some(t) = by_video.remove(&item.id) {
                item.tags = t;
            }
        }
    }

    Ok(items)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPlayerComment {
    pub id: String,
    pub no: i64,
    pub vpos_ms: i64,
    pub content: String,
    pub mail: String,
    pub commands: Vec<String>,
    pub user_id: Option<String>,
    pub posted_at: Option<String>,
    pub fork: String,
    pub is_owner: bool,
    pub nicoru_count: Option<i64>,
    pub score: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPlaybackPayload {
    pub video_id: String,
    pub title: String,
    pub description: Option<String>,
    pub duration_sec: i64,
    pub uploader_id: Option<String>,
    pub uploader_name: Option<String>,
    pub uploader_type: Option<String>,
    pub view_count: Option<i64>,
    pub comment_count: Option<i64>,
    pub mylist_count: Option<i64>,
    pub posted_at: Option<i64>,
    pub thumbnail_url: Option<String>,
    pub tags: Vec<LibraryTag>,
    /// 絶対パス。フロント側で `convertFileSrc` を通す。
    pub local_video_path: String,
    /// 音声 fMP4 が別ファイルである場合の絶対パス。dual-element 同期再生に使う。
    pub local_audio_path: Option<String>,
    pub local_thumbnail_path: Option<String>,
    pub comments: Vec<LocalPlayerComment>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryTag {
    pub name: String,
    pub is_locked: bool,
}

/// ローカルに DL 済みの動画がある場合のみ Some を返す。
/// 無ければ呼び出し側は `prepare_playback` (HLS) にフォールバックする。
#[tauri::command]
pub async fn prepare_local_playback(
    video_id: String,
    library: State<'_, Arc<LibraryHandle>>,
    app: tauri::AppHandle,
) -> Result<Option<LocalPlaybackPayload>> {
    use tauri::Manager;
    validate_video_id(&video_id)?;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("app_data_dir: {e}")))?;

    let conn = library.lock().await;

    let video_row = conn
        .query_row(
            "SELECT id, title, description, duration_sec, uploader_id, uploader_name, uploader_type, \
                    view_count, comment_count, mylist_count, posted_at, thumbnail_url, video_path \
             FROM videos WHERE id = ?1",
            rusqlite::params![video_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                ))
            },
        )
        .ok();
    let Some(row) = video_row else {
        return Ok(None);
    };
    let Some(video_rel_path) = row.12 else {
        return Ok(None);
    };

    let abs_video = app_data_dir.join(&video_rel_path);
    if !abs_video.exists() {
        return Ok(None);
    }
    let abs_audio = {
        let p = app_data_dir.join("videos").join(&row.0).join("audio.mp4");
        if p.exists() {
            Some(p.to_string_lossy().into_owned())
        } else {
            None
        }
    };
    let thumb_abs = {
        let p = app_data_dir
            .join("videos")
            .join(&row.0)
            .join("thumbnail.jpg");
        if p.exists() {
            Some(p.to_string_lossy().into_owned())
        } else {
            None
        }
    };

    // タグ
    let mut tag_stmt = conn
        .prepare("SELECT name, is_locked FROM tags WHERE video_id = ?1")
        .map_err(|e| AppError::Other(format!("prepare tags: {e}")))?;
    let tags: Vec<LibraryTag> = tag_stmt
        .query_map(rusqlite::params![video_id], |row| {
            Ok(LibraryTag {
                name: row.get::<_, String>(0)?,
                is_locked: row.get::<_, i64>(1)? != 0,
            })
        })
        .map_err(|e| AppError::Other(format!("query tags: {e}")))?
        .filter_map(|r| r.ok())
        .collect();

    // 最新の snapshot のコメント
    let snap_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM comment_snapshots WHERE video_id = ?1 \
             ORDER BY taken_at DESC, id DESC LIMIT 1",
            rusqlite::params![video_id],
            |row| row.get(0),
        )
        .ok();
    let comments: Vec<LocalPlayerComment> = if let Some(sid) = snap_id {
        let mut stmt = conn
            .prepare(
                "SELECT id, no, vpos_ms, content, mail, user_hash, is_owner, posted_at \
                 FROM comments WHERE snapshot_id = ?1 ORDER BY vpos_ms ASC",
            )
            .map_err(|e| AppError::Other(format!("prepare comments: {e}")))?;
        let rows = stmt
            .query_map(rusqlite::params![sid], |r| {
                let mail: Option<String> = r.get(4)?;
                let mail_str = mail.unwrap_or_default();
                let commands: Vec<String> =
                    mail_str.split_whitespace().map(|s| s.to_string()).collect();
                let is_owner = r.get::<_, i64>(6)? != 0;
                // niconicomments は fork="owner" / "main" / "easy" でスレを
                // 分けて挙動を変える。投稿者コメは必ず "owner" にしないと
                // 時間描画 / レイアウトが崩れる。
                let fork = if is_owner { "owner" } else { "main" };
                Ok(LocalPlayerComment {
                    id: r.get::<_, i64>(0)?.to_string(),
                    no: r.get(1)?,
                    vpos_ms: r.get(2)?,
                    content: r.get(3)?,
                    mail: mail_str,
                    commands,
                    user_id: r.get(5)?,
                    posted_at: r.get::<_, Option<i64>>(7)?.map(|t| t.to_string()),
                    fork: fork.to_string(),
                    is_owner,
                    nicoru_count: None,
                    score: None,
                })
            })
            .map_err(|e| AppError::Other(format!("query comments: {e}")))?;
        let collected: Vec<LocalPlayerComment> = rows.filter_map(|r| r.ok()).collect();
        collected
    } else {
        Vec::new()
    };

    Ok(Some(LocalPlaybackPayload {
        video_id: row.0,
        title: row.1,
        description: row.2,
        duration_sec: row.3,
        uploader_id: row.4,
        uploader_name: row.5,
        uploader_type: row.6,
        view_count: row.7,
        comment_count: row.8,
        mylist_count: row.9,
        posted_at: row.10,
        thumbnail_url: row.11,
        tags,
        local_video_path: abs_video.to_string_lossy().into_owned(),
        local_audio_path: abs_audio,
        local_thumbnail_path: thumb_abs,
        comments,
    }))
}

// =================== ライブラリ検索・整列・集計 ===================

#[tauri::command]
pub async fn query_library_videos(
    q: LibraryQuery,
    library: State<'_, Arc<LibraryHandle>>,
    app: tauri::AppHandle,
) -> Result<QueryResult> {
    use tauri::Manager;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("app_data_dir: {e}")))?;
    let conn = library.lock().await;
    let mut result = query::query_videos(&conn, &q).map_err(AppError::from)?;
    for item in &mut result.items {
        let thumb = app_data_dir
            .join("videos")
            .join(&item.id)
            .join("thumbnail.jpg");
        if thumb.exists() {
            item.local_thumbnail_path = Some(thumb.to_string_lossy().into_owned());
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn get_library_stats(library: State<'_, Arc<LibraryHandle>>) -> Result<LibraryStats> {
    let conn = library.lock().await;
    let stats = query::get_stats(&conn).map_err(AppError::from)?;
    Ok(stats)
}

#[tauri::command]
pub async fn list_library_tags(library: State<'_, Arc<LibraryHandle>>) -> Result<Vec<String>> {
    let conn = library.lock().await;
    let tags = query::list_all_tags(&conn).map_err(AppError::from)?;
    Ok(tags)
}

#[tauri::command]
pub async fn list_library_resolutions(
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<Vec<String>> {
    let conn = library.lock().await;
    let resolutions = query::list_resolutions(&conn).map_err(AppError::from)?;
    Ok(resolutions)
}

// =================== 設定 ===================

#[tauri::command]
pub async fn get_settings(
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<std::collections::HashMap<String, String>> {
    let conn = library.lock().await;
    settings::get_all(&conn).map_err(AppError::from)
}

#[tauri::command]
pub async fn set_setting(
    key: String,
    value: String,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<()> {
    let conn = library.lock().await;
    settings::set(&conn, &key, &value).map_err(AppError::from)
}

#[tauri::command]
pub async fn delete_setting(key: String, library: State<'_, Arc<LibraryHandle>>) -> Result<()> {
    let conn = library.lock().await;
    settings::delete(&conn, &key).map_err(AppError::from)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub identifier: String,
    pub data_dir: String,
    pub videos_dir: String,
    pub db_path: String,
    pub local_server_port: u16,
    pub ytdlp_available: bool,
    pub ytdlp_version: Option<String>,
    /// "bundled" / "sidecar" / "system_path" / "not_found"
    pub ytdlp_source: String,
    pub ytdlp_path: String,
    pub ffmpeg_available: bool,
    pub ffmpeg_version: Option<String>,
    pub ffmpeg_source: String,
    pub ffmpeg_path: String,
    pub library_video_count: i64,
    pub library_videos_size_bytes: u64,
}

#[tauri::command]
pub async fn get_app_info(
    library: State<'_, Arc<LibraryHandle>>,
    server: State<'_, LocalServer>,
    app: tauri::AppHandle,
) -> Result<AppInfo> {
    use tauri::Manager;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("app_data_dir: {e}")))?;
    let videos_dir = app_data_dir.join("videos");
    let db_path = app_data_dir.join("library.db");

    let yt = crate::downloader::tools::ytdlp(Some(&app));
    let ff = crate::downloader::tools::ffmpeg(Some(&app));
    let (ytdlp_available, ytdlp_version) = check_tool_version(&yt.command, "--version").await;
    let (ffmpeg_available, ffmpeg_version) = check_tool_version(&ff.command, "-version").await;
    let yt_source = match yt.source {
        crate::downloader::tools::BinarySource::Bundled => "bundled",
        crate::downloader::tools::BinarySource::Sidecar => "sidecar",
        crate::downloader::tools::BinarySource::SystemPath => "system_path",
        crate::downloader::tools::BinarySource::NotFound => "not_found",
    };
    let ff_source = match ff.source {
        crate::downloader::tools::BinarySource::Bundled => "bundled",
        crate::downloader::tools::BinarySource::Sidecar => "sidecar",
        crate::downloader::tools::BinarySource::SystemPath => "system_path",
        crate::downloader::tools::BinarySource::NotFound => "not_found",
    };

    let (count, size) = {
        let conn = library.lock().await;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM videos WHERE video_path IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        drop(conn);
        let size = dir_size(&videos_dir).await;
        (count, size)
    };

    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        identifier: "in.yajuvideo.nndd-next".to_string(),
        data_dir: app_data_dir.to_string_lossy().into_owned(),
        videos_dir: videos_dir.to_string_lossy().into_owned(),
        db_path: db_path.to_string_lossy().into_owned(),
        local_server_port: server.port,
        ytdlp_available,
        ytdlp_version,
        ytdlp_source: yt_source.to_string(),
        ytdlp_path: yt.command,
        ffmpeg_available,
        ffmpeg_version,
        ffmpeg_source: ff_source.to_string(),
        ffmpeg_path: ff.command,
        library_video_count: count,
        library_videos_size_bytes: size,
    })
}

async fn check_tool_version(cmd: &str, version_arg: &str) -> (bool, Option<String>) {
    match tokio::process::Command::new(cmd)
        .arg(version_arg)
        .output()
        .await
    {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout);
            let first_line = s.lines().next().unwrap_or("").trim().to_string();
            (
                true,
                if first_line.is_empty() {
                    None
                } else {
                    Some(first_line)
                },
            )
        }
        _ => (false, None),
    }
}

/// ディレクトリの累計バイト数（再帰）。失敗時は 0。
async fn dir_size(path: &std::path::Path) -> u64 {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || sync_dir_size(&path))
        .await
        .unwrap_or(0)
}

fn sync_dir_size(path: &std::path::Path) -> u64 {
    let mut total: u64 = 0;
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if let Ok(meta) = entry.metadata() {
            if meta.is_dir() {
                total = total.saturating_add(sync_dir_size(&p));
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_video_id_accepts_niconico_ids() {
        assert!(validate_video_id("sm12345").is_ok());
        assert!(validate_video_id("nm67890").is_ok());
        assert!(validate_video_id("so11111").is_ok());
        assert!(validate_video_id("watch_id-1_2-3").is_ok());
    }

    #[test]
    fn validate_video_id_rejects_path_traversal() {
        assert!(validate_video_id("..").is_err());
        assert!(validate_video_id("../etc").is_err());
        assert!(validate_video_id("../../foo").is_err());
        assert!(validate_video_id("a/b").is_err());
        assert!(validate_video_id("a\\b").is_err());
        assert!(validate_video_id("").is_err());
        assert!(validate_video_id(".").is_err());
        assert!(validate_video_id("foo bar").is_err());
        assert!(validate_video_id("foo\0bar").is_err());
    }

    #[test]
    fn validate_video_id_rejects_overlong() {
        let long = "a".repeat(65);
        assert!(validate_video_id(&long).is_err());
    }
}
