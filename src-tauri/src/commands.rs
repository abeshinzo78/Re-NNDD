//! Tauri invoke handlers. Thin glue between the frontend and the Rust modules;
//! domain logic lives under `api/`, `library/`, etc.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use reqwest::header;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::watch;

use crate::api::auth::{login_with_password, LoginOutcome, SessionStore};
use crate::api::comment::{Comment, CommentApi, ThreadsClient};
use crate::api::search::{NvapiSearchClient, SearchApi, SnapshotSearchClient};
use crate::api::types::{SearchQuery, SearchResponse};
use crate::api::video::{
    json_value_as_id_string, quality_candidates, NiconicoWatchClient, NvCommentSetup, SeriesInfo,
    WatchApi, WatchOwner, WatchVideoMeta,
};
use crate::downloader::tools;
use crate::downloader::ytdlp as ytdlp_mod;
use crate::error::{AppError, Result};
use crate::library::db::LibraryHandle;
use crate::library::query::{self, LibraryQuery, LibraryStats, QueryResult};
use crate::library::queue::{self, DownloadQueueItem};
use crate::library::settings;
use crate::library::snapshots;
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

/// niconico の nvapi 系エンドポイントは全部このブラウザ UA を期待する。
/// UA が `reqwest/...` のままだと一部レスポンスが空配列で返ってくる。
const NV_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// owner_id / user_id の入力検証。空・長すぎ・英数字以外は弾く。
fn validate_owner_id(owner_id: &str) -> std::result::Result<(), AppError> {
    if owner_id.is_empty()
        || owner_id.len() > 64
        || !owner_id.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(AppError::Other(format!("invalid owner_id: {owner_id:?}")));
    }
    Ok(())
}

/// niconico nvapi/HTML 取得用の `reqwest::Client` を作る。
fn build_nv_client() -> std::result::Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .user_agent(NV_USER_AGENT)
        .build()
        .map_err(crate::error::ApiError::from)
        .map_err(AppError::from)
}

/// 既に解決済みの `content_id` / `user_id` / `channel_id` と JSON ノードから
/// `UserVideoItem` を組み立てる。タイトル/サムネ/カウンタ系は呼び出し側 3 箇所
/// (ユーザー動画 API / マイリスト動画 API / シリーズ HTML スクレイプ) で全く
/// 同じパスを舐めているのでここに集約する。`id` 自体は API ごとに型/フィールド
/// 名/フォールバック先が違うので呼び出し側で抽出する責務にしている。
fn build_user_video_item(
    v: &serde_json::Value,
    content_id: String,
    user_id: Option<i64>,
    channel_id: Option<i64>,
) -> UserVideoItem {
    UserVideoItem {
        content_id,
        title: v["title"].as_str().unwrap_or("(無題)").to_string(),
        thumbnail_url: v["thumbnail"]["url"]
            .as_str()
            .or_else(|| v["thumbnail"]["listingUrl"].as_str())
            .or_else(|| v["thumbnailUrl"].as_str())
            .map(String::from),
        length_seconds: v["duration"]
            .as_i64()
            .or_else(|| v["lengthSeconds"].as_i64()),
        view_counter: v["count"]["view"]
            .as_i64()
            .or_else(|| v["viewCounter"].as_i64()),
        comment_counter: v["count"]["comment"]
            .as_i64()
            .or_else(|| v["commentCounter"].as_i64()),
        mylist_counter: v["count"]["mylist"]
            .as_i64()
            .or_else(|| v["mylistCounter"].as_i64()),
        start_time: v["registeredAt"]
            .as_str()
            .or_else(|| v["startTime"].as_str())
            .map(String::from),
        user_id,
        channel_id,
    }
}

/// `nvapi /v1/users/{id}/{kind}?page=...&pageSize=...` 系の「ページング付き一覧」
/// レスポンスから `(items_array, total_count)` を取り出す。
/// items は `data.{primary_array}` を優先し、無ければ `data.items` を見る
/// (niconico の API バージョン差を吸収するための fallback)。
async fn nv_fetch_paged_list(
    client: &reqwest::Client,
    url: &str,
    cookie: Option<String>,
    err_label: &str,
    primary_array: &str,
) -> Result<(Vec<serde_json::Value>, i64)> {
    let (json, _body) = nv_get_json(client, url, cookie, err_label).await?;
    let total_count = json["data"]["totalCount"].as_i64().unwrap_or(0);
    let items_val = json["data"][primary_array]
        .as_array()
        .or_else(|| json["data"]["items"].as_array())
        .cloned()
        .unwrap_or_default();
    Ok((items_val, total_count))
}

/// nvapi.nicovideo.jp 系エンドポイントへ GET し、`(parsed_json, body_text)` を返す。
/// 失敗時は `err_label` を含む `AppError::Other` を返す。`body_text` はデバッグ用に
/// プレビューしたい呼び出し側のために生で渡す。
async fn nv_get_json(
    client: &reqwest::Client,
    url: &str,
    cookie: Option<String>,
    err_label: &str,
) -> Result<(serde_json::Value, String)> {
    let mut req = client
        .get(url)
        .header("X-Frontend-Id", "6")
        .header("X-Frontend-Version", "0")
        .header(header::REFERER, "https://www.nicovideo.jp/")
        .header(header::ACCEPT, "application/json");

    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }

    let resp = req.send().await.map_err(crate::error::ApiError::from)?;
    let status = resp.status();
    let body = resp.text().await.map_err(crate::error::ApiError::from)?;

    if !status.is_success() {
        let preview: String = body.chars().take(200).collect();
        tracing::warn!(%url, %status, body = %preview, "{err_label}");
        return Err(AppError::Other(format!(
            "{err_label} ({status}): {preview}"
        )));
    }

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(crate::error::ApiError::from)?;
    Ok((json, body))
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

