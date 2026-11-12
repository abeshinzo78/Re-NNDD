//! ニコニコ側の視聴者 NG 設定 (`nvapi GET /v1/users/me/ng-comments/client`)。
//!
//! 公式 Android アプリ (9.19.0) の dex にある経路 (`/v1/users/me/ng-comments/client`
//! と `…/items`) で、Web も同じものを使う。ここは取り込み (読むだけ) の用途で、
//! ニコニコ側へは書き込まない。
//!
//! 応答 (2026-11 実測): `{ data: { revision, count, items: [{ type, source, registeredAt }] } }`。
//! `type` ごとの判定は Web プレイヤーの実装に合わせて読む:
//! - `word`: コメント本文 (改行を除く) の部分一致。大文字小文字を区別しない
//! - `id`: コメントの `userId` と一致
//! - `command`: `source` を空白で区切った全トークンがコメントのコマンドに含まれる
//!   (大文字小文字を区別しない)。`184` だけはコマンドが `184` だけのコメントに当たる
//!
//! 視聴データ (`comment.ng.viewer`) にも同じ一覧が入っている。

use std::sync::Arc;

use reqwest::{header, StatusCode};
use serde::Serialize;
use serde_json::Value;

use crate::api::auth::SessionStore;
use crate::api::nvapi;
use crate::error::ApiError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerNgItem {
    /// `word` / `id` / `command`。知らない種別もそのまま渡す (フロントが数える)。
    #[serde(rename = "type")]
    pub kind: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registered_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerNgList {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<i64>,
    pub items: Vec<ViewerNgItem>,
}

/// `GET /v1/users/me/ng-comments/client` の本文を読む。
pub fn parse_viewer_ng(body: &[u8]) -> Result<ViewerNgList, ApiError> {
    let data = nvapi::parse_data("ng-comments/client", body)?;
    // `items` が無いのを「0 件」と取り違えない (取り込みで何も起きないだけだが、
    // 形が変わったことには気付けるようにする)。
    let Some(raw_items) = data.get("items").and_then(Value::as_array) else {
        return Err(ApiError::ResponseShape(
            "ng-comments/client response has no items".into(),
        ));
    };
    let items = raw_items
        .iter()
        .filter_map(|node| {
            let kind = node.get("type").and_then(Value::as_str)?.trim().to_string();
            let source = node.get("source").and_then(Value::as_str)?.to_string();
            if kind.is_empty() || source.trim().is_empty() {
                return None;
            }
            Some(ViewerNgItem {
                kind,
                source,
                registered_at: node
                    .get("registeredAt")
                    .and_then(Value::as_str)
                    .map(String::from),
            })
        })
        .collect();
    Ok(ViewerNgList {
        revision: data.get("revision").and_then(Value::as_i64),
        items,
    })
}

pub struct ViewerNgClient {
    http: reqwest::Client,
    nvapi_base: String,
    session: Arc<SessionStore>,
}

impl ViewerNgClient {
    pub fn new(session: Arc<SessionStore>) -> Result<Self, ApiError> {
        Self::with_base(nvapi::NVAPI_BASE, session)
    }

    /// テスト用に nvapi の base を差し替える。
    pub fn with_base(nvapi_base: &str, session: Arc<SessionStore>) -> Result<Self, ApiError> {
        Ok(Self {
            http: nvapi::build_http()?,
            nvapi_base: nvapi::trim_base(nvapi_base),
            session,
        })
    }

    /// ログイン中のアカウントの視聴者 NG 設定を取る。
    pub async fn fetch(&self) -> Result<ViewerNgList, ApiError> {
        let Some(cookie) = self.session.session_cookie() else {
            return Err(ApiError::InvalidQuery(
                "ニコニコにログインしていないため NG 設定を取得できません".into(),
            ));
        };
        let url = format!("{}/v1/users/me/ng-comments/client", self.nvapi_base);
        let response = nvapi::with_frontend_headers(self.http.get(&url))
            .header(header::COOKIE, cookie)
            .send()
            .await?;
        let status = response.status();
        let body = response.bytes().await?;
        if status == StatusCode::UNAUTHORIZED {
            return Err(ApiError::ServerError {
                status: 401,
                message: "ニコニコのログインが切れているため NG 設定を取得できません".into(),
            });
        }
        if !status.is_success() {
            return Err(nvapi::status_error("ng-comments/client", status, &body));
        }
        parse_viewer_ng(&body)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const LIST: &str = r#"{"meta":{"status":200},"data":{"revision":25,"count":4,"items":[
        {"type":"word","source":"ネタバレ","registeredAt":"2026-11-12T20:00:00+09:00"},
        {"type":"id","source":"nvc:AbCdEf"},
        {"type":"command","source":"big red"},
        {"type":"word","source":"  "}
    ]}}"#;

    #[test]
    fn parses_the_viewer_ng_list() {
        let list = parse_viewer_ng(LIST.as_bytes()).unwrap();
        assert_eq!(list.revision, Some(25));
        // 空のパターンは捨てる
        assert_eq!(list.items.len(), 3);
        assert_eq!(list.items[0].kind, "word");
        assert_eq!(list.items[0].source, "ネタバレ");
        assert_eq!(
            list.items[0].registered_at.as_deref(),
            Some("2026-11-12T20:00:00+09:00")
        );
        assert_eq!(list.items[1].kind, "id");
        assert_eq!(list.items[2].kind, "command");

        let empty = parse_viewer_ng(
            br#"{"meta":{"status":200},"data":{"revision":24,"count":0,"items":[]}}"#,
        )
        .unwrap();
        assert!(empty.items.is_empty());
        assert!(parse_viewer_ng(br#"{"meta":{"status":200},"data":{"revision":1}}"#).is_err());
    }

    #[tokio::test]
    async fn fetches_with_the_session_only() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/users/me/ng-comments/client")
            .match_header("x-frontend-id", "6")
            .match_header("cookie", "user_session=user_session_1_abc")
            .with_status(200)
            .with_body(LIST)
            .create_async()
            .await;
        let session = SessionStore::empty();
        session.set("user_session_1_abc".into());
        session.set_domand_bid("bid".into());
        let client = ViewerNgClient::with_base(&server.url(), Arc::new(session)).unwrap();
        let list = client.fetch().await.expect("list");
        mock.assert_async().await;
        assert_eq!(list.items.len(), 3);
    }

    #[tokio::test]
    async fn needs_a_valid_login() {
        let mut server = mockito::Server::new_async().await;
        let guest =
            ViewerNgClient::with_base(&server.url(), Arc::new(SessionStore::empty())).unwrap();
        assert!(guest.fetch().await.is_err());

        let _m = server
            .mock("GET", "/v1/users/me/ng-comments/client")
            .with_status(401)
            .with_body(r#"{"meta":{"status":401,"errorCode":"UNAUTHORIZED"}}"#)
            .create_async()
            .await;
        let session = SessionStore::empty();
        session.set("expired".into());
        let client = ViewerNgClient::with_base(&server.url(), Arc::new(session)).unwrap();
        let err = client.fetch().await.unwrap_err();
        assert!(err.to_string().contains("ログイン"), "{err}");
    }
}
