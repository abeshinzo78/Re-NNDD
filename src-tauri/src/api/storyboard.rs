//! ストーリーボード (シークバーのシーンサムネイル)。
//!
//! 公式 Android アプリ (9.19.0) のネイティブ実装が使う
//! `POST nvapi /v1/watch/{id}/access-rights/storyboard` (APK の dex 文字列
//! `/v1/watch/%s/access-rights/storyboard`、UI 文字列「シーンのサムネイル」) と
//! 同じものを Web プレイヤーも使っている。発行は
//! [`crate::api::video::NiconicoWatchClient::request_storyboard`]、ここでは応答と
//! `storyboard.json` の読み方だけを持つ。
//!
//! `contentUrl` は `storyboard.json` の署名付き URL で、中身はこう (2026-11 実測):
//!
//! ```json
//! { "version": "1", "thumbnailWidth": 120, "thumbnailHeight": 90,
//!   "columns": 10, "rows": 10, "interval": 2000,
//!   "images": [{ "timestamp": 0, "url": "0001.jpg" },
//!              { "timestamp": 200000, "url": "0002.jpg" }] }
//! ```
//!
//! 画像は `columns × rows` 枚のコマを `interval` ミリ秒おきに並べたスプライトで、
//! `url` は JSON からの相対パス。署名 (`session` と CloudFront の `Policy` /
//! `Signature` / `Key-Pair-Id`) は同じディレクトリ全体 (`…/storyboard/*`) に効くので、
//! JSON と同じクエリを付ければ取れる。画像は Cookie も Referer も要らない (実測) ので
//! WebView の `<img>` / CSS 背景に直接渡せる。JSON は CORS ヘッダーが無いので
//! Rust 側で取る。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::error::ApiError;

/// ストーリーボードを配る CDN。これ以外のホストの URL は受け取らない。
pub const STORYBOARD_HOST: &str = "asset.domand.nicovideo.jp";

/// 1 コマの寸法・並びの上限。実測は 120×90 / 10×10。壊れた値で巨大な
/// 背景位置を計算しないよう、常識的な範囲に限る。
const MAX_THUMB_SIDE: u64 = 4096;
const MAX_GRID: u64 = 100;

/// `access-rights/storyboard` の `data`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryboardAccess {
    pub content_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expire_time: Option<String>,
}

/// フロントへ渡すストーリーボード。画像 URL は署名付きの絶対 URL。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Storyboard {
    pub thumbnail_width: u32,
    pub thumbnail_height: u32,
    pub columns: u32,
    pub rows: u32,
    /// コマの間隔 (ミリ秒)。
    pub interval_ms: u64,
    /// 開始時刻の昇順。
    pub images: Vec<StoryboardImage>,
    /// 署名の期限 (RFC 3339)。実測で発行から 3 時間。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expire_time: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryboardImage {
    /// このスプライトの先頭コマの時刻 (ミリ秒)。
    pub timestamp_ms: u64,
    pub url: String,
}

/// ストーリーボードの URL が https かつ [`STORYBOARD_HOST`] か確かめる。
pub fn validate_asset_url(raw: &str) -> Result<Url, ApiError> {
    let url = Url::parse(raw)?;
    if url.scheme() != "https" || url.host_str() != Some(STORYBOARD_HOST) {
        return Err(ApiError::ResponseShape(format!(
            "storyboard URL is not on {STORYBOARD_HOST}: {}",
            url.host_str().unwrap_or("")
        )));
    }
    Ok(url)
}

/// `access-rights/storyboard` の応答本文 (`{ meta, data }`) を読む。
pub fn parse_access_response(body: &[u8]) -> Result<StoryboardAccess, ApiError> {
    let root: Value = serde_json::from_slice(body).map_err(|e| {
        ApiError::ResponseShape(format!("storyboard access-rights JSON parse failed: {e}"))
    })?;
    let data = root.get("data").cloned().ok_or_else(|| {
        ApiError::ResponseShape("storyboard access-rights response has no data".into())
    })?;
    let access: StoryboardAccess = serde_json::from_value(data).map_err(|e| {
        ApiError::ResponseShape(format!("storyboard access-rights data is malformed: {e}"))
    })?;
    validate_asset_url(&access.content_url)?;
    Ok(access)
}

fn positive_u64(node: &Value, key: &str, max: u64) -> Result<u64, ApiError> {
    node.get(key)
        .and_then(Value::as_u64)
        .filter(|v| (1..=max).contains(v))
        .ok_or_else(|| ApiError::ResponseShape(format!("storyboard.json has no valid {key}")))
}

/// 画像の相対パスを `storyboard.json` の URL から解決し、署名のクエリを付ける。
/// 別ホストへ向いたものは捨てる。
fn resolve_image_url(base: &Url, relative: &str) -> Option<String> {
    let mut url = base.join(relative).ok()?;
    if url.scheme() != "https" || url.host_str() != Some(STORYBOARD_HOST) {
        return None;
    }
    if url.query().is_none() {
        url.set_query(base.query());
    }
    Some(url.into())
}

