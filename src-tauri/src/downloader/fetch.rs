//! Domand HTTP クライアント。
//!
//! niconico の Domand CDN は UA / Referer / sec-fetch-* を見て 403 を返すので、
//! ブラウザ風のヘッダを揃えて投げる必要がある。`commands::fetch_hls_resource`
//! と同じヘッダを使うが、こちらは Tauri command ではなく純粋な Rust 関数。

use std::sync::Arc;
use std::time::Duration;

use reqwest::header;

use crate::api::auth::SessionStore;
use crate::error::ApiError;

const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

#[derive(Clone)]
pub struct DomandClient {
    http: reqwest::Client,
    session: Arc<SessionStore>,
}

impl DomandClient {
    pub fn new(session: Arc<SessionStore>) -> Result<Self, ApiError> {
        // gzip は ON にしない: CloudFront が稀に Content-Encoding: gzip を
        // 非 gzip ボディに付けてくる事故対策（既存 fetch_hls_resource と同じ）。
        let http = reqwest::Client::builder()
            .user_agent(BROWSER_UA)
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { http, session })
    }

    /// 任意 URL を GET。CDN ホスト制限はかけない（呼び出し側が責任を持つ）。
    pub async fn get_bytes(&self, url: &str) -> Result<Vec<u8>, ApiError> {
        self.get_range(url, None, None).await
    }

    pub async fn get_text(&self, url: &str) -> Result<String, ApiError> {
        let bytes = self.get_bytes(url).await?;
        String::from_utf8(bytes)
            .map_err(|e| ApiError::ResponseShape(format!("non-UTF8 playlist body: {e}")))
    }

    /// HTTP Range 付き GET。`end` は **inclusive** （RFC 7233）。
    pub async fn get_range(
        &self,
        url: &str,
        start: Option<u64>,
        end: Option<u64>,
    ) -> Result<Vec<u8>, ApiError> {
        let mut request = self
            .http
            .get(url)
            .header(header::REFERER, "https://www.nicovideo.jp/")
            .header(header::ACCEPT, "*/*")
            .header(header::ACCEPT_LANGUAGE, "ja,en-US;q=0.9,en;q=0.8")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-site", "same-site")
            .header("sec-fetch-dest", "empty")
            .header(
                "sec-ch-ua",
                "\"Chromium\";v=\"130\", \"Not?A_Brand\";v=\"99\"",
            )
            .header("sec-ch-ua-mobile", "?0")
            .header("sec-ch-ua-platform", "\"Linux\"");
        if let Some(cookie) = self.session.cookie_header() {
            request = request.header(header::COOKIE, cookie);
        }
        let range_header = match (start, end) {
            (Some(s), Some(e)) if e >= s => Some(format!("bytes={s}-{e}")),
            (Some(s), None) => Some(format!("bytes={s}-")),
            _ => None,
        };
        if let Some(r) = range_header.as_ref() {
            request = request.header(header::RANGE, r);
        }

        let response = request.send().await?;
        let status = response.status();
        let bytes = response.bytes().await?.to_vec();
        if !status.is_success() {
            let preview = String::from_utf8_lossy(&bytes)
                .chars()
                .take(200)
                .collect::<String>();
            return Err(ApiError::ServerError {
                status: status.as_u16(),
                message: format!("Domand fetch {url} failed ({status}): {preview}"),
            });
        }
        Ok(bytes)
    }
}
