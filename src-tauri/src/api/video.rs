//! Niconico watch page + HLS access rights client.
//!
//! Two-step flow:
//! 1. Fetch the watch HTML, extract the `<meta name="server-response">`
//!    JSON (HTML-escaped). That payload contains the `accessRightKey` JWT
//!    and the `nvComment` setup.
//! 2. POST to `nvapi.nicovideo.jp/v1/watch/{id}/access-rights/hls` with
//!    the JWT to receive a signed CloudFront HLS URL.
//!
//! Reference shapes from `abeshinzo78/NicoCommentDL`
//! (`src/background/api/niconico.js`).

use async_trait::async_trait;
use rand::Rng;
use regex::Regex;
use reqwest::header;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashSet, sync::OnceLock};

use crate::api::auth::SessionStore;
use crate::error::ApiError;

const WATCH_URL_BASE: &str = "https://www.nicovideo.jp/watch/";
const HLS_API_BASE: &str = "https://nvapi.nicovideo.jp/v1/watch/";
/// Niconico browser frontend signature. Required on /access-rights/hls and
/// nv-comment requests; constant per `niconico.js`.
const FRONTEND_ID: &str = "6";
const FRONTEND_VERSION: &str = "0";
/// Niconico's watch page is mildly UA-sniffy. A modern Chrome string keeps
/// it serving the React app instead of a fallback page.
const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

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

fn html_unescape(s: &str) -> String {
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesInfo {
    pub id: i64,
    pub title: String,
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
    #[serde(default)]
    pub tags: Vec<VideoTag>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
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
    session: std::sync::Arc<SessionStore>,
}

impl NiconicoWatchClient {
    pub fn new(session: std::sync::Arc<SessionStore>) -> Result<Self, ApiError> {
        Self::with_bases(WATCH_URL_BASE, HLS_API_BASE, session)
    }

    pub fn with_bases(
        watch_base: &str,
        hls_base: &str,
        session: std::sync::Arc<SessionStore>,
    ) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .user_agent(BROWSER_UA)
            .gzip(true)
            .build()?;
        Ok(Self {
            http,
            watch_base: watch_base.to_string(),
            hls_base: hls_base.to_string(),
            session,
        })
    }
}

#[async_trait]
impl WatchApi for NiconicoWatchClient {
    async fn fetch_watch_page(&self, video_id: &str) -> Result<WatchPageData, ApiError> {
        if !is_valid_video_id(video_id) {
            return Err(ApiError::InvalidQuery(format!(
                "invalid video id {video_id:?}"
            )));
        }

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
        if let Some(cookie) = self.session.cookie_header() {
            request = request.header(header::COOKIE, cookie);
        }

        let response = request.json(&body).send().await?;
        let status = response.status();

        // Capture `domand_bid` Set-Cookie BEFORE consuming the body —
        // niconico's CloudFront rejects subsequent HLS fragment fetches
        // with 403 unless this cookie is forwarded.
        let mut captured_bid: Option<String> = None;
        for header_value in response.headers().get_all(header::SET_COOKIE) {
            let Ok(raw) = header_value.to_str() else {
                continue;
            };
            if let Some(rest) = raw.strip_prefix("domand_bid=") {
                let value = rest.split(';').next().unwrap_or("");
                if !value.is_empty() {
                    captured_bid = Some(value.to_string());
                }
            }
        }

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
        } else {
            tracing::warn!("access-rights/hls succeeded but no domand_bid cookie present");
        }
        Ok(parsed.data)
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

/// Public for unit-testability — extracts the `server-response` JSON and
/// projects the subset we care about.
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
    project_watch_data(&value)
}

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
        tags: parse_tags(response.pointer("/tag/items")),
    };

    let owner = response.get("owner").and_then(parse_owner);
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
        })
    });

    Ok(WatchPageData {
        video,
        owner,
        domand,
        nv_comment,
        watch_track_id,
        series,
    })
}

fn parse_tags(value: Option<&Value>) -> Vec<VideoTag> {
    value
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|node| {
                    Some(VideoTag {
                        name: node.get("name")?.as_str()?.to_string(),
                        is_locked: node
                            .get("isLocked")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        is_category: node
                            .get("isCategory")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_owner(node: &Value) -> Option<WatchOwner> {
    if node.is_null() {
        return None;
    }
    Some(WatchOwner {
        kind: node
            .get("ownerType")
            .and_then(Value::as_str)
            .unwrap_or("user")
            .to_string(),
        id: node.get("id").and_then(|v| {
            v.as_i64()
                .map(|n| n.to_string())
                .or_else(|| v.as_str().map(String::from))
        }),
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
    value
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|node| {
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
                .collect()
        })
        .unwrap_or_default()
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

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
                        "title": "テストシリーズ"
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
}