/// `storyboard.json` を読む。`content_url` は取得元の URL (相対パスの解決に使う)。
pub fn parse_manifest(
    body: &[u8],
    content_url: &str,
    expire_time: Option<String>,
) -> Result<Storyboard, ApiError> {
    let base = validate_asset_url(content_url)?;
    let root: Value = serde_json::from_slice(body)
        .map_err(|e| ApiError::ResponseShape(format!("storyboard.json parse failed: {e}")))?;

    let thumbnail_width = positive_u64(&root, "thumbnailWidth", MAX_THUMB_SIDE)?;
    let thumbnail_height = positive_u64(&root, "thumbnailHeight", MAX_THUMB_SIDE)?;
    let columns = positive_u64(&root, "columns", MAX_GRID)?;
    let rows = positive_u64(&root, "rows", MAX_GRID)?;
    let interval_ms = positive_u64(&root, "interval", u64::from(u32::MAX))?;

    let mut images: Vec<StoryboardImage> = root
        .get("images")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|node| {
                    let timestamp_ms = node.get("timestamp").and_then(Value::as_u64)?;
                    let relative = node.get("url").and_then(Value::as_str)?;
                    Some(StoryboardImage {
                        timestamp_ms,
                        url: resolve_image_url(&base, relative)?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if images.is_empty() {
        return Err(ApiError::ResponseShape(
            "storyboard.json lists no usable images".into(),
        ));
    }
    images.sort_by_key(|img| img.timestamp_ms);

    // 上の `positive_u64` で u32 に収まる範囲に限っているので切り捨ては起きない。
    let to_u32 = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
    Ok(Storyboard {
        thumbnail_width: to_u32(thumbnail_width),
        thumbnail_height: to_u32(thumbnail_height),
        columns: to_u32(columns),
        rows: to_u32(rows),
        interval_ms,
        images,
        expire_time,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // 署名部分は無害な値に置き換えてある。
    const CONTENT_URL: &str = "https://asset.domand.nicovideo.jp/abc123/storyboard/1/storyboard/storyboard.json?session=s1&Policy=p1&Signature=sig~1_&Key-Pair-Id=K1";
    const MANIFEST: &str = r#"{"version":"1","thumbnailWidth":120,"thumbnailHeight":90,"columns":10,"rows":10,"interval":2000,"images":[{"timestamp":200000,"url":"0002.jpg"},{"timestamp":0,"url":"0001.jpg"}]}"#;

    #[test]
    fn parses_the_manifest_and_signs_image_urls() {
        let sb = parse_manifest(
            MANIFEST.as_bytes(),
            CONTENT_URL,
            Some("2026-11-12T23:25:36+09:00".into()),
        )
        .unwrap();
        assert_eq!((sb.thumbnail_width, sb.thumbnail_height), (120, 90));
        assert_eq!((sb.columns, sb.rows), (10, 10));
        assert_eq!(sb.interval_ms, 2000);
        assert_eq!(sb.expire_time.as_deref(), Some("2026-11-12T23:25:36+09:00"));
        // 時刻順に並べ直し、JSON と同じ署名クエリを付ける。
        assert_eq!(sb.images.len(), 2);
        assert_eq!(sb.images[0].timestamp_ms, 0);
        assert_eq!(
            sb.images[0].url,
            "https://asset.domand.nicovideo.jp/abc123/storyboard/1/storyboard/0001.jpg?session=s1&Policy=p1&Signature=sig~1_&Key-Pair-Id=K1"
        );
        assert!(sb.images[1]
            .url
            .contains("/storyboard/0002.jpg?session=s1&"));
    }

    #[test]
    fn drops_images_that_point_elsewhere() {
        let manifest = r#"{"thumbnailWidth":120,"thumbnailHeight":90,"columns":10,"rows":10,"interval":2000,"images":[{"timestamp":0,"url":"https://evil.example/x.jpg"},{"timestamp":200000,"url":"0002.jpg"}]}"#;
        let sb = parse_manifest(manifest.as_bytes(), CONTENT_URL, None).unwrap();
        assert_eq!(sb.images.len(), 1);
        assert_eq!(sb.images[0].timestamp_ms, 200000);
    }

    #[test]
    fn rejects_broken_manifests() {
        for bad in [
            r#"{"thumbnailWidth":0,"thumbnailHeight":90,"columns":10,"rows":10,"interval":2000,"images":[{"timestamp":0,"url":"0001.jpg"}]}"#,
            r#"{"thumbnailWidth":120,"thumbnailHeight":90,"columns":10,"rows":10,"interval":2000,"images":[]}"#,
            r#"{"thumbnailWidth":120,"thumbnailHeight":90,"columns":1000,"rows":10,"interval":2000,"images":[{"timestamp":0,"url":"0001.jpg"}]}"#,
            "not json",
        ] {
            assert!(
                parse_manifest(bad.as_bytes(), CONTENT_URL, None).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn only_accepts_the_domand_asset_host() {
        assert!(validate_asset_url(CONTENT_URL).is_ok());
        assert!(validate_asset_url("http://asset.domand.nicovideo.jp/x").is_err());
        assert!(validate_asset_url("https://example.com/storyboard.json").is_err());
        assert!(parse_manifest(MANIFEST.as_bytes(), "https://example.com/s.json", None).is_err());
    }

    #[test]
    fn parses_the_access_rights_response() {
        let body = format!(
            r#"{{"meta":{{"status":201}},"data":{{"contentUrl":"{CONTENT_URL}","createTime":"2026-11-12T20:25:36+09:00","expireTime":"2026-11-12T23:25:36+09:00"}}}}"#
        );
        let access = parse_access_response(body.as_bytes()).unwrap();
        assert_eq!(access.content_url, CONTENT_URL);
        assert_eq!(
            access.expire_time.as_deref(),
            Some("2026-11-12T23:25:36+09:00")
        );
        assert!(parse_access_response(br#"{"meta":{"status":201}}"#).is_err());
        assert!(parse_access_response(
            br#"{"meta":{"status":201},"data":{"contentUrl":"https://example.com/x.json"}}"#
        )
        .is_err());
    }
}
