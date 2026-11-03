//! Niconico watch data + HLS access rights client.
//!
//! niconico の視聴ページは 2026 年に nvapi v4 へ移行した。Web プレイヤーと
//! 同じ流れで取得する:
//!
//! 1. `POST nvapi.nicovideo.jp/v4/watch/{id}` — 動画・タグ・メディア・コメント
//!    資格情報を返す。**副作用なし** (視聴履歴・再生数に触れない)。`media.hls`
//!    に視聴できる画質を全部含む HLS セッションが事前発行され、同じ応答で
//!    `domand_bid` Cookie も発行される。master と AES 鍵はこの Cookie が無いと
//!    403 になる (実測)。`X-Frontend-*` と `X-Request-With` が無いと 400。
//! 2. `POST /v4/watch/lazy/{id}` — 投稿者・シリーズ。`noSideEffect: true` を
//!    付けないと視聴が記録されてしまうので必ず付ける。鍵 (`lazy.authKey`) は
//!    数分で失効し、失効時は `400 KEY_EXPIRED`。
//! 3. `POST /v4/watch/{id}/side-effect` — 視聴履歴・再生数への記録。Web
//!    プレイヤーは再生開始時に呼ぶ。明示的に呼んだ時だけ記録される。
//! 4. 事前発行が無い時だけ `POST /v1/watch/{id}/access-rights/hls`。
//!
//! watch ページ HTML の `<meta name="server-response">` も `$watchV4` 形に
//! 変わった (投稿者・シリーズ・事前発行 HLS は入らない)。v4 API が使えない時の
//! フォールバックとして読み、旧 (v3) 形の埋め込みも引き続き解釈する。
//!
//! ゲストで得たデータ (`viewer` が null) の `accessRightKey` を `user_session`
//! 付きで access-rights に出すと `400 INVALID_PARAMETER` になる。セッションが
//! 失効していても v4 はゲスト扱いで 200 を返すので、ゲストかどうかは送った
//! Cookie ではなく応答の `viewer` で判断する。access-rights の `actionTrackId`
//! には `client.watchTrackId` を使う (v4 では送った actionTrackId がそのまま
//! 入り、HTML 埋め込みのゲスト鍵もこの値でしか通らない)。
//!
//! Reference shapes: `abeshinzo78/NicoCommentDL`, `kongyo2/niconicojs`,
//! `NNDD-Rebuild/NNDD-RE`, and live probing with a real session.

use async_trait::async_trait;
use rand::Rng;
use regex::Regex;
use reqwest::header;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashSet, sync::OnceLock, time::Duration};

use crate::api::auth::SessionStore;
use crate::error::ApiError;

const WATCH_URL_BASE: &str = "https://www.nicovideo.jp/watch/";
/// access-rights (`{NVAPI_BASE}/v1/watch/`) と v4 watch 系の base。
const NVAPI_BASE: &str = "https://nvapi.nicovideo.jp";
/// v4 watch 系は `X-Request-With` にこの origin を要求する。
const NICOVIDEO_ORIGIN: &str = "https://www.nicovideo.jp";
/// Niconico browser frontend signature. Required on /access-rights/hls and
/// nv-comment requests; constant per `niconico.js`.
const FRONTEND_ID: &str = "6";
const FRONTEND_VERSION: &str = "0";
/// Niconico's watch page is mildly UA-sniffy. A modern Chrome string keeps
/// it serving the React app instead of a fallback page.
const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
/// 応答が返らないまま固まるのを防ぐ。watch 系は数百 ms で返るので十分長い。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 事前発行 HLS の期限がこれ未満しか残っていなければ使わずに取り直す。
const PREISSUED_MIN_REMAINING_SECS: i64 = 120;

fn meta_regex() -> &'static Regex {
    // The pattern is a compile-time constant — `Regex::new` cannot fail
    // here. The targeted allow keeps the workspace `expect_used` deny
    // active for genuine production code paths.
    #[allow(clippy::expect_used)]
    fn build() -> Regex {
        Regex::new(r#"<meta name="server-response" content="([^"]*)""#)
            .expect("static regex compiles")
    }
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(build)
}

pub fn html_unescape(s: &str) -> String {
    let intermediate = s
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    // Decode numeric character references: &#NN; (decimal) and &#xNN; (hex)
    fn numref_regex() -> &'static Regex {
        // The pattern is a compile-time constant — `Regex::new` cannot fail
        // here. The targeted allow keeps the workspace `expect_used` deny
        // active for genuine production code paths.
        #[allow(clippy::expect_used)]
        fn build() -> Regex {
            Regex::new(r"&#(x?[0-9a-fA-F]+);").expect("static regex compiles")
        }
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(build)
    }
    let re = numref_regex();
    let mut result = String::with_capacity(intermediate.len());
    let mut last = 0;
    for cap in re.captures_iter(&intermediate) {
        // `captures_iter` only yields successful matches, so group 0 (the
        // whole match) and group 1 (the mandatory `(x?[0-9a-fA-F]+)`) are
        // always present. Use `let-else` instead of `unwrap` to satisfy
        // the workspace `unwrap_used` deny.
        let Some(m) = cap.get(0) else { continue };
        let Some(num_match) = cap.get(1) else {
            continue;
        };
        let num_str = num_match.as_str();
        let code_point = if let Some(hex) = num_str
            .strip_prefix('x')
            .or_else(|| num_str.strip_prefix('X'))
        {
            u32::from_str_radix(hex, 16).ok()
        } else {
            num_str.parse::<u32>().ok()
        };
        result.push_str(&intermediate[last..m.start()]);
        if let Some(cp) = code_point {
            if let Some(ch) = char::from_u32(cp) {
                result.push(ch);
            } else {
                result.push_str(m.as_str());
            }
        } else {
            result.push_str(m.as_str());
        }
        last = m.end();
    }
    result.push_str(&intermediate[last..]);
    result
}

