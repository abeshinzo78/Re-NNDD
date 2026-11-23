//! コンテンツツリー (`nvapi GET /v2/videos/{id}/contents-tree`) — 親作品と子作品。
//!
//! 公式 Android アプリ (9.19.0) の dex にある経路 (`/v2/videos/%s/contents-tree`)。
//! 応答 (2026-11 実測): `{ data: { parents: [video…], children: [video…] } }`。
//! 各要素は検索・ランキングと同じ `essential` 形の動画で、子作品は新しい順に最大
//! 15 件 (ページング用のパラメータは効かない)。削除済みの動画は空配列で 200。
//! ログイン不要。

use serde::Serialize;
use serde_json::Value;

use crate::api::nvapi;
use crate::error::ApiError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeVideo {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registered_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    /// センシティブとしてぼかし表示を求められている (`requireSensitiveMasking`)。
    #[serde(default)]
    pub require_sensitive_masking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentsTree {
    pub parents: Vec<TreeVideo>,
    pub children: Vec<TreeVideo>,
}

fn non_empty(node: &Value, pointer: &str) -> Option<String> {
    node.pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn parse_video(node: &Value) -> Option<TreeVideo> {
    let id = non_empty(node, "/id")?;
    Some(TreeVideo {
        title: non_empty(node, "/title").unwrap_or_else(|| id.clone()),
        // 署名付きで失効しうる nHdUrl などより、素の URL を優先する。
        thumbnail_url: [
            "/thumbnail/listingUrl",
            "/thumbnail/url",
            "/thumbnail/middleUrl",
        ]
        .iter()
        .find_map(|p| non_empty(node, p)),
        duration: node.get("duration").and_then(Value::as_i64),
        registered_at: non_empty(node, "/registeredAt"),
        view_count: node.pointer("/count/view").and_then(Value::as_i64),
        comment_count: node.pointer("/count/comment").and_then(Value::as_i64),
        owner_name: non_empty(node, "/owner/name"),
        require_sensitive_masking: node
            .get("requireSensitiveMasking")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        id,
    })
}

fn parse_list(data: &Value, key: &str) -> Vec<TreeVideo> {
    data.get(key)
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_video).collect())
        .unwrap_or_default()
}

/// `GET /v2/videos/{id}/contents-tree` の本文を読む。
pub fn parse_contents_tree(body: &[u8]) -> Result<ContentsTree, ApiError> {
    let data = nvapi::parse_data("contents-tree", body)?;
    Ok(ContentsTree {
        parents: parse_list(&data, "parents"),
        children: parse_list(&data, "children"),
    })
}

pub struct ContentsTreeClient {
    http: reqwest::Client,
    nvapi_base: String,
    cookie: Option<String>,
}

impl ContentsTreeClient {
    /// `cookie` はログイン中なら `user_session=…` (センシティブ表示の設定が効く)。
    pub fn new(cookie: Option<String>) -> Result<Self, ApiError> {
        Self::with_base(nvapi::NVAPI_BASE, cookie)
    }

    pub fn with_base(nvapi_base: &str, cookie: Option<String>) -> Result<Self, ApiError> {
        Ok(Self {
            http: nvapi::build_http()?,
            nvapi_base: nvapi::trim_base(nvapi_base),
            cookie,
        })
    }

    pub async fn fetch(&self, video_id: &str) -> Result<ContentsTree, ApiError> {
        crate::api::video::ensure_valid_video_id(video_id)?;
        let url = format!(
            "{}/v2/videos/{video_id}/contents-tree?sensitiveContents=mask",
            self.nvapi_base
        );
        let mut request = nvapi::with_frontend_headers(self.http.get(&url));
        if let Some(cookie) = &self.cookie {
            request = request.header(reqwest::header::COOKIE, cookie);
        }
        let response = request.send().await?;
        let status = response.status();
        let body = response.bytes().await?;
        if !status.is_success() {
            return Err(nvapi::status_error("contents-tree", status, &body));
        }
        parse_contents_tree(&body)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // 2026-11 の実応答 (sm32302688) を 1 件に縮めたもの。
    const TREE: &str = r#"{"meta":{"status":200},"data":{"parents":[{"type":"essential","id":"sm32300945","contentType":"long","title":"PS4版DQ11RTA 12:35:00 part1","registeredAt":"2017-11-21T06:48:11+09:00","count":{"view":4320,"comment":8,"mylist":20,"like":1},"thumbnail":{"url":"https://nicovideo.cdn.nimg.jp/thumbnails/32300945/32300945","middleUrl":"https://nicovideo.cdn.nimg.jp/thumbnails/32300945/32300945.M","listingUrl":"https://nicovideo.cdn.nimg.jp/thumbnails/32300945/32300945.M","nHdUrl":"https://img.cdn.nimg.jp/s/nicovideo/thumbnails/32300945/32300945.original/r640x360l?key=x"},"duration":1843,"owner":{"ownerType":"user","type":"user","id":"26655449","name":"dice"},"requireSensitiveMasking":false},{"title":"id の無い要素は捨てる"}],"children":[]}}"#;

    #[test]
    fn parses_parents_and_children() {
        let tree = parse_contents_tree(TREE.as_bytes()).unwrap();
        assert!(tree.children.is_empty());
        assert_eq!(tree.parents.len(), 1);
        let p = &tree.parents[0];
        assert_eq!(p.id, "sm32300945");
        assert_eq!(p.title, "PS4版DQ11RTA 12:35:00 part1");
        assert_eq!(
            p.thumbnail_url.as_deref(),
            Some("https://nicovideo.cdn.nimg.jp/thumbnails/32300945/32300945.M")
        );
        assert_eq!(p.duration, Some(1843));
        assert_eq!(p.view_count, Some(4320));
        assert_eq!(p.owner_name.as_deref(), Some("dice"));
        assert!(!p.require_sensitive_masking);

        let deleted =
            parse_contents_tree(br#"{"meta":{"status":200},"data":{"parents":[],"children":[]}}"#)
                .unwrap();
        assert!(deleted.parents.is_empty() && deleted.children.is_empty());
    }

    #[tokio::test]
    async fn fetches_with_masking_and_frontend_headers() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v2/videos/sm32302688/contents-tree")
            .match_query(mockito::Matcher::UrlEncoded(
                "sensitiveContents".into(),
                "mask".into(),
            ))
            .match_header("x-frontend-id", "6")
            .match_header("cookie", mockito::Matcher::Missing)
            .with_status(200)
            .with_body(TREE)
            .create_async()
            .await;
        let client = ContentsTreeClient::with_base(&server.url(), None).unwrap();
        let tree = client.fetch("sm32302688").await.expect("tree");
        mock.assert_async().await;
        assert_eq!(tree.parents.len(), 1);
        assert!(client.fetch("../etc").await.is_err());
    }
}
