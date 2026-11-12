//! 検索キーワードのサジェスト (`https://sug.search.nicovideo.jp/suggestion/expand/{q}`)。
//!
//! 公式 Android アプリ (9.19.0) のホスト表 (`res/values/strings.xml` の
//! `server_sugoi_api_url`) にあるホストで、Web の検索欄も同じものを使う。
//! 応答は `{ "candidates": ["…", …] }` (最大 10 件、ログイン不要 — 2026-11 実測)。
//! CORS ヘッダーを返さないので WebView からは直接取れず、Rust 側で取る。

use std::time::Duration;

use reqwest::header;
use serde_json::Value;
use url::Url;

use crate::api::nvapi;
use crate::error::ApiError;

pub const SUGGEST_BASE: &str = "https://sug.search.nicovideo.jp";
/// これより長い入力は候補が出ないので送らない。
pub const MAX_QUERY_CHARS: usize = 64;
const MAX_CANDIDATES: usize = 20;

pub struct SuggestClient {
    http: reqwest::Client,
    base: Url,
}

impl SuggestClient {
    pub fn new() -> Result<Self, ApiError> {
        Self::with_base(SUGGEST_BASE)
    }

    /// テスト用にホストを差し替える。
    pub fn with_base(base: &str) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .user_agent(nvapi::BROWSER_UA)
            .gzip(true)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self {
            http,
            base: Url::parse(base)?,
        })
    }

    fn build_url(&self, query: &str) -> Result<Url, ApiError> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|()| ApiError::InvalidQuery("suggest base cannot have a path".into()))?
            .pop_if_empty()
            .extend(["suggestion", "expand", query]);
        Ok(url)
    }

    /// 入力中のキーワードに対する候補。空白だけ・長すぎる入力は問い合わせずに空を返す。
    pub async fn suggest(&self, query: &str) -> Result<Vec<String>, ApiError> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS {
            return Ok(Vec::new());
        }
        let url = self.build_url(query)?;
        let response = self
            .http
            .get(url)
            .header(header::ACCEPT, "application/json")
            .header(header::REFERER, format!("{}/", nvapi::NICOVIDEO_ORIGIN))
            .send()
            .await?;
        let status = response.status();
        let body = response.bytes().await?;
        if !status.is_success() {
            return Err(nvapi::status_error("search suggestion", status, &body));
        }
        parse_candidates(&body)
    }
}

/// `{ "candidates": [...] }` を読む。重複と空文字は落とす。
pub fn parse_candidates(body: &[u8]) -> Result<Vec<String>, ApiError> {
    let root: Value = serde_json::from_slice(body)
        .map_err(|e| ApiError::ResponseShape(format!("suggestion JSON parse failed: {e}")))?;
    let Some(arr) = root.get("candidates").and_then(Value::as_array) else {
        return Err(ApiError::ResponseShape(
            "suggestion response has no candidates".into(),
        ));
    };
    let mut out: Vec<String> = Vec::with_capacity(arr.len().min(MAX_CANDIDATES));
    for candidate in arr.iter().filter_map(Value::as_str) {
        let candidate = candidate.trim();
        if candidate.is_empty() || out.iter().any(|c| c == candidate) {
            continue;
        }
        out.push(candidate.to_string());
        if out.len() >= MAX_CANDIDATES {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_candidates() {
        let body = r#"{"candidates":["初音ミク","初音ミクの消失","初音ミク"," ",3]}"#;
        assert_eq!(
            parse_candidates(body.as_bytes()).unwrap(),
            vec!["初音ミク".to_string(), "初音ミクの消失".to_string()]
        );
        assert!(parse_candidates(br#"{"other":[]}"#).is_err());
    }

    #[test]
    fn encodes_the_query_as_one_path_segment() {
        let client = SuggestClient::with_base("https://sug.search.nicovideo.jp").unwrap();
        let url = client.build_url("初音 ミク/AC").unwrap();
        assert_eq!(
            url.as_str(),
            "https://sug.search.nicovideo.jp/suggestion/expand/%E5%88%9D%E9%9F%B3%20%E3%83%9F%E3%82%AF%2FAC"
        );
    }

    #[tokio::test]
    async fn fetches_candidates_and_skips_empty_input() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/suggestion/expand/%E5%88%9D%E9%9F%B3")
            .with_status(200)
            .with_body(r#"{"candidates":["初音ミク","初音ミクの消失"]}"#)
            .create_async()
            .await;
        let client = SuggestClient::with_base(&server.url()).unwrap();
        assert_eq!(
            client.suggest(" 初音 ").await.unwrap(),
            vec!["初音ミク".to_string(), "初音ミクの消失".to_string()]
        );
        mock.assert_async().await;

        // 問い合わせずに空を返す (mock が無いので呼べば失敗する)。
        assert!(client.suggest("   ").await.unwrap().is_empty());
        let long: String = "あ".repeat(MAX_QUERY_CHARS + 1);
        assert!(client.suggest(&long).await.unwrap().is_empty());
    }
}