fn random_action_track_id() -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    let mut id = String::with_capacity(10);
    for _ in 0..10 {
        let idx = rng.gen_range(0..CHARS.len());
        id.push(CHARS[idx] as char);
    }
    let now = chrono::Utc::now().timestamp_millis();
    format!("{id}_{now}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchPageData {
    pub video: WatchVideoMeta,
    pub owner: Option<WatchOwner>,
    pub domand: Option<DomandSetup>,
    pub nv_comment: Option<NvCommentSetup>,
    pub watch_track_id: Option<String>,
    pub series: Option<SeriesInfo>,
    /// データをゲストとして得たか (`viewer` が null)。access-rights へ
    /// `user_session` を付けると 400 になるので、付けるかどうかをこれで決める。
    #[serde(default)]
    pub is_guest: bool,
    /// v4 が事前発行した HLS セッション (`media.hls`)。HTML 埋め込みには無い。
    #[serde(default)]
    pub preissued_hls: Option<HlsAccessRights>,
    /// lazy API (投稿者・シリーズ) の鍵 (`lazy.authKey`)。数分で失効する。
    #[serde(default)]
    pub lazy_key: Option<String>,
    /// 動画のジャンル。未設定 (`isNotSet`) なら `None`。
    #[serde(default)]
    pub genre: Option<WatchGenre>,
    /// `PURELY` (通常視聴) / `PAYMENT_PREVIEW_SUPPORTED` (有料のプレビューのみ) など。
    #[serde(default)]
    pub ok_reason: Option<String>,
    #[serde(default)]
    pub viewer: Option<WatchViewer>,
    /// 視聴制限の手掛かり。再生できない時の説明文を組み立てるのに使う。
    #[serde(default)]
    pub access: WatchAccess,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesInfo {
    pub id: i64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items_count: Option<i64>,
    #[serde(default)]
    pub is_listed: bool,
    /// シリーズ内で 1 つ前の動画 (lazy API の `series.video.prev`)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_video: Option<SeriesVideoRef>,
    /// シリーズ内で次の動画 (`series.video.next`)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_video: Option<SeriesVideoRef>,
    /// シリーズの最初の動画 (`series.video.first`)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_video: Option<SeriesVideoRef>,
}

/// シリーズ内の前後の動画への参照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesVideoRef {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchVideoMeta {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub duration: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered_at: Option<String>,
    #[serde(default)]
    pub is_deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mylist_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub like_count: Option<i64>,
    #[serde(default)]
    pub tags: Vec<VideoTag>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoTag {
    pub name: String,
    #[serde(default)]
    pub is_locked: bool,
    #[serde(default)]
    pub is_category: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchOwner {
    /// `"user"` or `"channel"`.
    pub kind: String,
    /// user は数値 ID、channel は `ch` を外した数値 ID (フロントの
    /// `/user/{id}?kind=channel` と検索結果の `channelId` に揃える)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchGenre {
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchViewer {
    #[serde(default)]
    pub is_premium: bool,
}

/// 視聴制限の手掛かり (v4 の `payment` / `video.permission`、v3 の `payment.video`)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchAccess {
    #[serde(default)]
    pub is_authentication_required: bool,
    /// レンタル / 購入 (PPV)。
    #[serde(default)]
    pub is_ppv: bool,
    /// チャンネル会員限定 (入会が必要)。
    #[serde(default)]
    pub is_admission: bool,
    /// プレミアム会員限定。
    #[serde(default)]
    pub is_premium_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomandSetup {
    pub access_right_key: String,
    pub videos: Vec<MediaTrack>,
    pub audios: Vec<MediaTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaTrack {
    pub id: String,
    #[serde(default)]
    pub is_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit_rate: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_level: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NvCommentSetup {
    pub server: String,
    pub thread_key: String,
    /// Raw `params` object, forwarded verbatim to the threads API.
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HlsAccessRights {
    pub content_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expire_time: Option<String>,
}

/// lazy API が返す投稿者とシリーズ。
#[derive(Debug, Clone, Default)]
pub struct WatchLazyData {
    pub owner: Option<WatchOwner>,
    pub series: Option<SeriesInfo>,
}

impl WatchLazyData {
    /// 取れた分だけ `page` の投稿者・シリーズを差し替える (埋め込みの JSON-LD
    /// 由来の投稿者より lazy の方がアイコン等が正確)。
    pub fn apply_to(self, page: &mut WatchPageData) {
        if self.owner.is_some() {
            page.owner = self.owner;
        }
        if self.series.is_some() {
            page.series = self.series;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityCandidate {
    pub video_track: String,
    pub audio_track: String,
    pub label: Option<String>,
}

#[async_trait]
pub trait WatchApi: Send + Sync {
    async fn fetch_watch_page(&self, video_id: &str) -> Result<WatchPageData, ApiError>;
    async fn fetch_hls(
        &self,
        video_id: &str,
        access_right_key: &str,
        video_track: &str,
        audio_track: &str,
    ) -> Result<HlsAccessRights, ApiError>;
    async fn fetch_hls_outputs(
        &self,
        video_id: &str,
        access_right_key: &str,
        action_track_id: Option<&str>,
        outputs: &[(String, String)],
    ) -> Result<HlsAccessRights, ApiError>;
}

pub struct NiconicoWatchClient {
    http: reqwest::Client,
    watch_base: String,
    hls_base: String,
    nvapi_base: String,
    session: std::sync::Arc<SessionStore>,
}

/// lazy 呼び出しの失敗。鍵の失効だけは watch を取り直して 1 回やり直す。
enum LazyFailure {
    KeyExpired,
    Other(ApiError),
}

impl NiconicoWatchClient {
    pub fn new(session: std::sync::Arc<SessionStore>) -> Result<Self, ApiError> {
        Self::with_endpoints(WATCH_URL_BASE, NVAPI_BASE, session)
    }

    /// 旧来の差し替え口。`hls_base` (`…/v1/watch/`) から nvapi の base を逆算する。
    pub fn with_bases(
        watch_base: &str,
        hls_base: &str,
        session: std::sync::Arc<SessionStore>,
    ) -> Result<Self, ApiError> {
        let trimmed = hls_base.trim_end_matches('/');
        let nvapi_base = trimmed.strip_suffix("/v1/watch").unwrap_or(trimmed);
        Self::build(watch_base, hls_base, nvapi_base, session)
    }

    /// watch ページと nvapi (v1 access-rights / v4 watch) の base を指定する。
    /// テストでは両方 mockito のサーバーに向ける。
    pub fn with_endpoints(
        watch_base: &str,
        nvapi_base: &str,
        session: std::sync::Arc<SessionStore>,
    ) -> Result<Self, ApiError> {
        let nvapi_base = nvapi_base.trim_end_matches('/');
        let hls_base = format!("{nvapi_base}/v1/watch/");
        Self::build(watch_base, &hls_base, nvapi_base, session)
    }

    fn build(
        watch_base: &str,
        hls_base: &str,
        nvapi_base: &str,
        session: std::sync::Arc<SessionStore>,
    ) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .user_agent(BROWSER_UA)
            .gzip(true)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            watch_base: watch_base.to_string(),
            hls_base: hls_base.to_string(),
            nvapi_base: nvapi_base.trim_end_matches('/').to_string(),
            session,
        })
    }

    /// v4 watch 系 (`/v4/watch/...`) へ送る POST の共通ヘッダ。
    fn v4_post(&self, url: &str) -> reqwest::RequestBuilder {
        self.http
            .post(url)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .header(header::REFERER, format!("{NICOVIDEO_ORIGIN}/"))
            .header("X-Frontend-Id", FRONTEND_ID)
            .header("X-Frontend-Version", FRONTEND_VERSION)
            .header("X-Request-With", NICOVIDEO_ORIGIN)
            .header("X-Niconico-Language", "ja-jp")
    }

    /// 視聴データを取得する主経路。v4 API を使い、使えなければ watch ページ HTML
    /// (`$watchV4` / 旧 v3 の埋め込み) に落ちる。削除・非公開などの確定した拒否
    /// ([`ApiError::WatchRefused`]) はフォールバックせずそのまま返す。
    pub async fn fetch_watch_data(&self, video_id: &str) -> Result<WatchPageData, ApiError> {
        ensure_valid_video_id(video_id)?;
        match self.fetch_watch_v4(video_id).await {
            Ok(data) => Ok(data),
            Err(refused @ ApiError::WatchRefused { .. }) => Err(refused),
            Err(primary) => {
                tracing::warn!(
                    video_id,
                    error = %primary,
                    "watch v4 API failed; falling back to the watch page"
                );
                match self.fetch_watch_page(video_id).await {
                    Ok(data) => Ok(data),
                    Err(refused @ ApiError::WatchRefused { .. }) => Err(refused),
                    Err(fallback) => {
                        tracing::warn!(video_id, error = %fallback, "watch page fallback failed too");
                        Err(primary)
                    }
                }
            }
        }
    }

    /// `POST /v4/watch/{id}`。副作用なし。応答の `domand_bid` を控えておく
    /// (事前発行 HLS の master / AES 鍵はこれが無いと 403)。
    ///
    /// Cookie は `user_session` だけを付け、`domand_bid` は送らない。送ると
    /// 応答で発行し直されず、ゲスト / ログインの切り替え後に古い bid が残る。
    pub async fn fetch_watch_v4(&self, video_id: &str) -> Result<WatchPageData, ApiError> {
        ensure_valid_video_id(video_id)?;
        let action_track_id = random_action_track_id();
        let session_cookie = self.session.session_cookie();
        let mut body = serde_json::json!({ "actionTrackId": action_track_id });
        if session_cookie.is_none() {
            body["asGuest"] = Value::Bool(true);
        }

        let url = format!("{}/v4/watch/{video_id}", self.nvapi_base);
        let mut request = self.v4_post(&url);
        if let Some(cookie) = session_cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let response = request.json(&body).send().await?;
        let status = response.status();
        let bid = capture_set_cookie(response.headers(), "domand_bid");
        let bytes = response.bytes().await?;
        if !status.is_success() {
            return Err(watch_http_error("watch v4", status.as_u16(), &bytes));
        }

        let mut data = parse_watch_v4_response(&bytes)?;
        if data.watch_track_id.is_none() {
            data.watch_track_id = Some(action_track_id);
        }
        if let Some(bid) = bid {
            tracing::debug!(bid_len = bid.len(), "captured domand_bid from watch v4");
            self.session.set_domand_bid(bid);
        }
        if data.is_guest && self.session.is_set() {
            tracing::warn!(
                video_id,
                "watch v4 answered as a guest although a user_session is set; the session may have expired"
            );
        }
        Ok(data)
    }

    /// `POST /v4/watch/lazy/{id}` (`noSideEffect: true`)。鍵が失効していたら
    /// watch を取り直して 1 回だけやり直す。
    pub async fn fetch_watch_lazy(&self, page: &WatchPageData) -> Result<WatchLazyData, ApiError> {
        let video_id = page.video.id.as_str();
        ensure_valid_video_id(video_id)?;
        let (Some(key), Some(track)) = (page.lazy_key.as_deref(), page.watch_track_id.as_deref())
        else {
            return Err(ApiError::ResponseShape(
                "watch data carries no lazy key".into(),
            ));
        };
        match self.post_lazy(video_id, key, track, page.is_guest).await {
            Ok(lazy) => Ok(lazy),
            Err(LazyFailure::Other(e)) => Err(e),
            Err(LazyFailure::KeyExpired) => {
                tracing::debug!(
                    video_id,
                    "watch lazy key expired; refetching watch data once"
                );
                let fresh = self.fetch_watch_v4(video_id).await?;
                let (Some(key), Some(track)) =
                    (fresh.lazy_key.as_deref(), fresh.watch_track_id.as_deref())
                else {
                    return Err(ApiError::ResponseShape(
                        "refetched watch data carries no lazy key".into(),
                    ));
                };
                match self.post_lazy(video_id, key, track, fresh.is_guest).await {
                    Ok(lazy) => Ok(lazy),
                    Err(LazyFailure::KeyExpired) => Err(ApiError::ResponseShape(
                        "watch lazy rejected a freshly issued key (KEY_EXPIRED)".into(),
                    )),
                    Err(LazyFailure::Other(e)) => Err(e),
                }
            }
        }
    }

    async fn post_lazy(
        &self,
        video_id: &str,
        key: &str,
        action_track_id: &str,
        guest: bool,
    ) -> Result<WatchLazyData, LazyFailure> {
        let url = format!("{}/v4/watch/lazy/{video_id}", self.nvapi_base);
        let body = serde_json::json!({
            "actionTrackId": action_track_id,
            "keyToken": key,
            // 付けないと視聴が記録される (視聴履歴・再生数)。
            "noSideEffect": true,
        });
        let mut request = self.v4_post(&url);
        if !guest {
            if let Some(cookie) = self.session.session_cookie() {
                request = request.header(header::COOKIE, cookie);
            }
        }
        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|e| LazyFailure::Other(e.into()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| LazyFailure::Other(e.into()))?;
        if !status.is_success() {
            if nvapi_error_code(&bytes).as_deref() == Some("KEY_EXPIRED") {
                return Err(LazyFailure::KeyExpired);
            }
            return Err(LazyFailure::Other(watch_http_error(
                "watch lazy",
                status.as_u16(),
                &bytes,
            )));
        }
        parse_watch_lazy_response(&bytes).map_err(LazyFailure::Other)
    }

    /// lazy API で投稿者とシリーズを補う。失敗しても再生は続けられるので
    /// ログだけ残して埋め込みデータのまま返す。
    pub async fn enrich_with_lazy(&self, page: &mut WatchPageData) {
        if page.lazy_key.is_none() {
            return;
        }
        match self.fetch_watch_lazy(page).await {
            Ok(lazy) => lazy.apply_to(page),
            Err(e) => {
                tracing::warn!(
                    video_id = %page.video.id,
                    error = %e,
                    "watch lazy failed; owner/series fall back to the embedded data"
                );
            }
        }
    }

    /// `POST /v4/watch/{id}/side-effect` — 視聴を niconico に記録する
    /// (ログイン中なら視聴履歴、ゲストでも再生数)。Web プレイヤーが再生開始時に
    /// 送るのと同じ。`action_track_id` は watch 取得時の `watchTrackId`。
    pub async fn record_view(
        &self,
        video_id: &str,
        action_track_id: &str,
        as_guest: bool,
    ) -> Result<(), ApiError> {
        ensure_valid_video_id(video_id)?;
        if action_track_id.trim().is_empty() {
            return Err(ApiError::InvalidQuery("actionTrackId is empty".into()));
        }
        let url = format!("{}/v4/watch/{video_id}/side-effect", self.nvapi_base);
        let mut request = self.v4_post(&url);
        if !as_guest {
            if let Some(cookie) = self.session.session_cookie() {
                request = request.header(header::COOKIE, cookie);
            }
        }
        let body = serde_json::json!({ "actionTrackId": action_track_id });
        let response = request.json(&body).send().await?;
        let status = response.status();
        if !status.is_success() {
            let bytes = response.bytes().await.unwrap_or_default();
            return Err(watch_http_error(
                "watch side-effect",
                status.as_u16(),
                &bytes,
            ));
        }
        Ok(())
    }

    /// 再生用の HLS URL を決める。v4 の事前発行があり、鍵に要る `domand_bid`
    /// も控えてあればそれを使う (追加リクエスト不要)。無ければ access-rights に
    /// `outputs` を全部出して master を発行させる。
    pub async fn resolve_hls(
        &self,
        page: &WatchPageData,
        outputs: &[(String, String)],
    ) -> Result<HlsAccessRights, ApiError> {
        if let Some(pre) = page.preissued_hls.as_ref() {
            if self.session.domand_bid().is_some()
                && !expires_within(pre.expire_time.as_deref(), PREISSUED_MIN_REMAINING_SECS)
            {
                return Ok(pre.clone());
            }
        }
        let domand = page.domand.as_ref().ok_or_else(|| {
            ApiError::ResponseShape("watch data carries no media.accessRightKey".into())
        })?;
        self.request_access_rights(
            &page.video.id,
            &domand.access_right_key,
            page.watch_track_id.as_deref(),
            outputs,
            !page.is_guest,
        )
        .await
    }

    /// `POST /v1/watch/{id}/access-rights/hls`。`with_session` が false (ゲストの
    /// 鍵) なら `user_session` を付けない — 付けると 400 INVALID_PARAMETER。
    async fn request_access_rights(
        &self,
        video_id: &str,
        access_right_key: &str,
        action_track_id: Option<&str>,
        outputs: &[(String, String)],
        with_session: bool,
    ) -> Result<HlsAccessRights, ApiError> {
        if outputs.is_empty() {
            return Err(ApiError::InvalidQuery(
                "HLS outputs must contain at least one video/audio pair".into(),
            ));
        }

        let action_track_id = action_track_id
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(random_action_track_id);
        let url = format!(
            "{}{video_id}/access-rights/hls?actionTrackId={}",
            self.hls_base,
            urlencoding_simple(&action_track_id)
        );

        let outputs: Vec<[&str; 2]> = outputs
            .iter()
            .map(|(video, audio)| [video.as_str(), audio.as_str()])
            .collect();
        let body = serde_json::json!({
            "outputs": outputs,
        });

        let mut request = self
            .http
            .post(&url)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json;charset=utf-8")
            .header("X-Access-Right-Key", access_right_key)
            .header("X-Frontend-Id", FRONTEND_ID)
            .header("X-Frontend-Version", FRONTEND_VERSION)
            .header("X-Request-With", "nicovideo")
            .header("X-Niconico-Language", "ja-jp");
        let mut cookie_parts = Vec::with_capacity(2);
        if with_session {
            if let Some(cookie) = self.session.session_cookie() {
                cookie_parts.push(cookie);
            }
        }
        if let Some(bid) = self.session.domand_bid() {
            cookie_parts.push(format!("domand_bid={bid}"));
        }
        if !cookie_parts.is_empty() {
            request = request.header(header::COOKIE, cookie_parts.join("; "));
        }

        let response = request.json(&body).send().await?;
        let status = response.status();

        // Capture `domand_bid` Set-Cookie BEFORE consuming the body —
        // niconico's CloudFront rejects subsequent HLS fragment fetches
        // with 403 unless this cookie is forwarded.
        let captured_bid = capture_set_cookie(response.headers(), "domand_bid");

        let bytes = response.bytes().await?;

        if !status.is_success() {
            let detail = String::from_utf8_lossy(&bytes).into_owned();
            return Err(ApiError::ServerError {
                status: status.as_u16(),
                message: format!("HLS access-rights {status}: {detail}"),
            });
        }

        #[derive(Deserialize)]
        struct Wrapper {
            meta: WrapperMeta,
            data: HlsAccessRights,
        }
        #[derive(Deserialize)]
        struct WrapperMeta {
            status: u16,
        }

        let parsed: Wrapper = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::ResponseShape(format!("failed to parse HLS body: {e}")))?;
        if parsed.meta.status != 201 {
            return Err(ApiError::ResponseShape(format!(
                "unexpected meta.status from HLS API: {}",
                parsed.meta.status
            )));
        }
        if let Some(bid) = captured_bid {
            tracing::debug!(bid_len = bid.len(), "captured domand_bid");
            self.session.set_domand_bid(bid);
        } else if self.session.domand_bid().is_none() {
            tracing::warn!("access-rights/hls succeeded but no domand_bid cookie present");
        }
        Ok(parsed.data)
    }
}

#[async_trait]
impl WatchApi for NiconicoWatchClient {
    /// watch ページ HTML を取って埋め込み JSON を読む (`$watchV4` / 旧 v3 形)。
    /// v4 API が使えない時のフォールバック。投稿者・シリーズ・事前発行 HLS は
    /// 入っていないので、必要なら [`NiconicoWatchClient::enrich_with_lazy`] で補う。
    async fn fetch_watch_page(&self, video_id: &str) -> Result<WatchPageData, ApiError> {
        ensure_valid_video_id(video_id)?;

        let url = format!("{}{video_id}", self.watch_base);
        let mut request = self
            .http
            .get(&url)
            .header(header::ACCEPT, "text/html,application/xhtml+xml");
        if let Some(cookie) = self.session.cookie_header() {
            request = request.header(header::COOKIE, cookie);
        }

        let response = request.send().await?;
        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            return Err(ApiError::ServerError {
                status: status.as_u16(),
                message: format!("watch page returned {status}"),
            });
        }

        parse_watch_html(&body)
    }

    async fn fetch_hls(
        &self,
        video_id: &str,
        access_right_key: &str,
        video_track: &str,
        audio_track: &str,
    ) -> Result<HlsAccessRights, ApiError> {
        self.fetch_hls_outputs(
            video_id,
            access_right_key,
            None,
            &[(video_track.to_string(), audio_track.to_string())],
        )
        .await
    }

    async fn fetch_hls_outputs(
        &self,
        video_id: &str,
        access_right_key: &str,
        action_track_id: Option<&str>,
        outputs: &[(String, String)],
    ) -> Result<HlsAccessRights, ApiError> {
        self.request_access_rights(
            video_id,
            access_right_key,
            action_track_id,
            outputs,
            self.session.is_set(),
        )
        .await
    }
}

/// `Set-Cookie` から `name` の値を拾う。空値は無視する。
fn capture_set_cookie(headers: &header::HeaderMap, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|raw| raw.strip_prefix(prefix.as_str()))
        .map(|rest| rest.split(';').next().unwrap_or("").to_string())
        .find(|value| !value.is_empty())
}

/// RFC 3339 の期限まで `secs` 秒を切っているか。読めない値は「切れていない」扱い
/// (使ってみて 403 なら呼び出し側の再発行に任せる)。
fn expires_within(expire_time: Option<&str>, secs: i64) -> bool {
    let Some(raw) = expire_time else {
        return false;
    };
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(at) => at.timestamp() - chrono::Utc::now().timestamp() < secs,
        Err(_) => false,
    }
}

/// nvapi のエラー応答 `{ meta: { errorCode } }` からコードだけ取り出す。
fn nvapi_error_code(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    value
        .pointer("/meta/errorCode")
        .and_then(Value::as_str)
        .map(String::from)
}

/// watch 系の非 2xx 応答を [`ApiError`] にする。削除・存在しない・メンテなど
/// フォールバックしても変わらないものは [`ApiError::WatchRefused`]。
fn watch_http_error(label: &str, status: u16, body: &[u8]) -> ApiError {
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    let error_code = parsed
        .as_ref()
        .and_then(|v| v.pointer("/meta/errorCode"))
        .and_then(Value::as_str)
        .map(String::from);
    let reason_code = parsed
        .as_ref()
        .and_then(|v| v.pointer("/data/reasonCode"))
        .and_then(Value::as_str)
        .map(String::from);
    let definitive = matches!(status, 403 | 404 | 410)
        || error_code.as_deref() == Some("MAINTENANCE")
        || reason_code.is_some();
    if definitive {
        return ApiError::WatchRefused {
            status,
            message: watch_refusal_message(status, error_code.as_deref(), reason_code.as_deref()),
            error_code,
            reason_code,
        };
    }
    let detail: String = String::from_utf8_lossy(body).chars().take(300).collect();
    ApiError::ServerError {
        status,
        message: format!("{label} returned {status}: {detail}"),
    }
}

/// 拒否理由を利用者向けの文に。末尾に診断用の生コードを付ける。
pub fn watch_refusal_message(
    status: u16,
    error_code: Option<&str>,
    reason_code: Option<&str>,
) -> String {
    let base = match reason_code {
        Some("ADMINISTRATOR_DELETE_VIDEO") => "この動画は運営によって削除されています",
        Some("RIGHT_HOLDER_DELETE_VIDEO") => "この動画は権利者の申し立てにより削除されています",
        Some("CONTRACT_DELETE_VIDEO") => "この動画は契約の都合により削除されています",
        Some("DELETED_VIDEO") | Some("OWNER_DELETE_VIDEO") => {
            "この動画は投稿者によって削除されています"
        }
        Some("PRIVATE_VIDEO") | Some("HIDDEN_VIDEO") => "この動画は非公開です",
        Some("DOMESTIC_VIDEO") => "この動画は日本国内からのみ視聴できます",
        Some("NOT_FOUND") => "動画が見つかりません（削除されたか、ID が間違っています）",
        _ => match (status, error_code) {
            (_, Some("MAINTENANCE")) => {
                "ニコニコ動画はメンテナンス中です。しばらくしてから再度お試しください"
            }
            (404, _) | (_, Some("NOT_FOUND")) => {
                "動画が見つかりません（削除されたか、ID が間違っています）"
            }
            (401, _) => "この動画の視聴にはログインが必要です",
            _ => "この動画は視聴できません（削除・非公開・視聴制限など）",
        },
    };
    let codes: Vec<&str> = [error_code, reason_code].into_iter().flatten().collect();
    if codes.is_empty() {
        format!("{base} (HTTP {status})")
    } else {
        format!("{base} (HTTP {status} {})", codes.join(" / "))
    }
}

/// 再生情報 (`media`) が無い時に、なぜ再生できないかを利用者向けに説明する。
pub fn playback_unavailable_message(page: &WatchPageData) -> String {
    let premium_viewer = page.viewer.as_ref().is_some_and(|v| v.is_premium);
    if page.video.is_deleted {
        return "この動画は削除されています".into();
    }
    if page.access.is_admission {
        return "チャンネル会員限定の動画です（チャンネルへの入会が必要です）".into();
    }
    if page.access.is_ppv {
        return "有料 (レンタル・購入) の動画です".into();
    }
    if page.access.is_premium_only && !premium_viewer {
        return "プレミアム会員限定の動画です".into();
    }
    if page.is_guest && page.access.is_authentication_required {
        return "この動画の視聴にはログインが必要です".into();
    }
    match page.ok_reason.as_deref() {
        Some("PAYMENT_PREVIEW_SUPPORTED") => {
            "有料動画のため、プレビュー以外は再生できません".into()
        }
        _ => {
            "この動画は再生情報が取得できません（削除済み・プレミアム限定・要ログインなど）".into()
        }
    }
}

/// Lightweight URL component encoder for the small set of chars that appear
/// in actionTrackId — pulling in `urlencoding` would add another dep just
/// for this. The id only contains `[A-Za-z0-9_]` so this is a no-op in
/// practice; we keep it for safety if generation ever changes.
fn urlencoding_simple(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            for byte in ch.to_string().as_bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

fn is_valid_video_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn ensure_valid_video_id(video_id: &str) -> Result<(), ApiError> {
    if is_valid_video_id(video_id) {
        Ok(())
    } else {
        Err(ApiError::InvalidQuery(format!(
            "invalid video id {video_id:?}"
        )))
    }
}

/// Public for unit-testability — extracts the `server-response` JSON and
/// projects the subset we care about. `$watchV4` 形 (2026-) と旧 v3 形の両方を読む。
pub fn parse_watch_html(html: &str) -> Result<WatchPageData, ApiError> {
    let captures = meta_regex().captures(html).ok_or_else(|| {
        ApiError::ResponseShape("watch page missing <meta name=\"server-response\">".into())
    })?;
    let raw = captures
        .get(1)
        .ok_or_else(|| ApiError::ResponseShape("server-response capture group missing".into()))?
        .as_str();
    let decoded = html_unescape(raw);
    let value: Value = serde_json::from_str(&decoded)
        .map_err(|e| ApiError::ResponseShape(format!("server-response JSON parse failed: {e}")))?;
    if let Some(v4) = value.pointer("/data/response/$watchV4/data") {
        return project_watch_v4(v4);
    }
    project_watch_data(&value)
}

/// `POST /v4/watch/{id}` の応答本文 (`{ meta, data }`) を読む。
pub fn parse_watch_v4_response(bytes: &[u8]) -> Result<WatchPageData, ApiError> {
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|e| ApiError::ResponseShape(format!("watch v4 JSON parse failed: {e}")))?;
    let data = root
        .get("data")
        .filter(|d| d.is_object())
        .ok_or_else(|| ApiError::ResponseShape("watch v4 response has no data".into()))?;
    project_watch_v4(data)
}

/// `POST /v4/watch/lazy/{id}` の応答本文を読む。
pub fn parse_watch_lazy_response(bytes: &[u8]) -> Result<WatchLazyData, ApiError> {
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|e| ApiError::ResponseShape(format!("watch lazy JSON parse failed: {e}")))?;
    let data = root
        .get("data")
        .filter(|d| d.is_object())
        .ok_or_else(|| ApiError::ResponseShape("watch lazy response has no data".into()))?;
    Ok(WatchLazyData {
        owner: data.get("owner").and_then(parse_lazy_owner),
        series: data.get("series").and_then(parse_lazy_series),
    })
}

/// 投稿者 ID を表示・遷移に使う形へ揃える。channel は `ch` を外した数値にする
/// (フロントは `/user/{id}?kind=channel` から `ch{id}` を組み立てる)。
pub fn normalize_owner_id(kind: &str, raw: &str) -> String {
    let raw = raw.trim();
    if kind == "channel" {
        if let Some(rest) = raw.strip_prefix("ch") {
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                return rest.to_string();
            }
        }
    }
    raw.to_string()
}