/// オンライン動画検索。`engine` で検索エンジンを切り替える:
/// - `"snapshot"` (既定): 公開スナップショット検索 API v2。認証不要・日次更新。
/// - `"nvapi"`: niconico Web クライアントと同じ内部検索 API。ログイン中は
///   保存済みセッション Cookie を付けて呼ぶため、結果が視聴者アカウントに
///   追従する (センシティブ表示など)。
#[tauri::command]
pub async fn search_videos_online(
    query: SearchQuery,
    engine: Option<String>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<SearchResponse> {
    let response = match engine.as_deref() {
        Some("nvapi") => {
            // ログイン済みならセッション Cookie を付けて認証付き検索にする。
            let client = NvapiSearchClient::new(store.cookie_header()).map_err(AppError::from)?;
            client.search(&query).await.map_err(AppError::from)?
        }
        _ => {
            let client = SnapshotSearchClient::new().map_err(AppError::from)?;
            client.search(&query).await.map_err(AppError::from)?
        }
    };
    Ok(response)
}

#[derive(Debug, Clone, Serialize)]
pub struct RelatedVideoItem {
    #[serde(rename = "contentId")]
    pub content_id: Option<String>,
    pub title: Option<String>,
    #[serde(rename = "viewCounter")]
    pub view_counter: Option<i64>,
    #[serde(rename = "commentCounter")]
    pub comment_counter: Option<i64>,
    #[serde(rename = "mylistCounter")]
    pub mylist_counter: Option<i64>,
    #[serde(rename = "lengthSeconds")]
    pub length_seconds: Option<i64>,
    #[serde(rename = "thumbnailUrl")]
    pub thumbnail_url: Option<String>,
    #[serde(rename = "startTime")]
    pub start_time: Option<String>,
    #[serde(rename = "userId")]
    pub user_id: Option<i64>,
    #[serde(rename = "channelId")]
    pub channel_id: Option<i64>,
}

#[tauri::command]
pub async fn fetch_related_videos(
    video_id: String,
    limit: Option<i32>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<Vec<RelatedVideoItem>> {
    let limit = limit.unwrap_or(12).min(50);
    let client = build_nv_client()?;
    let cookie = store.cookie_header();
    let has_cookie = cookie.is_some();

    let url = format!(
        "https://nvapi.nicovideo.jp/v1/recommend?recipeId=video_watch_recommendation&videoId={video_id}&site=nicovideo&frontendId=6&frontendVersion=0&limit={limit}"
    );

    let mut req = client
        .get(&url)
        .header("X-Frontend-Id", "6")
        .header("X-Frontend-Version", "0")
        .header("X-Niconico-Language", "ja-jp")
        .header("X-Request-With", "https://www.nicovideo.jp")
        .header(header::REFERER, "https://www.nicovideo.jp/")
        .header(header::ACCEPT, "application/json;charset=utf-8");

    if let Some(ref c) = cookie {
        req = req.header(header::COOKIE, c.as_str());
    }

    tracing::debug!(%url, %has_cookie, "fetch_related_videos request");

    let resp = req.send().await.map_err(crate::error::ApiError::from)?;
    let status = resp.status();
    let body = resp.text().await.map_err(crate::error::ApiError::from)?;

    if !status.is_success() {
        let preview: String = body.chars().take(300).collect();
        tracing::warn!(%url, %status, body = %preview, "fetch_related_videos");
        return Err(AppError::Other(format!(
            "fetch_related_videos ({status}): {preview}"
        )));
    }

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(crate::error::ApiError::from)?;

    parse_related_videos(json)
}

fn parse_related_videos(json: serde_json::Value) -> Result<Vec<RelatedVideoItem>> {
    let items = json.pointer("/data/items").and_then(|v| v.as_array());
    let videos: Vec<RelatedVideoItem> = items
        .map(|arr| {
            arr.iter()
                .filter(|item| {
                    item.get("contentType")
                        .and_then(|v| v.as_str())
                        .map(|t| t == "video")
                        .unwrap_or(false)
                })
                .filter_map(|item| {
                    let content = item.get("content")?;
                    let content_id = content.get("id").and_then(|v| v.as_str()).map(String::from);
                    let title = content
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let view_counter = content
                        .get("count")
                        .and_then(|c| c.get("view"))
                        .and_then(|v| v.as_i64());
                    let comment_counter = content
                        .get("count")
                        .and_then(|c| c.get("comment"))
                        .and_then(|v| v.as_i64());
                    let mylist_counter = content
                        .get("count")
                        .and_then(|c| c.get("mylist"))
                        .and_then(|v| v.as_i64());
                    let length_seconds = content.get("duration").and_then(|v| v.as_i64());
                    // `url` を最優先する。`listingUrl` は古い動画だと
                    // `img.cdn.nimg.jp/...?key=...` の署名付き(失効しうる)URL に
                    // なる事があり「たまに表示されない」原因になるため、安定した
                    // `url` を先に見る (build_user_video_item / ランキングと同順序)。
                    let thumbnail_url = content
                        .get("thumbnail")
                        .and_then(|t| {
                            t.get("url")
                                .or_else(|| t.get("listingUrl"))
                                .or_else(|| t.get("middleUrl"))
                        })
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let start_time = content
                        .get("registeredAt")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let user_id = content
                        .get("owner")
                        .and_then(|o| o.get("id"))
                        .and_then(|v| v.as_str())
                        .and_then(|raw| raw.parse::<i64>().ok());
                    let channel_id =
                        content
                            .get("channelId")
                            .and_then(|v| v.as_i64())
                            .or_else(|| {
                                content
                                    .get("owner")
                                    .and_then(|o| o.get("channelId"))
                                    .and_then(|v| v.as_i64())
                            });

                    Some(RelatedVideoItem {
                        content_id,
                        title,
                        view_counter,
                        comment_counter,
                        mylist_counter,
                        length_seconds,
                        thumbnail_url,
                        start_time,
                        user_id,
                        channel_id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(videos)
}

/// `getthumbinfo` XML から `<thumbnail_url>…</thumbnail_url>` を抜き出す。
/// 削除/非公開動画は `status="fail"` で本要素が無いので `None` を返す。
fn parse_thumbnail_url_from_xml(xml: &str) -> Option<String> {
    let start_tag = "<thumbnail_url>";
    let end_tag = "</thumbnail_url>";
    let start = xml.find(start_tag)? + start_tag.len();
    let end = xml[start..].find(end_tag)? + start;
    let url = xml[start..end].trim();
    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}

/// 動画 ID から「現行の」サムネイル URL を権威的に再解決する。
///
/// フロントの `<img>` は API/履歴/ライブラリに保存済みの URL をそのまま貼るが、
/// 投稿者がサムネを差し替えるとハッシュ付き URL (`{id}.{hash}`) に変わって旧 URL が
/// 404 になったり、署名付き URL の鍵が失効したりして「たまに表示されない」。
/// その時にこのコマンドで `getthumbinfo`(権威ソース)から現行 URL を取り直す。
/// セッション Cookie を付けてセンシティブ/会員向けの判定も通す。
#[tauri::command]
pub async fn resolve_thumbnail_url(
    video_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<Option<String>> {
    validate_video_id(&video_id)?;
    let client = build_nv_client()?;
    let url = format!("https://ext.nicovideo.jp/api/getthumbinfo/{video_id}");

    let mut req = client.get(&url).header(header::ACCEPT, "application/xml");
    if let Some(c) = store.cookie_header() {
        req = req.header(header::COOKIE, c);
    }

    let resp = req.send().await.map_err(crate::error::ApiError::from)?;
    let status = resp.status();
    let body = resp.text().await.map_err(crate::error::ApiError::from)?;

    if !status.is_success() {
        let preview: String = body.chars().take(200).collect();
        tracing::warn!(%url, %status, body = %preview, "resolve_thumbnail_url");
        return Err(AppError::Other(format!("サムネ解決 API エラー ({status})")));
    }

    Ok(parse_thumbnail_url_from_xml(&body))
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

#[tauri::command]
pub async fn login_mfa(
    mfa_session: String,
    one_time_password: String,
    store: State<'_, Arc<SessionStore>>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<LoginResult> {
    let outcome = crate::api::auth::login_mfa(&mfa_session, &one_time_password)
        .await
        .map_err(AppError::from)?;
    match outcome {
        LoginOutcome::Success { user_session } => {
            let conn = library.lock().await;
            store.set_with_conn(user_session, &conn);
            Ok(LoginResult::Success)
        }
        LoginOutcome::Mfa { .. } => Ok(LoginResult::Mfa {
            mfa_session: Some(mfa_session),
        }),
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
    pub series: Option<SeriesInfo>,
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
    pub is_short: bool,
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

    let is_short = page.video.content_type.as_deref() == Some("short");

    Ok(PlaybackPayload {
        video: page.video,
        owner: page.owner,
        series: page.series,
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
        is_short,
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

/// Fetch a signed Domand HLS resource for hls.js inside the WebView.
///
/// Linux WebKit/Tauri can fail on direct cross-origin HLS fragment/key loads.
/// Keep this deliberately narrow: only signed Domand delivery/asset URLs are
/// accepted, so the command cannot become a general-purpose local HTTP proxy.
///
/// Domand fronts CloudFront with niconico-side checks that look at
/// `User-Agent` and `Referer`. Without a browser-like UA + a niconico
/// referer the CDN returns 403, even though the URL is signed.
///
/// The body is returned as a raw [`tauri::ipc::Response`] (an `ArrayBuffer`
/// on the JS side), not a base64-in-JSON envelope: HLS segments are multi-MB
/// and fetched continuously during playback, which is exactly the "large
/// download HTTP response" case the Tauri docs say to stream as raw bytes
/// rather than serialize to JSON. See `read_local_file` for the same pattern.
#[tauri::command]
pub async fn fetch_hls_resource(
    url: String,
    range_start: Option<u64>,
    range_end: Option<u64>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<tauri::ipc::Response> {
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

    // Hand the bytes straight to the IPC layer as a raw buffer. `status` and
    // `content_type` are intentionally not forwarded: they were only ever used
    // for logging (above) and as informational fields on the hls.js loader
    // success callback. The loader reports a synthetic 200 on success, and
    // hls.js derives the payload kind from the URL / responseType, not from
    // the Content-Type header.
    Ok(tauri::ipc::Response::new(bytes.to_vec()))
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_thumbnail_url: Option<String>,
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
    validate_owner_id(&owner_id)?;
    let client = build_nv_client()?;

    let url = if owner_kind == "channel" {
        format!(
            "https://nvapi.nicovideo.jp/v1/channels/{}/videos?page={}&pageSize={}&sortKey={}&sortOrder={}",
            owner_id, page, page_size, sort_key, sort_order
        )
    } else {
        // v3 endpoint returns items wrapped in { essential: …, series: … }
        format!(
            "https://nvapi.nicovideo.jp/v3/users/{}/videos?page={}&pageSize={}&sortKey={}&sortOrder={}&sensitiveContents=mask",
            owner_id, page, page_size, sort_key, sort_order
        )
    };

    let (json, body) = nv_get_json(
        &client,
        &url,
        store.cookie_header(),
        "ユーザー動画 API エラー",
    )
    .await?;

    let preview: String = body.chars().take(500).collect();
    tracing::info!(%url, body = %preview, "user videos API response");

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
        // ユーザー動画 API は user/channel id が string で返るケースがあるので
        // i64 / 数字文字列の双方を受ける lax 抽出を使う。
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
        items.push(build_user_video_item(v, id, uid, cid));
    }

    Ok(UserVideosResponse {
        total_count,
        items,
        debug_raw: Some(body.chars().take(2000).collect()),
        series_title: None,
        series_description: None,
        series_thumbnail_url: None,
    })
}

#[tauri::command]
pub async fn fetch_series_videos(
    series_id: String,
    _page: u32,
    _page_size: u32,
    store: State<'_, Arc<SessionStore>>,
) -> Result<UserVideosResponse> {
    let client = build_nv_client()?;

    // Step 1: get series metadata from NV API. メタ取得は失敗しても致命的じゃ
    // ないので 4xx でも Null で先へ進む (Step2/3 が動画一覧を別経路で取りに行く)。
    let meta_url = format!("https://nvapi.nicovideo.jp/v1/series/{series_id}");
    let meta_json = match nv_get_json(
        &client,
        &meta_url,
        store.cookie_header(),
        "シリーズメタ API エラー",
    )
    .await
    {
        Ok((j, _)) => j,
        Err(_) => serde_json::Value::Null,
    };

    let series_title = meta_json["data"]["detail"]["title"]
        .as_str()
        .map(String::from);
    let series_description = meta_json["data"]["detail"]["description"]
        .as_str()
        .map(String::from);
    let series_thumbnail_url = meta_json["data"]["detail"]["thumbnailUrl"]
        .as_str()
        .or_else(|| meta_json["data"]["detail"]["thumbnail"]["url"].as_str())
        .map(String::from);

    // Step 2: try yt-dlp for video list (most reliable)
    let cookie = store.cookie_header();
    match fetch_series_videos_via_ytdlp(&series_id, cookie).await {
        Ok(items) if !items.is_empty() => {
            let total_count = items.len() as i64;
            return Ok(UserVideosResponse {
                total_count,
                items,
                debug_raw: None,
                series_title,
                series_description,
                series_thumbnail_url,
            });
        }
        Ok(_) => {
            tracing::info!(
                "yt-dlp returned empty list for series {}, trying HTML scrape",
                series_id
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "yt-dlp failed for series {}, trying HTML scrape", series_id);
        }
    }

    // Step 3: fallback — scrape series HTML page for video list
    let html_url = format!("https://www.nicovideo.jp/series/{series_id}");
    let mut html_req = client
        .get(&html_url)
        .header(header::ACCEPT, "text/html,application/xhtml+xml");

    if let Some(cookie) = store.cookie_header() {
        html_req = html_req.header(header::COOKIE, cookie);
    }

    let html_resp = html_req
        .send()
        .await
        .map_err(crate::error::ApiError::from)?;
    let html_status = html_resp.status();
    let html_body = html_resp
        .text()
        .await
        .map_err(crate::error::ApiError::from)?;

    if !html_status.is_success() {
        let preview: String = html_body.chars().take(200).collect();
        return Err(AppError::Other(format!(
            "シリーズページ取得エラー ({html_status}): {preview}"
        )));
    }

    let items = extract_series_videos_from_html(&html_body);
    let total_count = items.len() as i64;

    Ok(UserVideosResponse {
        total_count,
        items,
        debug_raw: None,
        series_title,
        series_description,
        series_thumbnail_url,
    })
}

/// Fallback: scrape server-response meta from series HTML page.
/// Only works if the series page embeds video data in the same pattern
/// as the watch page (unlikely for modern niconico series pages).
fn extract_series_videos_from_html(html: &str) -> Vec<UserVideoItem> {
    let re = regex::Regex::new(r#"<meta name="server-response" content="([^"]*)""#).ok();
    let raw = re
        .as_ref()
        .and_then(|r| r.captures(html).and_then(|c| c.get(1)).map(|m| m.as_str()));

    let json_str = match raw {
        Some(s) => crate::api::video::html_unescape(s),
        None => return Vec::new(),
    };

    let root: serde_json::Value = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let items_val = root
        .pointer("/data/response/series/items")
        .and_then(|v| v.as_array())
        .or_else(|| {
            root.pointer("/data/response/items")
                .and_then(|v| v.as_array())
        })
        .or_else(|| {
            root.pointer("/data/response/videos")
                .and_then(|v| v.as_array())
        })
        .or_else(|| root["data"]["response"]["series"]["items"].as_array())
        .or_else(|| root["data"]["response"]["items"].as_array())
        .cloned()
        .unwrap_or_default();

    let mut items = Vec::with_capacity(items_val.len());
    for raw_item in items_val {
        let v = if raw_item["essential"].is_object() {
            &raw_item["essential"]
        } else if raw_item["video"].is_object() {
            &raw_item["video"]
        } else {
            &raw_item
        };

        let id = json_value_as_id_string(&v["id"])
            .or_else(|| v["contentId"].as_str().map(String::from))
            .unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let uid = v["owner"]["id"].as_i64().or_else(|| v["userId"].as_i64());
        items.push(build_user_video_item(v, id, uid, None));
    }

    items
}

/// Use yt-dlp --dump-json --flat-playlist to list series videos.
/// Matches NicomusicBot's approach: reliable, handles API changes.
async fn fetch_series_videos_via_ytdlp(
    series_id: &str,
    cookie_header: Option<String>,
) -> Result<Vec<UserVideoItem>, AppError> {
    let yt = tools::ytdlp(None);
    if matches!(yt.source, tools::BinarySource::NotFound) {
        return Err(AppError::Other(
            "yt-dlp が見つかりません。インストールしてください。".into(),
        ));
    }

    let url = format!("https://www.nicovideo.jp/series/{series_id}");

    // Write cookies to temp file (Netscape format for yt-dlp compatibility)
    let tmp_dir = std::env::temp_dir().join("nndd-series");
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| AppError::Other(format!("一時ディレクトリ作成失敗: {e}")))?;
    let cookies_file = if let Some(ref cookie) = cookie_header {
        let path = tmp_dir.join("cookies.txt");
        let netscape = ytdlp_mod::build_netscape_cookies(cookie);
        if let Err(e) = tokio::fs::write(&path, netscape).await {
            tracing::warn!(error = %e, "failed to write yt-dlp cookies file");
            None
        } else {
            Some(path)
        }
    } else {
        None
    };

    let mut cmd = tools::tokio_command(&yt.command);
    cmd.arg("--dump-json")
        .arg("--flat-playlist")
        .arg("--no-warnings")
        .arg("--no-colors");

    if let Some(ref p) = cookies_file {
        cmd.arg("--cookies").arg(p);
    }

    cmd.arg(&url)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let output = cmd
        .output()
        .await
        .map_err(|e| AppError::Other(format!("yt-dlp 実行失敗: {e}")))?;

    // Clean up temp files
    if let Some(ref p) = cookies_file {
        let _ = tokio::fs::remove_file(p).await;
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let preview: String = stderr.chars().take(300).collect();
        return Err(AppError::Other(format!(
            "シリーズ動画の取得に失敗しました (yt-dlp exit {:?}): {preview}",
            output.status.code()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut items = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let json: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = json["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        let title = json["title"].as_str().unwrap_or("(無題)").to_string();
        let thumbnail_url = json["thumbnail"]
            .as_str()
            .or_else(|| json["thumbnail_url"].as_str())
            .map(String::from);
        let duration = json["duration"]
            .as_i64()
            .or_else(|| json["duration_string"].as_i64());

        items.push(UserVideoItem {
            content_id: id,
            title,
            thumbnail_url,
            length_seconds: duration,
            view_counter: None,
            comment_counter: None,
            mylist_counter: None,
            start_time: None,
            user_id: None,
            channel_id: None,
        });
    }

    Ok(items)
}

// =================== ユーザーマイリスト・シリーズ一覧 ===================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMylistSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub thumbnail_url: Option<String>,
    pub items_count: Option<i64>,
    pub is_public: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMylistsResponse {
    pub items: Vec<UserMylistSummary>,
    pub total_count: i64,
}

#[tauri::command]
pub async fn fetch_user_mylists(
    owner_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<UserMylistsResponse> {
    validate_owner_id(&owner_id)?;

    let client = build_nv_client()?;
    let url = format!("https://nvapi.nicovideo.jp/v1/users/{owner_id}/mylists?page=1&pageSize=50");
    let (items_val, total_count) = nv_fetch_paged_list(
        &client,
        &url,
        store.cookie_header(),
        "マイリスト一覧 API エラー",
        "mylists",
    )
    .await?;

    let mut items = Vec::with_capacity(items_val.len());
    for node in &items_val {
        let id = json_value_as_id_string(&node["id"]).unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let name = node["name"].as_str().unwrap_or("(無題)").to_string();
        let description = node["description"].as_str().map(String::from);
        let thumbnail_url = node["thumbnailUrl"]
            .as_str()
            .or_else(|| node["thumbnail"]["url"].as_str())
            .map(String::from);
        let items_count = node["itemsCount"]
            .as_i64()
            .or_else(|| node["totalItemCount"].as_i64());
        let is_public = node["isPublic"].as_bool().unwrap_or(true);

        items.push(UserMylistSummary {
            id,
            name,
            description,
            thumbnail_url,
            items_count,
            is_public,
        });
    }

    Ok(UserMylistsResponse { items, total_count })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSeriesSummary {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub thumbnail_url: Option<String>,
    pub items_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSeriesListResponse {
    pub items: Vec<UserSeriesSummary>,
    pub total_count: i64,
}

#[tauri::command]
pub async fn fetch_user_series_list(
    owner_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<UserSeriesListResponse> {
    validate_owner_id(&owner_id)?;

    let client = build_nv_client()?;
    let url = format!("https://nvapi.nicovideo.jp/v1/users/{owner_id}/series?page=1&pageSize=50");
    let (items_val, total_count) = nv_fetch_paged_list(
        &client,
        &url,
        store.cookie_header(),
        "シリーズ一覧 API エラー",
        "series",
    )
    .await?;

    let mut items = Vec::with_capacity(items_val.len());
    for node in &items_val {
        let id = json_value_as_id_string(&node["id"]).unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let title = node["title"].as_str().unwrap_or("(無題)").to_string();
        let description = node["description"]
            .as_str()
            .or_else(|| node["decoratedDescriptionHtml"].as_str())
            .map(String::from);
        let thumbnail_url = node["thumbnailUrl"]
            .as_str()
            .or_else(|| node["thumbnail"]["url"].as_str())
            .map(String::from);
        let items_count = node["itemsCount"]
            .as_i64()
            .or_else(|| node["videoCount"].as_i64());

        items.push(UserSeriesSummary {
            id,
            title,
            description,
            thumbnail_url,
            items_count,
        });
    }

    Ok(UserSeriesListResponse { items, total_count })
}

#[tauri::command]
pub async fn fetch_mylist_videos(
    mylist_id: String,
    page: u32,
    page_size: u32,
    store: State<'_, Arc<SessionStore>>,
) -> Result<UserVideosResponse> {
    let client = build_nv_client()?;
    let url = format!(
        "https://nvapi.nicovideo.jp/v2/mylists/{mylist_id}?pageSize={page_size}&page={page}"
    );
    let (json, body) = nv_get_json(
        &client,
        &url,
        store.cookie_header(),
        "マイリスト動画 API エラー",
    )
    .await?;

    let preview: String = body.chars().take(500).collect();
    tracing::info!(%url, body = %preview, "mylist videos API response");

    let mylist = &json["data"]["mylist"];
    let total_count = mylist["totalItemCount"]
        .as_i64()
        .or_else(|| json["data"]["totalCount"].as_i64())
        .unwrap_or(0);

    let items_val = mylist["items"].as_array().cloned().unwrap_or_default();

    let mut items = Vec::with_capacity(items_val.len());
    for raw_item in &items_val {
        let v = &raw_item["video"];
        let id = json_value_as_id_string(&v["id"])
            .or_else(|| v["contentId"].as_str().map(String::from))
            .unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let uid = v["owner"]["id"].as_i64().or_else(|| v["userId"].as_i64());
        items.push(build_user_video_item(v, id, uid, None));
    }

    Ok(UserVideosResponse {
        total_count,
        items,
        debug_raw: None,
        series_title: None,
        series_description: None,
        series_thumbnail_url: None,
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

    // プラグイン: ダウンロード開始通知 (listener 0 で no-op)。
    crate::plugins::emit_event(
        &app,
        "download:start",
        serde_json::json!({ "id": id, "videoId": video_id }),
    );

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
                // mark_status が失敗した場合は plugins に download:complete を
                // 送らない (Codex #5 P2: キュー行が done に永続化されていない
                // のに完了通知だけが届くと、プラグインの内部状態が DB と乖離)。
                let mark_ok = match queue::mark_status(&conn, id, "done") {
                    Ok(_) => true,
                    Err(e) => {
                        tracing::error!(error = %e, queue_id = id, "failed to mark done");
                        false
                    }
                };
                drop(conn);
                if mark_ok {
                    crate::plugins::emit_event(
                        &app_for_task,
                        "download:complete",
                        serde_json::json!({ "id": id, "videoId": video_id }),
                    );
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
                drop(conn);
                // プラグイン: ダウンロード失敗通知
                crate::plugins::emit_event(
                    &app_for_task,
                    "download:error",
                    serde_json::json!({ "id": id, "videoId": video_id, "message": msg }),
                );
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
            is_short: p.video.content_type.as_deref() == Some("short")
                || video_id.starts_with("ss"),
        }
    } else {
        // watch page が取れなかったケース（fallback）。yt-dlp info.json から組む。
        let is_short_fallback = match (info["width"].as_i64(), info["height"].as_i64()) {
            (Some(w), Some(h)) if w > 0 && h > 0 => h > w,
            _ => false,
        };
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
            is_short: is_short_fallback || video_id.starts_with("ss"),
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
fn build_local_media_url(video_id: &str, file: &str, server: &LocalServer) -> Result<String> {
    validate_video_id(video_id)?;
    Ok(format!(
        "http://127.0.0.1:{}/v/{}/{}",
        server.port, video_id, file
    ))
}

#[tauri::command]
pub fn local_video_url(video_id: String, server: State<'_, LocalServer>) -> Result<String> {
    build_local_media_url(&video_id, "video.mp4", &server)
}

#[tauri::command]
pub fn local_audio_url(video_id: String, server: State<'_, LocalServer>) -> Result<String> {
    build_local_media_url(&video_id, "audio.mp4", &server)
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

#[tauri::command]
pub async fn extract_video_frame(
    video_id: String,
    seek_sec: f64,
    app: tauri::AppHandle,
) -> Result<Option<String>> {
    use tauri::Manager;
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("app_data_dir: {e}")))?;
    let dir = app_data_dir.join("videos").join(&video_id);
    let video_path = dir.join("video.mp4");
    if !video_path.exists() {
        return Ok(None);
    }
    let png = crate::downloader::ffmpeg::extract_frame(Some(&app), &video_path, seek_sec).await;
    Ok(png.map(|b| BASE64.encode(b)))
}

#[tauri::command]
pub async fn extract_online_frame(
    hls_url: String,
    seek_sec: f64,
    app: tauri::AppHandle,
) -> Result<Option<String>> {
    let png =
        crate::downloader::ffmpeg::extract_frame_from_url(Some(&app), &hls_url, seek_sec).await;
    Ok(png.map(|b| BASE64.encode(b)))
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
    /// 縦長ショート動画かどうか。resolution から判定。
    pub is_short: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryTag {
    pub name: String,
    pub is_locked: bool,
}

/// ローカルに DL 済みの動画がある場合のみ Some を返す。
/// 無ければ呼び出し側は `prepare_playback` (HLS) にフォールバックする。
/// `snapshot_id` を指定するとそのスナップショットのコメントを返す。
/// 省略時は最新スナップショット（後方互換）。
#[tauri::command]
pub async fn prepare_local_playback(
    video_id: String,
    snapshot_id: Option<i64>,
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
                    view_count, comment_count, mylist_count, posted_at, thumbnail_url, video_path, is_short \
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
                    row.get::<_, i64>(13)?,
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

    let is_short = row.13 != 0;

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

    // 最新の snapshot のコメント（snapshot_id 指定時はそれを使用）
    let snap_id: Option<i64> = if let Some(sid) = snapshot_id {
        Some(sid)
    } else {
        conn.query_row(
            "SELECT id FROM comment_snapshots WHERE video_id = ?1 \
             ORDER BY taken_at DESC, id DESC LIMIT 1",
            rusqlite::params![video_id],
            |row| row.get(0),
        )
        .ok()
    };
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
        is_short,
    }))
}

// =================== コメントスナップショット運用 ===================

/// 指定動画の全スナップショットを一覧取得。
#[tauri::command]
pub async fn list_comment_snapshots(
    video_id: String,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<Vec<snapshots::CommentSnapshotRow>> {
    validate_video_id(&video_id)?;
    let conn = library.lock().await;
    snapshots::list_snapshots(&conn, &video_id).map_err(AppError::from)
}

/// スナップショットに含まれるコメントを LocalPlayerComment 形式で取得。
#[tauri::command]
pub async fn load_snapshot_comments(
    snapshot_id: i64,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<Vec<LocalPlayerComment>> {
    let conn = library.lock().await;
    let mut stmt = conn
        .prepare(
            "SELECT id, no, vpos_ms, content, mail, user_hash, is_owner, posted_at \
             FROM comments WHERE snapshot_id = ?1 ORDER BY vpos_ms ASC",
        )
        .map_err(|e| AppError::Other(format!("prepare: {e}")))?;

    let rows = stmt
        .query_map(rusqlite::params![snapshot_id], |row| {
            let mail: Option<String> = row.get(4)?;
            let mail_str = mail.unwrap_or_default();
            let commands: Vec<String> =
                mail_str.split_whitespace().map(|s| s.to_string()).collect();
            let is_owner = row.get::<_, i64>(6)? != 0;
            let fork = if is_owner { "owner" } else { "main" };
            Ok(LocalPlayerComment {
                id: row.get::<_, i64>(0)?.to_string(),
                no: row.get(1)?,
                vpos_ms: row.get(2)?,
                content: row.get(3)?,
                mail: mail_str,
                commands,
                user_id: row.get(5)?,
                posted_at: row.get::<_, Option<i64>>(7)?.map(|t| t.to_string()),
                fork: fork.to_string(),
                is_owner,
                nicoru_count: None,
                score: None,
            })
        })
        .map_err(|e| AppError::Other(format!("query comments: {e}")))?;
    let collected: Vec<LocalPlayerComment> = rows.filter_map(|r| r.ok()).collect();
    Ok(collected)
}

/// スナップショットを削除（CASCADE でコメントも消える）。
#[tauri::command]
pub async fn delete_comment_snapshot(
    snapshot_id: i64,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<bool> {
    let conn = library.lock().await;
    snapshots::delete_snapshot(&conn, snapshot_id).map_err(AppError::from)
}

/// スナップショットの note を更新。null でクリア。
#[tauri::command]
pub async fn update_snapshot_note(
    snapshot_id: i64,
    note: Option<String>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<bool> {
    let conn = library.lock().await;
    snapshots::update_snapshot_note(&conn, snapshot_id, note.as_deref()).map_err(AppError::from)
}

/// DL 済み動画のコメントを niconico API から再取得し、新規スナップショットを作成。
/// 成功時は新しい snapshot_id を返す。
#[tauri::command]
pub async fn refetch_video_comments(
    video_id: String,
    library: State<'_, Arc<LibraryHandle>>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<i64> {
    use crate::api::video::NiconicoWatchClient;
    validate_video_id(&video_id)?;

    // 動画がライブラリに存在するか確認
    {
        let conn = library.lock().await;
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM videos WHERE id = ?1 AND video_path IS NOT NULL",
                rusqlite::params![video_id],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if !exists {
            return Err(AppError::Other("動画がダウンロードされていません".into()));
        }
    }

    let session = Arc::clone(&store);
    let watch = NiconicoWatchClient::new(Arc::clone(&session)).map_err(AppError::from)?;
    let page = watch.fetch_watch_page(&video_id).await.ok();
    let comments_dto = if let Some(p) = page.as_ref().and_then(|p| p.nv_comment.as_ref()) {
        let cclient = ThreadsClient::new(Arc::clone(&session)).map_err(AppError::from)?;
        cclient.fetch_comments(p).await.unwrap_or_default()
    } else {
        return Err(AppError::Other(
            "watch ページからコメント情報を取得できませんでした".into(),
        ));
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

    let mut conn = library.lock().await;
    let snapshot_id = snapshots::take_snapshot(&mut conn, &video_id, &comment_records, None)
        .map_err(AppError::from)?;

    tracing::info!(
        video_id = %video_id,
        snapshot_id = snapshot_id,
        comments = comment_records.len(),
        "refetched video comments"
    );
    Ok(snapshot_id)
}

// =================== コメント焼き込みエクスポート ===================
//
// 旧実装は Rust 側で独自に ASS 字幕を生成していたが、座標・サイズが本物の
// niconico と大きくずれていた。現在はフロント (WebView) が
// `@xpadev-net/niconicomments` で 1 フレームずつ Canvas に描画し、その PNG を
// stdin 経由で ffmpeg へ流し込んで元動画へオーバーレイする
// (niconicomments-convert と同じ構成)。Rust 側は ffmpeg セッションの管理と
// 映像合成のみを担い、コメントの座標計算には一切関与しない。

use crate::downloader::burnin::{self, BurnInSessions};

/// `burnin_start` のオプション。すべて省略可。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BurnInStartOptions {
    /// 出力幅 (px)。省略時は元動画の幅。高さは 16:9 で自動算出。
    pub width: Option<u32>,
    /// フレームレート (既定 30)。
    pub fps: Option<u32>,
    /// 不透明度 0..1 (既定 1.0)。ffmpeg のオーバーレイ時に適用。
    pub opacity: Option<f64>,
    /// 出力先フォルダ (省略時は app_data の exports/)。
    pub output_dir: Option<String>,
}

/// `burnin_start` の戻り値。フロントはこれを見てフレーム生成する。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BurnInStart {
    pub session_id: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration_sec: f64,
    pub total_frames: u64,
}

/// `burnin_finish` の戻り値。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BurnInFinish {
    pub output_path: String,
    pub width: u32,
    pub height: u32,
}

/// `"1280x720"` → `(1280, 720)`。
fn parse_resolution(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// 偶数へ丸める (libx264 / yuv420p は偶数解像度が必須)。
fn to_even(n: u32) -> u32 {
    n & !1
}

static BURNIN_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 焼き込みセッションを開始する。ffmpeg を起動し、フロントがフレームを流し込める
/// 状態にして出力解像度・フレーム数などの metadata を返す。
///
/// コメントデータ自体はフロントが既に保持しているため、ここでは扱わない
/// (動画ファイル・音声・解像度・長さの解決と ffmpeg 起動のみ)。
#[tauri::command]
pub async fn burnin_start(
    video_id: String,
    options: Option<BurnInStartOptions>,
    library: State<'_, Arc<LibraryHandle>>,
    sessions: State<'_, BurnInSessions>,
    app: tauri::AppHandle,
) -> Result<BurnInStart> {
    use tauri::Manager;
    validate_video_id(&video_id)?;
    let options = options.unwrap_or_default();

    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("app_data_dir: {e}")))?;

    let (video_rel_path, resolution, duration_sec) = {
        let conn = library.lock().await;
        let row = conn
            .query_row(
                "SELECT video_path, resolution, duration_sec FROM videos WHERE id = ?1",
                rusqlite::params![video_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| AppError::Other(format!("query video: {e}")))?;
        let Some((Some(video_rel_path), resolution, duration_sec)) = row else {
            return Err(AppError::Other("動画がダウンロードされていません".into()));
        };
        (video_rel_path, resolution, duration_sec)
    };

    let abs_video = app_data_dir.join(&video_rel_path);
    if !abs_video.exists() {
        return Err(AppError::Other(format!(
            "動画ファイルが見つかりません: {}",
            abs_video.display()
        )));
    }
    // 別ファイル音声 (audio.mp4) があれば多重化する。
    let audio_path = app_data_dir
        .join("videos")
        .join(&video_id)
        .join("audio.mp4");
    let audio = if audio_path.exists() {
        Some(audio_path)
    } else {
        None
    };

    // 解像度・長さの決定 (DB 優先、欠けていれば ffmpeg でプローブ)。
    let mut src_w = resolution
        .as_deref()
        .and_then(parse_resolution)
        .map(|d| d.0);
    let mut src_h = resolution
        .as_deref()
        .and_then(parse_resolution)
        .map(|d| d.1);
    let mut duration = if duration_sec > 0 {
        duration_sec as f64
    } else {
        0.0
    };
    if src_w.is_none() || src_h.is_none() || duration <= 0.0 {
        if let Some(info) = crate::downloader::ffmpeg::probe_video(Some(&app), &abs_video).await {
            src_w.get_or_insert(info.width);
            src_h.get_or_insert(info.height);
            if duration <= 0.0 {
                duration = info.duration_sec;
            }
        }
    }
    let (Some(src_w), Some(_src_h)) = (src_w, src_h) else {
        return Err(AppError::Other("動画の解像度を判定できませんでした".into()));
    };
    if duration <= 0.0 {
        return Err(AppError::Other("動画の長さを判定できませんでした".into()));
    }

    // 出力解像度を 16:9 に正規化。幅はオプション or 元動画幅、高さは 9/16。
    let out_w = to_even(options.width.unwrap_or(src_w).clamp(64, 3840));
    let out_h = to_even((f64::from(out_w) * 9.0 / 16.0).round() as u32).max(2);
    let fps = options.fps.unwrap_or(30).clamp(1, 120);
    let opacity = options.opacity.unwrap_or(1.0).clamp(0.0, 1.0);
    // フロントのフレームループ (computeTotalFrames) と一致させる: ceil(duration)*fps。
    let total_frames = (duration.ceil() as u64) * u64::from(fps);

    // 出力先。指定があればそのフォルダ、無ければ app_data の exports/ 配下
    // (cleanup_storage が触らない場所)。
    let exports_dir = match options
        .output_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(dir) => std::path::PathBuf::from(dir),
        None => app_data_dir.join("exports"),
    };
    tokio::fs::create_dir_all(&exports_dir)
        .await
        .map_err(|e| AppError::Other(format!("create exports dir: {e}")))?;
    let stamp = crate::library::now_unix_secs();
    let output_path = exports_dir.join(format!("{video_id}_{stamp}.mp4"));

    let session = burnin::spawn_session(
        Some(&app),
        &abs_video,
        audio.as_deref(),
        &output_path,
        out_w,
        out_h,
        fps,
        opacity,
        total_frames,
    )
    .await
    .map_err(AppError::from)?;

    let seq = BURNIN_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let session_id = format!("burnin-{stamp}-{seq}");
    sessions.insert(session_id.clone(), session);

    tracing::info!(
        video_id = %video_id,
        session_id = %session_id,
        out_w,
        out_h,
        fps,
        total_frames,
        "comment burn-in session started"
    );

    Ok(BurnInStart {
        session_id,
        width: out_w,
        height: out_h,
        fps,
        duration_sec: duration,
        total_frames,
    })
}

/// コメントフレーム (PNG) を 1 枚 ffmpeg へ流し込む。
///
/// body は raw バイナリで `burnin::parse_feed_frame` のフレーミングに従う:
/// `[u8 flag][u32 LE session_len][session][payload]`。flag で frame / empty /
/// set-empty を切り替える。透明フレームは set-empty で 1 度だけ転送し使い回す。
#[tauri::command]
pub async fn burnin_feed(
    request: tauri::ipc::Request<'_>,
    sessions: State<'_, BurnInSessions>,
) -> Result<()> {
    let body: &[u8] = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes.as_slice(),
        _ => return Err(AppError::Other("burnin_feed: raw body required".into())),
    };
    let (flag, sid, payload) = burnin::parse_feed_frame(body)
        .ok_or_else(|| AppError::Other("burnin_feed: malformed frame".into()))?;
    let session = sessions
        .get(&sid)
        .ok_or_else(|| AppError::Other("burnin_feed: unknown session".into()))?;
    let mut s = session.lock().await;
    match flag {
        burnin::FLAG_SET_EMPTY => s.set_empty(payload.to_vec()),
        burnin::FLAG_EMPTY => s.write_empty().await.map_err(AppError::from)?,
        burnin::FLAG_FRAME => s.write_frame(payload).await.map_err(AppError::from)?,
        _ => return Err(AppError::Other("burnin_feed: bad flag".into())),
    }
    Ok(())
}

/// セッションを完了する。stdin を閉じて ffmpeg の終了を待ち、出力パスを返す。
#[tauri::command]
pub async fn burnin_finish(
    session_id: String,
    sessions: State<'_, BurnInSessions>,
) -> Result<BurnInFinish> {
    let Some(session) = sessions.remove(&session_id) else {
        return Err(AppError::Other("burnin_finish: unknown session".into()));
    };
    let mut s = session.lock().await;
    s.finish().await.map_err(AppError::from)?;
    let out = s.output_path.to_string_lossy().into_owned();
    tracing::info!(session_id = %session_id, output = %out, "comment burn-in finished");
    Ok(BurnInFinish {
        output_path: out,
        width: s.width,
        height: s.height,
    })
}

/// セッションを中断する。ffmpeg を kill して部分出力を消す。
#[tauri::command]
pub async fn burnin_cancel(session_id: String, sessions: State<'_, BurnInSessions>) -> Result<()> {
    if let Some(session) = sessions.remove(&session_id) {
        let mut s = session.lock().await;
        s.kill().await;
        let _ = tokio::fs::remove_file(&s.output_path).await;
    }
    Ok(())
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentSearchHitDto {
    pub video_id: String,
    pub video_title: String,
    pub comment_no: i64,
    pub vpos_ms: i64,
    pub content: String,
    pub user_hash: Option<String>,
    pub posted_at: Option<i64>,
}

impl From<query::CommentSearchHit> for CommentSearchHitDto {
    fn from(h: query::CommentSearchHit) -> Self {
        Self {
            video_id: h.video_id,
            video_title: h.video_title,
            comment_no: h.comment_no,
            vpos_ms: h.vpos_ms,
            content: h.content,
            user_hash: h.user_hash,
            posted_at: h.posted_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentSearchResultDto {
    pub items: Vec<CommentSearchHitDto>,
    pub total_count: i64,
    pub offset: u32,
    pub limit: u32,
}

impl From<query::CommentSearchResult> for CommentSearchResultDto {
    fn from(r: query::CommentSearchResult) -> Self {
        Self {
            items: r.items.into_iter().map(Into::into).collect(),
            total_count: r.total_count,
            offset: r.offset,
            limit: r.limit,
        }
    }
}

#[tauri::command]
pub async fn search_library_comments(
    query: String,
    offset: Option<u32>,
    limit: Option<u32>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<CommentSearchResultDto> {
    if query.chars().count() < 3 {
        return Err(AppError::Other(
            "コメント検索は3文字以上のクエリが必要です".into(),
        ));
    }
    let conn = library.lock().await;
    let result = query::search_comments(&conn, &query, offset.unwrap_or(0), limit.unwrap_or(50))
        .map_err(AppError::from)?;
    Ok(result.into())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploaderInfoDto {
    pub uploader_id: String,
    pub uploader_name: Option<String>,
    pub video_count: i64,
    pub total_duration_sec: i64,
}

impl From<query::UploaderInfo> for UploaderInfoDto {
    fn from(u: query::UploaderInfo) -> Self {
        Self {
            uploader_id: u.uploader_id,
            uploader_name: u.uploader_name,
            video_count: u.video_count,
            total_duration_sec: u.total_duration_sec,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistDto {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub source: String,
    pub source_official_id: Option<String>,
    pub imported_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub item_count: i64,
}

impl From<crate::library::playlists::Playlist> for PlaylistDto {
    fn from(p: crate::library::playlists::Playlist) -> Self {
        Self {
            id: p.id,
            name: p.name,
            parent_id: p.parent_id,
            source: p.source,
            source_official_id: p.source_official_id,
            imported_at: p.imported_at,
            created_at: p.created_at,
            updated_at: p.updated_at,
            item_count: p.item_count,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistItemDto {
    pub playlist_id: i64,
    pub video_id: String,
    pub position: i64,
    pub added_at: i64,
    pub note: Option<String>,
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration_sec: Option<i64>,
}

impl From<crate::library::playlists::PlaylistItem> for PlaylistItemDto {
    fn from(i: crate::library::playlists::PlaylistItem) -> Self {
        Self {
            playlist_id: i.playlist_id,
            video_id: i.video_id,
            position: i.position,
            added_at: i.added_at,
            note: i.note,
            title: i.title,
            thumbnail_url: i.thumbnail_url,
            duration_sec: i.duration_sec,
        }
    }
}

#[tauri::command]
pub async fn list_library_uploaders(
    limit: Option<u32>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<Vec<UploaderInfoDto>> {
    let conn = library.lock().await;
    let uploaders = query::list_uploaders(&conn, limit.unwrap_or(50)).map_err(AppError::from)?;
    Ok(uploaders.into_iter().map(Into::into).collect())
}

// =================== プレイリスト CRUD ===================

#[tauri::command]
pub async fn list_playlists(library: State<'_, Arc<LibraryHandle>>) -> Result<Vec<PlaylistDto>> {
    let conn = library.lock().await;
    let playlists = crate::library::playlists::list_playlists(&conn).map_err(AppError::from)?;
    Ok(playlists.into_iter().map(Into::into).collect())
}

#[tauri::command]
pub async fn create_playlist(
    name: String,
    parent_id: Option<i64>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<PlaylistDto> {
    let conn = library.lock().await;
    let playlist = crate::library::playlists::create_playlist(&conn, &name, parent_id)
        .map_err(AppError::from)?;
    Ok(playlist.into())
}

#[tauri::command]
pub async fn update_playlist(
    id: i64,
    name: String,
    parent_id: Option<i64>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<PlaylistDto> {
    let conn = library.lock().await;
    let playlist = crate::library::playlists::update_playlist(&conn, id, &name, parent_id)
        .map_err(AppError::from)?;
    Ok(playlist.into())
}

#[tauri::command]
pub async fn delete_playlist(id: i64, library: State<'_, Arc<LibraryHandle>>) -> Result<bool> {
    let conn = library.lock().await;
    crate::library::playlists::delete_playlist(&conn, id).map_err(AppError::from)
}

#[tauri::command]
pub async fn list_playlist_items(
    playlist_id: i64,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<Vec<PlaylistItemDto>> {
    let conn = library.lock().await;
    let items = crate::library::playlists::list_playlist_items(&conn, playlist_id)
        .map_err(AppError::from)?;
    Ok(items.into_iter().map(Into::into).collect())
}

#[tauri::command]
pub async fn add_playlist_item(
    playlist_id: i64,
    video_id: String,
    position: Option<i64>,
    note: Option<String>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<PlaylistItemDto> {
    let conn = library.lock().await;
    let item = crate::library::playlists::add_playlist_item(
        &conn,
        playlist_id,
        &video_id,
        position,
        note.as_deref(),
    )
    .map_err(AppError::from)?;
    Ok(item.into())
}

#[tauri::command]
pub async fn remove_playlist_item(
    playlist_id: i64,
    video_id: String,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<bool> {
    let conn = library.lock().await;
    crate::library::playlists::remove_playlist_item(&conn, playlist_id, &video_id)
        .map_err(AppError::from)
}

// =================== 再生履歴 ===================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayHistoryItemDto {
    pub id: i64,
    pub video_id: String,
    pub played_at: i64,
    pub duration_played_sec: f64,
    pub position_at_close_sec: Option<f64>,
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration_sec: Option<i64>,
    pub is_short: bool,
}

impl From<crate::library::history::PlayHistoryItem> for PlayHistoryItemDto {
    fn from(i: crate::library::history::PlayHistoryItem) -> Self {
        Self {
            id: i.id,
            video_id: i.video_id,
            played_at: i.played_at,
            duration_played_sec: i.duration_played_sec,
            position_at_close_sec: i.position_at_close_sec,
            title: i.title,
            thumbnail_url: i.thumbnail_url,
            duration_sec: i.duration_sec,
            is_short: i.is_short,
        }
    }
}

#[tauri::command]
pub async fn record_playback(
    video_id: String,
    duration_played_sec: f64,
    position_at_close_sec: Option<f64>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<PlayHistoryItemDto> {
    let conn = library.lock().await;
    let item = crate::library::history::record_playback(
        &conn,
        &video_id,
        duration_played_sec,
        position_at_close_sec,
    )
    .map_err(AppError::from)?;
    Ok(item.into())
}

#[tauri::command]
pub async fn list_play_history(
    offset: Option<u32>,
    limit: Option<u32>,
    is_short: Option<bool>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<Vec<PlayHistoryItemDto>> {
    let conn = library.lock().await;
    let items = crate::library::history::list_play_history(
        &conn,
        offset.unwrap_or(0),
        limit.unwrap_or(50),
        is_short,
    )
    .map_err(AppError::from)?;
    Ok(items.into_iter().map(Into::into).collect())
}

#[tauri::command]
pub async fn delete_play_history_item(
    id: i64,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<bool> {
    let conn = library.lock().await;
    crate::library::history::delete_play_history_item(&conn, id).map_err(AppError::from)
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
    // Windows でコンソールウィンドウがチラつかないようヘルパ経由で起動する。
    match crate::downloader::tools::tokio_command(cmd)
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

// =================== ランキング ===================

/// niconico ランキングページの HTML を取得する。
/// ブラウザ (WebView) から直接 fetch すると CORS で弾かれるため、
/// Rust 側の reqwest で取得して HTML 文字列を返す。
/// フロントエンド側で `@kongyo2/nicoran-api` の `extractAndParse` に渡す。
#[tauri::command]
pub async fn fetch_ranking_html(url: String) -> Result<String> {
    let parsed = url::Url::parse(&url).map_err(|e| AppError::Other(format!("invalid url: {e}")))?;
    let host = parsed.host_str().unwrap_or("");
    if host != "www.nicovideo.jp" {
        return Err(AppError::Other(format!(
            "ランキング取得は nicovideo.jp のみ許可: {host}"
        )));
    }
    let path = parsed.path();
    if !path.starts_with("/ranking/") {
        return Err(AppError::Other(format!(
            "ランキング以外のパスは許可されません: {path}"
        )));
    }

    let client = reqwest::Client::builder()
        .user_agent(NV_USER_AGENT)
        .build()
        .map_err(crate::error::ApiError::from)?;

    let resp = client
        .get(&url)
        .header(header::ACCEPT, "text/html,application/xhtml+xml")
        .header(header::ACCEPT_LANGUAGE, "ja,en-US;q=0.9,en;q=0.8")
        .send()
        .await
        .map_err(crate::error::ApiError::from)?;

    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "ランキングページ取得エラー ({status}): {url}"
        )));
    }

    let html = resp.text().await.map_err(crate::error::ApiError::from)?;

    tracing::debug!(%url, size = html.len(), "ranking HTML fetched");
    Ok(html)
}

// =================== ショート動画ランキング ===================
//
// Snapshot Search API はショート動画 (`ss` プレフィックス) を索引しておらず、
// 公式の `/ranking/genre/*` ページ HTML にもショートは含まれない
// (https://blog.nicovideo.jp/niconews/270458.html「ランキングにはショートは
// 掲載されません」)。
//
// 一方で niconico の新 Web クライアントは `nvapi.nicovideo.jp/v2/search/video`
// を `selectContentType=short` 付きで叩いてショート一覧を取得している。
// このエンドポイントは `keyword` / `tag` / `lockTag` のいずれかを必須とするが、
// 一般的な日本語の助詞 (例: 「の」) を渡せばタイトル/説明文/タグの広いマッチで
// ほぼ全ショートを拾える (確認時点で 24,000+ 件)。
//
// sortKey:
//   - hot           (sortOrder=none) … トレンド (Web 既定)
//   - viewCount     (sortOrder=desc) … 再生数
//   - registeredAt  (sortOrder=desc) … 新着
//   - commentCount  (sortOrder=desc) … コメ数
//   - mylistCount   (sortOrder=desc) … マイリスト数
//   - likeCount     (sortOrder=desc) … いいね数

const SHORT_SEARCH_URL: &str = "https://nvapi.nicovideo.jp/v2/search/video";

/// 一覧 API の 1 文字ワイルドカード。「の」は約 24,056 件の short を返す。
/// 並び順が viewCount desc / hot のとき、上位の結果は他のひらがなを使った
/// 場合と一致するので問題なし。
const SHORT_SEARCH_WILDCARD: &str = "の";

#[derive(Debug, Clone, Serialize)]
pub struct ShortRankingCount {
    pub view: i64,
    pub comment: i64,
    pub mylist: i64,
    pub like: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortRankingThumbnail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middle_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub large_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listing_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_hd_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortRankingOwner {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortRankingItem {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registered_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_description: Option<String>,
    pub count: ShortRankingCount,
    pub thumbnail: ShortRankingThumbnail,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<ShortRankingOwner>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortRankingResponse {
    pub items: Vec<ShortRankingItem>,
    pub total_count: i64,
    pub has_next: bool,
}

/// `sort_key` / `genre` が API 仕様で許可された値かを検証する。
/// API の 400 を増やさないよう、不正値はここで弾く。
fn validate_short_sort(sort_key: &str) -> Result<&'static str> {
    // 戻り値は対応する sortOrder。hot だけ none、それ以外は desc。
    match sort_key {
        "hot" => Ok("none"),
        "viewCount" | "registeredAt" | "commentCount" | "mylistCount" | "likeCount" => Ok("desc"),
        other => Err(AppError::Other(format!(
            "invalid sort_key for shorts: {other}"
        ))),
    }
}

/// `/v2/search/video?selectContentType=short` が受け付けるジャンル名。
/// 公式ランキングのジャンル名と一部しか重ならない (例: music_sound, commentary_lecture)
/// ことに注意。
const SHORT_GENRES: &[&str] = &[
    "anime",
    "game",
    "music_sound",
    "entertainment",
    "dance",
    "commentary_lecture",
    "cooking",
    "nature",
    "vehicle",
    "radio",
    "sports",
    "animal",
    "other",
];

fn validate_short_genre(genre: &str) -> Result<()> {
    if SHORT_GENRES.contains(&genre) {
        Ok(())
    } else {
        Err(AppError::Other(format!(
            "invalid genre for shorts: {genre}"
        )))
    }
}

/// niconico ショート動画のランキング相当を取得する。
/// `genre` を `None` にすると全ジャンルから集計する。
#[tauri::command]
pub async fn search_short_ranking(
    sort_key: Option<String>,
    genre: Option<String>,
    page: Option<u32>,
    page_size: Option<u32>,
) -> Result<ShortRankingResponse> {
    let sort_key = sort_key.as_deref().unwrap_or("hot");
    let sort_order = validate_short_sort(sort_key)?;
    if let Some(g) = genre.as_deref() {
        validate_short_genre(g)?;
    }
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(50).clamp(1, 100);

    let mut url = url::Url::parse(SHORT_SEARCH_URL).map_err(|e| AppError::Other(e.to_string()))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("selectContentType", "short");
        q.append_pair("sortKey", sort_key);
        q.append_pair("sortOrder", sort_order);
        q.append_pair("keyword", SHORT_SEARCH_WILDCARD);
        if let Some(g) = genre.as_deref() {
            q.append_pair("genres", g);
        }
        q.append_pair("pageSize", &page_size.to_string());
        q.append_pair("page", &page.to_string());
    }

    let client = build_nv_client()?;
    let (json, _body) =
        nv_get_json(&client, url.as_str(), None, "short ranking fetch failed").await?;

    let data = &json["data"];
    let total_count = data["totalCount"].as_i64().unwrap_or(0);
    let has_next = data["hasNext"].as_bool().unwrap_or(false);
    let raw_items = data["items"].as_array().cloned().unwrap_or_default();

    let items: Vec<ShortRankingItem> = raw_items
        .into_iter()
        .filter_map(parse_short_ranking_item)
        .collect();

    tracing::debug!(
        ?sort_key,
        ?genre,
        page,
        page_size,
        total_count,
        items = items.len(),
        "short ranking fetched"
    );

    Ok(ShortRankingResponse {
        items,
        total_count,
        has_next,
    })
}

fn parse_short_ranking_item(v: serde_json::Value) -> Option<ShortRankingItem> {
    let id = v.get("id")?.as_str()?.to_string();
    if id.is_empty() {
        return None;
    }
    let title = v
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("(無題)")
        .to_string();
    let content_type = v
        .get("contentType")
        .and_then(|x| x.as_str())
        .map(String::from);
    let registered_at = v
        .get("registeredAt")
        .and_then(|x| x.as_str())
        .map(String::from);
    let duration = v.get("duration").and_then(|x| x.as_i64());
    let short_description = v
        .get("shortDescription")
        .and_then(|x| x.as_str())
        .map(String::from);

    let count_v = v.get("count");
    let count = ShortRankingCount {
        view: count_v
            .and_then(|c| c.get("view"))
            .and_then(|x| x.as_i64())
            .unwrap_or(0),
        comment: count_v
            .and_then(|c| c.get("comment"))
            .and_then(|x| x.as_i64())
            .unwrap_or(0),
        mylist: count_v
            .and_then(|c| c.get("mylist"))
            .and_then(|x| x.as_i64())
            .unwrap_or(0),
        like: count_v
            .and_then(|c| c.get("like"))
            .and_then(|x| x.as_i64())
            .unwrap_or(0),
    };

    let thumb_v = v.get("thumbnail");
    let thumbnail = ShortRankingThumbnail {
        url: thumb_v
            .and_then(|t| t.get("url"))
            .and_then(|x| x.as_str())
            .map(String::from),
        middle_url: thumb_v
            .and_then(|t| t.get("middleUrl"))
            .and_then(|x| x.as_str())
            .map(String::from),
        large_url: thumb_v
            .and_then(|t| t.get("largeUrl"))
            .and_then(|x| x.as_str())
            .map(String::from),
        listing_url: thumb_v
            .and_then(|t| t.get("listingUrl"))
            .and_then(|x| x.as_str())
            .map(String::from),
        n_hd_url: thumb_v
            .and_then(|t| t.get("nHdUrl"))
            .and_then(|x| x.as_str())
            .map(String::from),
        short_url: thumb_v
            .and_then(|t| t.get("shortUrl"))
            .and_then(|x| x.as_str())
            .map(String::from),
    };

    let owner = v.get("owner").filter(|o| !o.is_null()).map(|o| {
        // owner.id は文字列か数値のどちらでも返るので両対応にする。
        let owner_id = o.get("id").and_then(|x| {
            x.as_str()
                .map(String::from)
                .or_else(|| x.as_i64().map(|n| n.to_string()))
        });
        ShortRankingOwner {
            owner_type: o
                .get("ownerType")
                .and_then(|x| x.as_str())
                .map(String::from),
            id: owner_id,
            name: o.get("name").and_then(|x| x.as_str()).map(String::from),
            icon_url: o.get("iconUrl").and_then(|x| x.as_str()).map(String::from),
        }
    });

    Some(ShortRankingItem {
        id,
        title,
        content_type,
        registered_at,
        duration,
        short_description,
        count,
        thumbnail,
        owner,
    })
}

/// niconico 動画ページ (watch/{id}) の HTML を取得する。
/// `@kongyo2/nicotag-api` の `extractAndParse` でタグ情報を抜くために、
/// ランキング NG のタグフィルタから呼ばれる。
/// (フロントエンドからは CORS と認証 Cookie の都合で直 fetch できない。)
///
/// 一部の動画 (年齢制限・会員限定・ログイン必須など) は認証 Cookie が
/// 無いと watch ページが返らずタグが取れない。ログイン中であれば
/// 保存済みセッション Cookie を付けて取得を試みる。
#[tauri::command]
pub async fn fetch_video_html(
    video_id: String,
    store: State<'_, Arc<SessionStore>>,
) -> Result<String> {
    validate_video_id(&video_id)?;

    let url = format!("https://www.nicovideo.jp/watch/{video_id}");
    let client = build_nv_client()?;

    let mut req = client
        .get(&url)
        .header(header::ACCEPT, "text/html,application/xhtml+xml")
        .header(header::ACCEPT_LANGUAGE, "ja,en-US;q=0.9,en;q=0.8");
    if let Some(cookie) = store.cookie_header() {
        req = req.header(header::COOKIE, cookie);
    }

    let resp = req.send().await.map_err(crate::error::ApiError::from)?;

    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::Other(format!(
            "動画ページ取得エラー ({status}): {url}"
        )));
    }

    let html = resp.text().await.map_err(crate::error::ApiError::from)?;
    tracing::debug!(%video_id, size = html.len(), "watch HTML fetched");
    Ok(html)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
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

    #[test]
    fn parse_thumbnail_url_extracts_plain_and_hashed() {
        let plain = r#"<nicovideo_thumb_response status="ok"><thumb>
            <thumbnail_url>https://nicovideo.cdn.nimg.jp/thumbnails/6857306/6857306</thumbnail_url>
            </thumb></nicovideo_thumb_response>"#;
        assert_eq!(
            parse_thumbnail_url_from_xml(plain).as_deref(),
            Some("https://nicovideo.cdn.nimg.jp/thumbnails/6857306/6857306")
        );
        // 投稿者がサムネ差し替え済みのケース: ハッシュ付き URL を返す。
        let hashed = "<thumbnail_url>https://nicovideo.cdn.nimg.jp/thumbnails/46425662/46425662.53524752</thumbnail_url>";
        assert_eq!(
            parse_thumbnail_url_from_xml(hashed).as_deref(),
            Some("https://nicovideo.cdn.nimg.jp/thumbnails/46425662/46425662.53524752")
        );
    }

    #[test]
    fn parse_thumbnail_url_handles_deleted_and_empty() {
        // 削除済み動画は status="fail" で thumbnail_url 要素が無い。
        let deleted = r#"<nicovideo_thumb_response status="fail"><error>
            <code>DELETED</code></error></nicovideo_thumb_response>"#;
        assert_eq!(parse_thumbnail_url_from_xml(deleted), None);
        assert_eq!(
            parse_thumbnail_url_from_xml("<thumbnail_url></thumbnail_url>"),
            None
        );
        assert_eq!(parse_thumbnail_url_from_xml(""), None);
    }

    #[test]
    fn parse_related_videos_prefers_stable_url_over_signed_listing() {
        // 古い動画の recommend では listingUrl が署名付き(失効しうる)URL に
        // なるため、安定した `url` を優先する事を保証する回帰テスト。
        let json = serde_json::json!({
            "data": { "items": [{
                "contentType": "video",
                "content": {
                    "id": "sm6913290",
                    "title": "old video",
                    "thumbnail": {
                        "url": "https://nicovideo.cdn.nimg.jp/thumbnails/6913290/6913290",
                        "listingUrl": "https://img.cdn.nimg.jp/s/nicovideo/thumbnails/6913290/6913290.original/r320x180l?key=deadbeef",
                        "middleUrl": serde_json::Value::Null,
                        "largeUrl": serde_json::Value::Null
                    }
                }
            }]}
        });
        let videos = parse_related_videos(json).unwrap();
        assert_eq!(videos.len(), 1);
        assert_eq!(
            videos[0].thumbnail_url.as_deref(),
            Some("https://nicovideo.cdn.nimg.jp/thumbnails/6913290/6913290")
        );
    }

    #[test]
    fn validate_short_sort_maps_known_keys() {
        assert_eq!(validate_short_sort("hot").unwrap(), "none");
        assert_eq!(validate_short_sort("viewCount").unwrap(), "desc");
        assert_eq!(validate_short_sort("registeredAt").unwrap(), "desc");
        assert_eq!(validate_short_sort("commentCount").unwrap(), "desc");
        assert_eq!(validate_short_sort("mylistCount").unwrap(), "desc");
        assert_eq!(validate_short_sort("likeCount").unwrap(), "desc");
    }

    #[test]
    fn validate_short_sort_rejects_unknown() {
        assert!(validate_short_sort("popularity").is_err());
        assert!(validate_short_sort("").is_err());
        assert!(validate_short_sort("HOT").is_err());
    }

    #[test]
    fn validate_short_genre_accepts_supported() {
        for g in SHORT_GENRES {
            assert!(validate_short_genre(g).is_ok(), "genre `{g}` should be ok");
        }
    }

    #[test]
    fn validate_short_genre_rejects_unsupported() {
        // 公式ランキングにはあるがショート検索 API では弾かれるジャンル群。
        for bad in &["vocaloid", "voicesynth", "sing", "play", "travel", "music"] {
            assert!(
                validate_short_genre(bad).is_err(),
                "genre `{bad}` should be rejected"
            );
        }
        assert!(validate_short_genre("").is_err());
        assert!(validate_short_genre("all").is_err());
    }

    #[test]
    fn parse_short_ranking_item_handles_minimal_object() {
        let v = serde_json::json!({
            "id": "ss12345",
            "title": "テスト",
        });
        let item = parse_short_ranking_item(v).expect("should parse");
        assert_eq!(item.id, "ss12345");
        assert_eq!(item.title, "テスト");
        assert_eq!(item.count.view, 0);
        assert!(item.owner.is_none());
    }

    #[test]
    fn parse_short_ranking_item_handles_full_object() {
        let v = serde_json::json!({
            "id": "ss46342592",
            "contentType": "short",
            "title": "サンプル",
            "registeredAt": "2026-05-22T21:00:00+09:00",
            "duration": 17,
            "shortDescription": "desc",
            "count": {"view": 100, "comment": 5, "mylist": 2, "like": 7},
            "thumbnail": {
                "url": "https://example.test/thumb.jpg",
                "listingUrl": "https://example.test/thumb_l.jpg"
            },
            "owner": {
                "ownerType": "user",
                "id": "12345",
                "name": "投稿者",
                "iconUrl": "https://example.test/icon.jpg"
            }
        });
        let item = parse_short_ranking_item(v).expect("should parse");
        assert_eq!(item.id, "ss46342592");
        assert_eq!(item.content_type.as_deref(), Some("short"));
        assert_eq!(item.duration, Some(17));
        assert_eq!(item.count.view, 100);
        assert_eq!(item.count.like, 7);
        assert_eq!(
            item.thumbnail.url.as_deref(),
            Some("https://example.test/thumb.jpg")
        );
        let owner = item.owner.expect("owner present");
        assert_eq!(owner.id.as_deref(), Some("12345"));
        assert_eq!(owner.name.as_deref(), Some("投稿者"));
    }

    #[test]
    fn parse_short_ranking_item_handles_numeric_owner_id() {
        // owner.id が数値で来るバリエーション (nvapi の他エンドポイントでは
        // 文字列で来るが、念のため両対応にしているのでテストで保証する)
        let v = serde_json::json!({
            "id": "ss1",
            "title": "x",
            "owner": {"id": 42, "name": "ｎ"}
        });
        let item = parse_short_ranking_item(v).expect("should parse");
        assert_eq!(
            item.owner.as_ref().and_then(|o| o.id.as_deref()),
            Some("42")
        );
    }

    #[test]
    fn parse_short_ranking_item_rejects_missing_id() {
        let v = serde_json::json!({"title": "no id"});
        assert!(parse_short_ranking_item(v).is_none());
    }
}