fn non_empty_str(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn bool_at(value: &Value, pointer: &str) -> bool {
    value
        .pointer(pointer)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// v4 の watch データ (`data` / `$watchV4.data`) を射影する。
fn project_watch_v4(data: &Value) -> Result<WatchPageData, ApiError> {
    let video_node = data
        .get("video")
        .filter(|v| v.is_object())
        .ok_or_else(|| ApiError::ResponseShape("watch v4 data has no video".into()))?;

    // `normal` が安定した素の URL。`player` / `ogp` は署名付きで失効する。
    let thumbnail_url = ["normal", "large", "middle", "short", "player", "ogp"]
        .iter()
        .find_map(|key| non_empty_str(video_node.pointer(&format!("/thumbnail/{key}"))));

    let video = WatchVideoMeta {
        id: video_node
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::ResponseShape("video.id missing".into()))?
            .to_string(),
        title: video_node
            .get("title")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::ResponseShape("video.title missing".into()))?
            .to_string(),
        description: video_node
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        duration: video_node
            .get("duration")
            .and_then(Value::as_i64)
            .ok_or_else(|| ApiError::ResponseShape("video.duration missing".into()))?,
        thumbnail_url,
        registered_at: non_empty_str(video_node.get("registeredAt")),
        is_deleted: bool_at(video_node, "/permission/isDeleted"),
        view_count: video_node.pointer("/count/view").and_then(Value::as_i64),
        comment_count: video_node.pointer("/count/comment").and_then(Value::as_i64),
        mylist_count: video_node.pointer("/count/mylist").and_then(Value::as_i64),
        like_count: video_node.pointer("/count/like").and_then(Value::as_i64),
        tags: parse_tags(data.pointer("/tags/items")),
        content_type: non_empty_str(video_node.get("contentType")),
    };

    let media = data.get("media").filter(|m| m.is_object());
    let domand = media.and_then(|m| {
        let access_right_key = m.get("accessRightKey")?.as_str()?.to_string();
        Some(DomandSetup {
            access_right_key,
            videos: parse_tracks(m.pointer("/contents/videos")),
            audios: parse_tracks(m.pointer("/contents/audios")),
        })
    });
    let preissued_hls = media
        .and_then(|m| m.get("hls"))
        .filter(|h| h.is_object())
        .and_then(|h| {
            Some(HlsAccessRights {
                content_url: non_empty_str(h.get("url"))?,
                create_time: non_empty_str(h.get("createdAt")),
                expire_time: non_empty_str(h.get("expiredAt")),
            })
        });

    let viewer = data
        .get("viewer")
        .filter(|v| v.is_object())
        .map(|v| WatchViewer {
            is_premium: v.get("isPremium").and_then(Value::as_bool).unwrap_or(false),
        });

    Ok(WatchPageData {
        video,
        // 投稿者は lazy で取る。それまでは JSON-LD の owner で埋めておく。
        owner: data
            .pointer("/metadata/jsonLd/owner")
            .and_then(parse_json_ld_owner),
        domand,
        nv_comment: data
            .pointer("/comment/nvComment")
            .and_then(parse_nv_comment),
        watch_track_id: non_empty_str(data.pointer("/client/watchTrackId")),
        series: None,
        is_guest: viewer.is_none(),
        preissued_hls,
        lazy_key: non_empty_str(data.pointer("/lazy/authKey")),
        genre: data.get("genre").and_then(parse_genre),
        ok_reason: non_empty_str(data.get("okReason")),
        viewer,
        access: WatchAccess {
            is_authentication_required: bool_at(video_node, "/permission/isAuthenticationRequired"),
            is_ppv: bool_at(data, "/payment/ppv/isEnabled"),
            is_admission: bool_at(data, "/payment/admission/isEnabled"),
            is_premium_only: bool_at(data, "/payment/premium/isEnabled"),
        },
    })
}

/// 旧 (v3) 形の埋め込み `/data/response/{video,owner,media.domand,…}` を射影する。
fn project_watch_data(root: &Value) -> Result<WatchPageData, ApiError> {
    let response = root
        .pointer("/data/response")
        .ok_or_else(|| ApiError::ResponseShape("missing /data/response".into()))?;

    let video_node = response
        .get("video")
        .ok_or_else(|| ApiError::ResponseShape("missing /data/response/video".into()))?;

    let video = WatchVideoMeta {
        id: video_node
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::ResponseShape("video.id missing".into()))?
            .to_string(),
        title: video_node
            .get("title")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::ResponseShape("video.title missing".into()))?
            .to_string(),
        description: video_node
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        duration: video_node
            .get("duration")
            .and_then(Value::as_i64)
            .ok_or_else(|| ApiError::ResponseShape("video.duration missing".into()))?,
        thumbnail_url: video_node
            .pointer("/thumbnail/url")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .or_else(|| {
                video_node
                    .pointer("/thumbnail/largeUrl")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string())
            }),
        registered_at: video_node
            .get("registeredAt")
            .and_then(Value::as_str)
            .map(|s| s.to_string()),
        is_deleted: video_node
            .get("isDeleted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        view_count: video_node.pointer("/count/view").and_then(Value::as_i64),
        comment_count: video_node.pointer("/count/comment").and_then(Value::as_i64),
        mylist_count: video_node.pointer("/count/mylist").and_then(Value::as_i64),
        like_count: video_node.pointer("/count/like").and_then(Value::as_i64),
        tags: parse_tags(response.pointer("/tag/items")),
        content_type: video_node
            .get("contentType")
            .and_then(Value::as_str)
            .map(|s| s.to_string()),
    };

    // v3 のチャンネル動画は `owner` が null で `channel` に投稿元が入る。
    let owner = response
        .get("owner")
        .and_then(parse_owner)
        .or_else(|| response.get("channel").and_then(parse_v3_channel_owner));
    let domand = response.pointer("/media/domand").and_then(parse_domand);
    let nv_comment = response
        .pointer("/comment/nvComment")
        .and_then(parse_nv_comment);
    let watch_track_id = response
        .pointer("/client/watchTrackId")
        .and_then(Value::as_str)
        .map(String::from);

    let series = response.get("series").and_then(|s| {
        Some(SeriesInfo {
            id: s.get("id")?.as_i64()?,
            title: s.get("title")?.as_str()?.to_string(),
            description: s
                .get("description")
                .and_then(Value::as_str)
                .map(String::from),
            thumbnail_url: s
                .pointer("/thumbnail/url")
                .and_then(Value::as_str)
                .map(String::from)
                .or_else(|| {
                    s.get("thumbnailUrl")
                        .and_then(Value::as_str)
                        .map(String::from)
                }),
            items_count: s.get("itemsCount").and_then(Value::as_i64),
            is_listed: s.get("isListed").and_then(Value::as_bool).unwrap_or(true),
            prev_video: None,
            next_video: None,
            first_video: None,
        })
    });

    let viewer = response
        .get("viewer")
        .filter(|v| v.is_object())
        .map(|v| WatchViewer {
            is_premium: v.get("isPremium").and_then(Value::as_bool).unwrap_or(false),
        });

    Ok(WatchPageData {
        video,
        owner,
        domand,
        nv_comment,
        watch_track_id,
        series,
        is_guest: viewer.is_none(),
        preissued_hls: None,
        lazy_key: None,
        genre: response.get("genre").and_then(parse_genre),
        ok_reason: non_empty_str(response.get("okReason")),
        viewer,
        access: WatchAccess {
            is_authentication_required: bool_at(video_node, "/isAuthenticationRequired"),
            is_ppv: bool_at(response, "/payment/video/isPpv"),
            is_admission: bool_at(response, "/payment/video/isAdmission"),
            is_premium_only: bool_at(response, "/payment/video/isPremium"),
        },
    })
}

/// `value` を JSON 配列として読み、各要素に `parse_item` を適用して `None` を
/// 捨てた `Vec<T>` を返す。配列でない/`None` の場合は空 Vec。
fn parse_json_array<T>(
    value: Option<&Value>,
    parse_item: impl FnMut(&Value) -> Option<T>,
) -> Vec<T> {
    let Some(arr) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter().filter_map(parse_item).collect()
}

/// niconico の `id` 系フィールドは API バージョンによって i64 / 数字文字列 /
/// 文字列の 3 種で返ってくる。どれでも文字列として受け取れるように正規化する。
pub fn json_value_as_id_string(v: &Value) -> Option<String> {
    v.as_i64()
        .map(|n| n.to_string())
        .or_else(|| v.as_str().map(String::from))
}

fn parse_tags(value: Option<&Value>) -> Vec<VideoTag> {
    parse_json_array(value, |node| {
        Some(VideoTag {
            name: node.get("name")?.as_str()?.to_string(),
            is_locked: node
                .get("isLocked")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            // v4 にはカテゴリタグの概念が無い (ジャンルに置き換わった)。
            is_category: node
                .get("isCategory")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    })
}

fn parse_genre(node: &Value) -> Option<WatchGenre> {
    if !node.is_object() || bool_at(node, "/isNotSet") {
        return None;
    }
    let key = non_empty_str(node.get("key"))?;
    if key == "none" {
        return None;
    }
    Some(WatchGenre {
        label: non_empty_str(node.get("label")).unwrap_or_else(|| key.clone()),
        key,
    })
}

fn parse_owner(node: &Value) -> Option<WatchOwner> {
    if node.is_null() {
        return None;
    }
    let kind = node
        .get("ownerType")
        .and_then(Value::as_str)
        .unwrap_or("user")
        .to_string();
    Some(WatchOwner {
        id: node
            .get("id")
            .and_then(json_value_as_id_string)
            .map(|id| normalize_owner_id(&kind, &id)),
        kind,
        nickname: node
            .get("nickname")
            .and_then(Value::as_str)
            .map(String::from),
        icon_url: node
            .get("iconUrl")
            .and_then(Value::as_str)
            .map(String::from),
    })
}

fn parse_v3_channel_owner(node: &Value) -> Option<WatchOwner> {
    let raw_id = node.get("id").and_then(json_value_as_id_string)?;
    Some(WatchOwner {
        kind: "channel".into(),
        id: Some(normalize_owner_id("channel", &raw_id)),
        nickname: non_empty_str(node.get("name")),
        icon_url: non_empty_str(node.pointer("/thumbnail/url")),
    })
}

/// v4 `metadata.jsonLd.owner` (`{ id, type, name, iconUrl }`)。lazy が取れない
/// 時の投稿者情報。
fn parse_json_ld_owner(node: &Value) -> Option<WatchOwner> {
    if !node.is_object() {
        return None;
    }
    let kind = non_empty_str(node.get("type")).unwrap_or_else(|| "user".into());
    Some(WatchOwner {
        id: node
            .get("id")
            .and_then(json_value_as_id_string)
            .map(|id| normalize_owner_id(&kind, &id)),
        nickname: non_empty_str(node.get("name")),
        icon_url: non_empty_str(node.get("iconUrl")),
        kind,
    })
}

/// lazy の `owner`。user は `{ id: number, nickname, icon.url }`、channel は
/// `{ id: "ch…", name, thumbnail.url }`。
fn parse_lazy_owner(node: &Value) -> Option<WatchOwner> {
    if !node.is_object() {
        return None;
    }
    let kind = non_empty_str(node.get("type")).unwrap_or_else(|| "user".into());
    let nickname = non_empty_str(node.get("nickname")).or_else(|| non_empty_str(node.get("name")));
    let icon_url = non_empty_str(node.pointer("/icon/url"))
        .or_else(|| non_empty_str(node.pointer("/thumbnail/url")))
        .or_else(|| non_empty_str(node.get("iconUrl")));
    Some(WatchOwner {
        id: node
            .get("id")
            .and_then(json_value_as_id_string)
            .map(|id| normalize_owner_id(&kind, &id)),
        kind,
        nickname,
        icon_url,
    })
}

fn parse_series_video_ref(node: Option<&Value>) -> Option<SeriesVideoRef> {
    let node = node.filter(|n| n.is_object())?;
    Some(SeriesVideoRef {
        id: node.get("id").and_then(json_value_as_id_string)?,
        title: node
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        thumbnail_url: ["url", "listingUrl", "middleUrl"]
            .iter()
            .find_map(|key| non_empty_str(node.pointer(&format!("/thumbnail/{key}")))),
    })
}

/// lazy の `series` (`{ id, title, description, thumbnailUrl, video: { prev, next, first } }`)。
fn parse_lazy_series(node: &Value) -> Option<SeriesInfo> {
    if !node.is_object() {
        return None;
    }
    Some(SeriesInfo {
        id: node.get("id").and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })?,
        title: node.get("title").and_then(Value::as_str)?.to_string(),
        description: non_empty_str(node.get("description")),
        thumbnail_url: non_empty_str(node.get("thumbnailUrl")),
        items_count: node.get("itemsCount").and_then(Value::as_i64),
        is_listed: node
            .get("isListed")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        prev_video: parse_series_video_ref(node.pointer("/video/prev")),
        next_video: parse_series_video_ref(node.pointer("/video/next")),
        first_video: parse_series_video_ref(node.pointer("/video/first")),
    })
}

fn parse_domand(node: &Value) -> Option<DomandSetup> {
    let access_right_key = node.get("accessRightKey")?.as_str()?.to_string();
    let videos = parse_tracks(node.get("videos"));
    let audios = parse_tracks(node.get("audios"));
    Some(DomandSetup {
        access_right_key,
        videos,
        audios,
    })
}

fn parse_tracks(value: Option<&Value>) -> Vec<MediaTrack> {
    parse_json_array(value, |node| {
        Some(MediaTrack {
            id: node.get("id")?.as_str()?.to_string(),
            is_available: node
                .get("isAvailable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            label: node.get("label").and_then(Value::as_str).map(String::from),
            bit_rate: node.get("bitRate").and_then(Value::as_i64),
            width: node.get("width").and_then(Value::as_i64),
            height: node.get("height").and_then(Value::as_i64),
            quality_level: node.get("qualityLevel").and_then(Value::as_i64),
        })
    })
}

fn parse_nv_comment(node: &Value) -> Option<NvCommentSetup> {
    Some(NvCommentSetup {
        server: node.get("server")?.as_str()?.to_string(),
        thread_key: node.get("threadKey")?.as_str()?.to_string(),
        params: node.get("params").cloned().unwrap_or(Value::Null),
    })
}

#[derive(Debug, Clone, Default)]
struct AuthorizedQualities {
    videos: HashSet<String>,
    audios: HashSet<String>,
}

fn authorized_qualities(access_right_key: &str) -> Option<AuthorizedQualities> {
    let payload = access_right_key.split('.').nth(1)?;
    let bytes = base64url_decode(payload)?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;

    Some(AuthorizedQualities {
        videos: string_set(value.get("v")),
        audios: string_set(value.get("a")),
    })
}

fn string_set(value: Option<&Value>) -> HashSet<String> {
    value
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let mut buffer = 0u32;
    let mut bits = 0u8;
    let mut out = Vec::with_capacity(input.len() * 3 / 4);

    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32;

        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }

    Some(out)
}

fn is_authorized_track(track: &MediaTrack, allowed: &HashSet<String>) -> bool {
    allowed.is_empty() || allowed.contains(&track.id)
}

/// Build playable quality candidates, ordered from best to worst.
///
/// Niconico can mark a track as available while the `accessRightKey` only
/// authorizes a subset. HLS access-rights returns HTTP 400 INVALID_PARAMETER
/// when an unauthorized or incompatible pair is submitted, so callers should
/// try these in order until one succeeds.
pub fn quality_candidates(domand: &DomandSetup) -> Vec<QualityCandidate> {
    let authorized = authorized_qualities(&domand.access_right_key).unwrap_or_default();

    let mut videos: Vec<&MediaTrack> = domand
        .videos
        .iter()
        .filter(|v| v.is_available && is_authorized_track(v, &authorized.videos))
        .collect();
    videos.sort_by(|a, b| {
        b.quality_level
            .unwrap_or(0)
            .cmp(&a.quality_level.unwrap_or(0))
            .then_with(|| b.bit_rate.unwrap_or(0).cmp(&a.bit_rate.unwrap_or(0)))
    });

    let mut audios: Vec<&MediaTrack> = domand
        .audios
        .iter()
        .filter(|a| a.is_available && is_authorized_track(a, &authorized.audios))
        .collect();
    audios.sort_by(|a, b| {
        b.quality_level
            .unwrap_or(0)
            .cmp(&a.quality_level.unwrap_or(0))
            .then_with(|| b.bit_rate.unwrap_or(0).cmp(&a.bit_rate.unwrap_or(0)))
    });

    let mut candidates = Vec::new();
    for video in videos {
        for audio in &audios {
            candidates.push(QualityCandidate {
                video_track: video.id.clone(),
                audio_track: audio.id.clone(),
                label: video.label.clone(),
            });
        }
    }
    candidates
}

/// Pick the highest-quality available video track + the best audio track.
/// Returns the IDs in `(video_id, audio_id, label)` form.
pub fn pick_best_quality(domand: &DomandSetup) -> Option<(String, String, Option<String>)> {
    quality_candidates(domand)
        .into_iter()
        .next()
        .map(|candidate| {
            (
                candidate.video_track,
                candidate.audio_track,
                candidate.label,
            )
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // 2026-10-31 にゲスト (Cookie なし) で取得した実応答。accessRightKey は
    // 画質 ID (`v`/`a`) だけを残した署名なしの JWT に、HLS の署名付き URL・
    // threadKey・lazy.authKey・nicosid・watchTrackId・editKey・追跡用 ID は
    // 無害な値に置き換えてある。watch ページは server-response の meta だけ、
    // チャンネル一覧は一覧部分だけを残している。取り直す時も同じ値を伏せること。
    const V4_PAGE_SM9: &str =
        include_str!("../../tests/fixtures/niconico/watch_page_v4_guest_sm9.html");
    const V4_API_SERIES: &str =
        include_str!("../../tests/fixtures/niconico/watch_v4_guest_sm41984256.json");
    const V4_API_CHANNEL: &str =
        include_str!("../../tests/fixtures/niconico/watch_v4_guest_so46975082.json");
    const LAZY_USER_SERIES: &str =
        include_str!("../../tests/fixtures/niconico/watch_lazy_user_series_sm41984256.json");
    const LAZY_CHANNEL: &str =
        include_str!("../../tests/fixtures/niconico/watch_lazy_channel_so46975082.json");
    const V4_DELETED: &str =
        include_str!("../../tests/fixtures/niconico/watch_v4_error_deleted_sm1.json");
    const V4_NOT_FOUND: &str =
        include_str!("../../tests/fixtures/niconico/watch_v4_error_not_found.json");

    fn build_fixture() -> String {
        let json = serde_json::json!({
            "data": {
                "response": {
                    "video": {
                        "id": "sm9",
                        "title": "テスト",
                        "description": "説明",
                        "duration": 320,
                        "isDeleted": false,
                        "contentType": "video",
                        "registeredAt": "2007-03-06T00:33:00+09:00",
                        "count": {"view": 100, "comment": 50, "mylist": 5},
                        "thumbnail": {"url": "https://example.test/thumb.jpg"}
                    },
                    "owner": {
                        "ownerType": "user",
                        "id": 12345,
                        "nickname": "投稿者",
                        "iconUrl": "https://example.test/icon.jpg"
                    },
                    "series": {
                        "id": 999,
                        "title": "テストシリーズ",
                        "description": "シリーズの説明文",
                        "thumbnailUrl": "https://example.test/series_thumb.jpg",
                        "itemsCount": 5,
                        "isListed": true
                    },
                    "client": {
                        "watchTrackId": "fixtureTrack_1234567890"
                    },
                    "tag": {
                        "items": [
                            {"name": "VOCALOID", "isLocked": true, "isCategory": false},
                            {"name": "初音ミク", "isLocked": false, "isCategory": false}
                        ]
                    },
                    "media": {
                        "domand": {
                            "accessRightKey": "eyJhbGciOiJIUzI1NiJ9.eyJ2IjpbInZpZGVvLWgyNjQtNzIwcCJdLCJhIjpbImF1ZGlvLWFhYy0xMjhrYnBzIl19.sig",
                            "videos": [
                                {"id": "video-h264-360p", "isAvailable": true, "label": "360p",
                                 "bitRate": 600000, "width": 640, "height": 360, "qualityLevel": 1},
                                {"id": "video-h264-720p", "isAvailable": true, "label": "720p",
                                 "bitRate": 2000000, "width": 1280, "height": 720, "qualityLevel": 3}
                            ],
                            "audios": [
                                {"id": "audio-aac-128kbps", "isAvailable": true,
                                 "bitRate": 128000, "qualityLevel": 1}
                            ]
                        }
                    },
                    "comment": {
                        "nvComment": {
                            "server": "https://public.nvcomment.nicovideo.jp",
                            "threadKey": "tk-abcdef",
                            "params": {"language": "ja-jp", "targets": []}
                        }
                    }
                }
            }
        });
        let escaped = serde_json::to_string(&json)
            .unwrap()
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        format!(
            "<!DOCTYPE html><html><head>\n<meta name=\"server-response\" content=\"{escaped}\">\n</head></html>"
        )
    }

    #[test]
    fn parses_watch_html_fixture() {
        let html = build_fixture();
        let data = parse_watch_html(&html).expect("parse");
        assert_eq!(data.video.id, "sm9");
        assert_eq!(data.video.title, "テスト");
        assert_eq!(data.video.duration, 320);
        assert_eq!(data.video.view_count, Some(100));
        assert_eq!(
            data.watch_track_id.as_deref(),
            Some("fixtureTrack_1234567890")
        );

        let domand = data.domand.expect("domand");
        assert!(domand.access_right_key.starts_with("eyJ"));
        assert_eq!(domand.videos.len(), 2);
        assert_eq!(domand.audios.len(), 1);

        let nv = data.nv_comment.expect("nvComment");
        assert_eq!(nv.server, "https://public.nvcomment.nicovideo.jp");
        assert_eq!(nv.thread_key, "tk-abcdef");
    }

    #[test]
    fn parses_video_tags() {
        let html = build_fixture();
        let data = parse_watch_html(&html).expect("parse");
        assert_eq!(data.video.tags.len(), 2);
        assert_eq!(data.video.tags[0].name, "VOCALOID");
        assert!(data.video.tags[0].is_locked);
        assert_eq!(data.video.tags[1].name, "初音ミク");
        assert!(!data.video.tags[1].is_locked);
    }

    #[test]
    fn parses_series_info() {
        let html = build_fixture();
        let data = parse_watch_html(&html).expect("parse");
        let series = data.series.expect("series");
        assert_eq!(series.id, 999);
        assert_eq!(series.title, "テストシリーズ");
        assert_eq!(series.description.as_deref(), Some("シリーズの説明文"));
        assert_eq!(
            series.thumbnail_url.as_deref(),
            Some("https://example.test/series_thumb.jpg")
        );
        assert_eq!(series.items_count, Some(5));
        assert!(series.is_listed);
    }

    #[test]
    fn pick_best_quality_picks_highest() {
        let html = build_fixture();
        let data = parse_watch_html(&html).unwrap();
        let domand = data.domand.unwrap();
        let (vid, aud, label) = pick_best_quality(&domand).unwrap();
        assert_eq!(vid, "video-h264-720p");
        assert_eq!(aud, "audio-aac-128kbps");
        assert_eq!(label.as_deref(), Some("720p"));
    }

    #[test]
    fn pick_best_quality_respects_access_right_key() {
        let html = build_fixture().replace(
            "eyJ2IjpbInZpZGVvLWgyNjQtNzIwcCJdLCJhIjpbImF1ZGlvLWFhYy0xMjhrYnBzIl19",
            "eyJ2IjpbInZpZGVvLWgyNjQtMzYwcCJdLCJhIjpbImF1ZGlvLWFhYy0xMjhrYnBzIl19",
        );
        let data = parse_watch_html(&html).unwrap();
        let domand = data.domand.unwrap();
        let (vid, aud, label) = pick_best_quality(&domand).unwrap();
        assert_eq!(vid, "video-h264-360p");
        assert_eq!(aud, "audio-aac-128kbps");
        assert_eq!(label.as_deref(), Some("360p"));
    }

    #[test]
    fn rejects_html_without_meta() {
        let err = parse_watch_html("<html></html>").unwrap_err();
        assert!(matches!(err, ApiError::ResponseShape(_)));
    }

    #[test]
    fn rejects_invalid_video_id() {
        assert!(!is_valid_video_id(""));
        assert!(!is_valid_video_id("../etc/passwd"));
        assert!(is_valid_video_id("sm9"));
        assert!(is_valid_video_id("so12345"));
    }

    // ---- 2026 年以降の `$watchV4` 形 (実応答) ----

    #[test]
    fn parses_the_current_watch_page_html() {
        // 旧パーサは `/data/response/video` が無くてここで落ちていた。
        let data = parse_watch_html(V4_PAGE_SM9).expect("parse $watchV4 page");
        assert_eq!(data.video.id, "sm9");
        assert_eq!(data.video.duration, 320);
        assert!(data.video.title.contains("陰陽師"));
        assert_eq!(
            data.video.thumbnail_url.as_deref(),
            Some("https://nicovideo.cdn.nimg.jp/thumbnails/9/9")
        );
        assert_eq!(
            data.video.registered_at.as_deref(),
            Some("2007-03-06T00:33:00+09:00")
        );
        assert!(!data.video.is_deleted);
        assert!(data.video.view_count.unwrap_or(0) > 0);
        assert!(data.video.like_count.is_some());
        assert_eq!(data.video.content_type.as_deref(), Some("long"));
        assert!(!data.video.tags.is_empty());
        assert!(data
            .video
            .tags
            .iter()
            .any(|t| t.name == "陰陽師" && t.is_locked));

        // HTML 埋め込みはゲスト (viewer null)。事前発行 HLS は入らない。
        assert!(data.is_guest);
        assert!(data.preissued_hls.is_none());
        assert_eq!(data.lazy_key.as_deref(), Some("redacted-lazy-auth-key"));
        assert_eq!(
            data.watch_track_id.as_deref(),
            Some("Redacted00_1700000000000")
        );
        assert_eq!(data.genre, None, "sm9 はジャンル未設定");
        assert_eq!(data.ok_reason.as_deref(), Some("PURELY"));

        let owner = data.owner.expect("JSON-LD owner");
        assert_eq!(owner.kind, "user");
        assert_eq!(owner.id.as_deref(), Some("4"));
        assert_eq!(owner.nickname.as_deref(), Some("中の"));

        let domand = data.domand.expect("domand");
        assert!(!domand.videos.is_empty());
        assert!(!domand.audios.is_empty());
        let best = pick_best_quality(&domand).expect("a playable pair");
        assert_eq!(best.0, "video-h264-360p");

        let nv = data.nv_comment.expect("nvComment");
        assert_eq!(nv.server, "https://public.nvcomment.nicovideo.jp");
        assert_eq!(nv.params["targets"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn parses_the_v4_watch_api_response() {
        let data = parse_watch_v4_response(V4_API_SERIES.as_bytes()).expect("parse v4");
        assert_eq!(data.video.id, "sm41984256");
        assert!(data.is_guest);
        let hls = data.preissued_hls.as_ref().expect("v4 pre-issues HLS");
        assert!(hls
            .content_url
            .starts_with("https://delivery.domand.nicovideo.jp/"));
        assert!(hls.expire_time.is_some());
        assert_eq!(
            data.genre,
            Some(WatchGenre {
                key: "game".into(),
                label: "ゲーム".into()
            })
        );
        let domand = data.domand.as_ref().expect("domand");
        let candidates = quality_candidates(domand);
        assert_eq!(candidates[0].video_track, "video-h264-720p");
        assert_eq!(candidates[0].label.as_deref(), Some("720p"));
        assert!(
            data.series.is_none(),
            "v4 本体にシリーズは無い (lazy から取る)"
        );
    }

    #[test]
    fn v4_channel_video_owner_is_normalized() {
        let data = parse_watch_v4_response(V4_API_CHANNEL.as_bytes()).expect("parse v4");
        let owner = data.owner.expect("JSON-LD owner");
        assert_eq!(owner.kind, "channel");
        assert_eq!(owner.id.as_deref(), Some("2650183"), "`ch` は外す");
    }

    #[test]
    fn parses_lazy_owner_and_series() {
        let lazy = parse_watch_lazy_response(LAZY_USER_SERIES.as_bytes()).expect("lazy");
        let owner = lazy.owner.expect("owner");
        assert_eq!(owner.kind, "user");
        assert_eq!(owner.id.as_deref(), Some("108373"));
        assert_eq!(owner.nickname.as_deref(), Some("おやつ"));
        assert!(owner.icon_url.unwrap().starts_with("https://"));

        let series = lazy.series.expect("series");
        assert_eq!(series.id, 425178);
        assert_eq!(series.title, "AI実況シリーズ");
        assert!(series.thumbnail_url.is_some());
        assert!(series.prev_video.is_none(), "最初の動画には前が無い");
        assert_eq!(
            series.first_video.as_ref().map(|v| v.id.as_str()),
            Some("sm41984256")
        );
        let next = series.next_video.expect("next");
        assert!(next.id.starts_with("sm"));
        assert!(!next.title.is_empty());
    }

    #[test]
    fn parses_lazy_channel_owner() {
        let lazy = parse_watch_lazy_response(LAZY_CHANNEL.as_bytes()).expect("lazy");
        let owner = lazy.owner.expect("owner");
        assert_eq!(owner.kind, "channel");
        assert_eq!(owner.id.as_deref(), Some("2650183"));
        assert!(owner.nickname.unwrap().contains("チート付与魔術師"));
        assert!(owner.icon_url.unwrap().contains("ch2650183"));
    }

    #[test]
    fn maps_refusals_to_readable_messages() {
        let deleted = watch_http_error("watch v4", 403, V4_DELETED.as_bytes());
        match deleted {
            ApiError::WatchRefused {
                status,
                reason_code,
                message,
                ..
            } => {
                assert_eq!(status, 403);
                assert_eq!(reason_code.as_deref(), Some("ADMINISTRATOR_DELETE_VIDEO"));
                assert!(message.contains("運営"), "{message}");
            }
            other => panic!("expected WatchRefused, got {other:?}"),
        }
        let missing = watch_http_error("watch v4", 404, V4_NOT_FOUND.as_bytes());
        assert!(
            matches!(&missing, ApiError::WatchRefused { message, .. } if message.contains("見つかりません")),
            "{missing:?}"
        );
        // 400 (ヘッダ不足など) は確定した拒否ではないので HTML へ落とせる。
        let bad = watch_http_error(
            "watch v4",
            400,
            br#"{"meta":{"status":400,"errorCode":"INVALID_PARAMETER"}}"#,
        );
        assert!(matches!(bad, ApiError::ServerError { status: 400, .. }));
        let maintenance = watch_http_error(
            "watch v4",
            503,
            br#"{"meta":{"status":503,"errorCode":"MAINTENANCE"}}"#,
        );
        assert!(
            matches!(&maintenance, ApiError::WatchRefused { message, .. } if message.contains("メンテナンス")),
            "{maintenance:?}"
        );
    }

    #[test]
    fn explains_why_media_is_missing() {
        let mut data = parse_watch_v4_response(V4_API_SERIES.as_bytes()).unwrap();
        data.domand = None;
        data.access.is_admission = true;
        assert!(playback_unavailable_message(&data).contains("チャンネル会員限定"));
        data.access.is_admission = false;
        data.access.is_premium_only = true;
        assert!(playback_unavailable_message(&data).contains("プレミアム"));
        data.viewer = Some(WatchViewer { is_premium: true });
        data.access.is_premium_only = true;
        assert!(!playback_unavailable_message(&data).contains("プレミアム会員限定"));
    }

    #[test]
    fn normalizes_owner_ids() {
        assert_eq!(normalize_owner_id("channel", "ch2650183"), "2650183");
        assert_eq!(normalize_owner_id("channel", "2650183"), "2650183");
        assert_eq!(normalize_owner_id("channel", "chiifuyo"), "chiifuyo");
        assert_eq!(normalize_owner_id("user", "ch123"), "ch123");
    }

    #[test]
    fn expiry_check_handles_missing_and_bad_values() {
        assert!(!expires_within(None, 60));
        assert!(!expires_within(Some("not-a-date"), 60));
        assert!(expires_within(Some("2000-01-01T00:00:00+00:00"), 60));
        assert!(!expires_within(Some("2999-01-01T00:00:00+00:00"), 60));
    }

    // ---- HTTP の振る舞い (mockito) ----

    fn v4_body_with_hls(expire: &str) -> String {
        let mut v: Value = serde_json::from_str(V4_API_SERIES).unwrap();
        v["data"]["media"]["hls"]["expiredAt"] = Value::String(expire.into());
        v.to_string()
    }

    fn client_for(server: &mockito::Server, session: Arc<SessionStore>) -> NiconicoWatchClient {
        let base = server.url();
        NiconicoWatchClient::with_endpoints(&format!("{base}/watch/"), &base, session).unwrap()
    }

    #[tokio::test]
    async fn v4_guest_request_sends_as_guest_and_captures_domand_bid() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v4/watch/sm41984256")
            .match_header("x-frontend-id", "6")
            .match_header("x-request-with", "https://www.nicovideo.jp")
            .match_header("cookie", mockito::Matcher::Missing)
            .match_body(mockito::Matcher::PartialJson(
                serde_json::json!({ "asGuest": true }),
            ))
            .with_status(200)
            .with_header("set-cookie", "domand_bid=guest-bid; Path=/; Secure")
            .with_body(V4_API_SERIES)
            .create_async()
            .await;

        let session = Arc::new(SessionStore::empty());
        let client = client_for(&server, Arc::clone(&session));
        let data = client.fetch_watch_v4("sm41984256").await.expect("v4");
        mock.assert_async().await;
        assert!(data.is_guest);
        assert_eq!(session.domand_bid().as_deref(), Some("guest-bid"));
    }

    #[tokio::test]
    async fn v4_logged_in_request_sends_only_the_session_cookie() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v4/watch/sm41984256")
            .match_header("cookie", "user_session=user_session_1_abc")
            .match_body(mockito::Matcher::Regex(
                r#"^\{"actionTrackId":"[A-Za-z0-9]{10}_\d+"\}$"#.into(),
            ))
            .with_status(200)
            .with_body(V4_API_SERIES)
            .create_async()
            .await;

        let session = Arc::new(SessionStore::empty());
        session.set("user_session_1_abc".into());
        session.set_domand_bid("stale".into());
        let client = client_for(&server, Arc::clone(&session));
        client.fetch_watch_v4("sm41984256").await.expect("v4");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn falls_back_to_the_watch_page_when_v4_is_rejected() {
        let mut server = mockito::Server::new_async().await;
        let v4 = server
            .mock("POST", "/v4/watch/sm9")
            .with_status(400)
            .with_body(r#"{"meta":{"status":400,"errorCode":"INVALID_PARAMETER"}}"#)
            .create_async()
            .await;
        let page = server
            .mock("GET", "/watch/sm9")
            .with_status(200)
            .with_body(V4_PAGE_SM9)
            .create_async()
            .await;

        let client = client_for(&server, Arc::new(SessionStore::empty()));
        let data = client.fetch_watch_data("sm9").await.expect("fallback");
        v4.assert_async().await;
        page.assert_async().await;
        assert_eq!(data.video.id, "sm9");
    }

    #[tokio::test]
    async fn refusals_do_not_fall_back() {
        let mut server = mockito::Server::new_async().await;
        let _v4 = server
            .mock("POST", "/v4/watch/sm1")
            .with_status(403)
            .with_body(V4_DELETED)
            .create_async()
            .await;
        let page = server
            .mock("GET", "/watch/sm1")
            .expect(0)
            .create_async()
            .await;

        let client = client_for(&server, Arc::new(SessionStore::empty()));
        let err = client.fetch_watch_data("sm1").await.unwrap_err();
        page.assert_async().await;
        assert!(matches!(err, ApiError::WatchRefused { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn resolve_hls_uses_the_preissued_session_when_the_bid_is_known() {
        let server = mockito::Server::new_async().await;
        let session = Arc::new(SessionStore::empty());
        session.set_domand_bid("bid".into());
        let client = client_for(&server, Arc::clone(&session));
        let data =
            parse_watch_v4_response(v4_body_with_hls("2999-01-01T00:00:00+00:00").as_bytes())
                .unwrap();
        let hls = client.resolve_hls(&data, &[]).await.expect("pre-issued");
        assert_eq!(
            hls.content_url,
            data.preissued_hls.as_ref().unwrap().content_url
        );
    }

    #[tokio::test]
    async fn resolve_hls_issues_rights_without_the_session_for_guest_keys() {
        let mut server = mockito::Server::new_async().await;
        let rights = server
            .mock("POST", "/v1/watch/sm41984256/access-rights/hls")
            .match_query(mockito::Matcher::UrlEncoded(
                "actionTrackId".into(),
                "Redacted00_1700000000000".into(),
            ))
            // ゲストの鍵に user_session を付けると 400 になる。
            .match_header("cookie", mockito::Matcher::Missing)
            .with_status(201)
            .with_header("set-cookie", "domand_bid=fresh; Path=/")
            .with_body(r#"{"meta":{"status":201},"data":{"contentUrl":"https://delivery.domand.nicovideo.jp/x/master.m3u8","createTime":"a","expireTime":"b"}}"#)
            .create_async()
            .await;

        // セッションは設定済みだが、データはゲスト扱い (= 失効したセッション)。
        let session = Arc::new(SessionStore::empty());
        session.set("user_session_expired".into());
        let client = client_for(&server, Arc::clone(&session));
        // 期限切れ間近の事前発行は使わない。
        let data =
            parse_watch_v4_response(v4_body_with_hls("2000-01-01T00:00:00+00:00").as_bytes())
                .unwrap();
        assert!(data.is_guest);
        let outputs = vec![(
            "video-h264-720p".to_string(),
            "audio-aac-192kbps".to_string(),
        )];
        let hls = client.resolve_hls(&data, &outputs).await.expect("rights");
        rights.assert_async().await;
        assert!(hls.content_url.ends_with("master.m3u8"));
        assert_eq!(session.domand_bid().as_deref(), Some("fresh"));
    }

    #[tokio::test]
    async fn lazy_sends_no_side_effect_and_refetches_once_on_key_expiry() {
        let mut server = mockito::Server::new_async().await;
        let expired = server
            .mock("POST", "/v4/watch/lazy/sm41984256")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "keyToken": "stale-key",
                "noSideEffect": true
            })))
            .with_status(400)
            .with_body(r#"{"meta":{"status":400,"errorCode":"KEY_EXPIRED"}}"#)
            .create_async()
            .await;
        let refetch = server
            .mock("POST", "/v4/watch/sm41984256")
            .with_status(200)
            .with_body(V4_API_SERIES)
            .create_async()
            .await;
        let fresh = server
            .mock("POST", "/v4/watch/lazy/sm41984256")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "keyToken": "redacted-lazy-auth-key",
                "noSideEffect": true
            })))
            .with_status(200)
            .with_body(LAZY_USER_SERIES)
            .create_async()
            .await;

        let client = client_for(&server, Arc::new(SessionStore::empty()));
        let mut data = parse_watch_v4_response(V4_API_SERIES.as_bytes()).unwrap();
        data.lazy_key = Some("stale-key".into());
        client.enrich_with_lazy(&mut data).await;
        expired.assert_async().await;
        refetch.assert_async().await;
        fresh.assert_async().await;
        assert_eq!(data.series.as_ref().map(|s| s.id), Some(425178));
        assert_eq!(
            data.owner.as_ref().and_then(|o| o.nickname.as_deref()),
            Some("おやつ")
        );
    }

    #[tokio::test]
    async fn record_view_posts_the_action_track_id_with_the_session() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v4/watch/sm9/side-effect")
            .match_header("cookie", "user_session=user_session_1_abc")
            .match_body(mockito::Matcher::Json(
                serde_json::json!({ "actionTrackId": "Track00000_1" }),
            ))
            .with_status(200)
            .with_body(r#"{"meta":{"status":200}}"#)
            .create_async()
            .await;
        let session = Arc::new(SessionStore::empty());
        session.set("user_session_1_abc".into());
        let client = client_for(&server, session);
        client
            .record_view("sm9", "Track00000_1", false)
            .await
            .expect("side-effect");
        mock.assert_async().await;
    }
}
