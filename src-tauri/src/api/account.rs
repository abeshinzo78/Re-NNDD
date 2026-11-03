//! ログイン中のアカウント (`nvapi /v1/users/me`)。
//!
//! 保存してある `user_session` がまだ有効かを確かめるのに使う。niconico の
//! セッションは期限が切れても多くの API がゲスト扱いで 200 を返すので、
//! 「Cookie を持っている」だけでは本当にログインできているか分からない。
//! `/v1/users/me` は無効なセッションに `401 UNAUTHORIZED` を返す。

use std::time::Duration;

use reqwest::header;
use serde::Serialize;
use serde_json::Value;

use crate::api::auth::SessionStore;
use crate::error::ApiError;

const NVAPI_BASE: &str = "https://nvapi.nicovideo.jp";
const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub id: String,
    pub nickname: String,
    pub is_premium: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// セッションの状態。フロントの設定画面がそのまま表示に使う。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AccountState {
    /// `user_session` を持っていない。
    LoggedOut,
    /// 有効なセッション。
    Valid { account: AccountInfo },
    /// `user_session` はあるが niconico に拒否された (期限切れ・ログアウト済み)。
    Expired,
    /// 通信エラーなどで判定できなかった。
    Unknown { message: String },
}

/// `/v1/users/me` の `data.user` を読む。
pub fn parse_account(body: &[u8]) -> Result<AccountInfo, ApiError> {
    let root: Value = serde_json::from_slice(body)?;
    let user = root
        .pointer("/data/user")
        .filter(|u| u.is_object())
        .ok_or_else(|| ApiError::ResponseShape("users/me has no data.user".into()))?;
    let id = user
        .get("id")
        .and_then(crate::api::video::json_value_as_id_string)
        .ok_or_else(|| ApiError::ResponseShape("users/me has no user.id".into()))?;
    Ok(AccountInfo {
        id,
        nickname: user
            .get("nickname")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        is_premium: user
            .get("isPremium")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        icon_url: ["large", "small"].iter().find_map(|k| {
            user.pointer(&format!("/icons/{k}"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(String::from)
        }),
    })
}

/// 保存中のセッションで `/v1/users/me` を叩いて状態を返す。
pub async fn fetch_account_state(session: &SessionStore) -> AccountState {
    fetch_account_state_at(NVAPI_BASE, session).await
}

pub async fn fetch_account_state_at(base: &str, session: &SessionStore) -> AccountState {
    let Some(cookie) = session.session_cookie() else {
        return AccountState::LoggedOut;
    };
    match request_account(base, &cookie).await {
        Ok(Some(account)) => AccountState::Valid { account },
        Ok(None) => AccountState::Expired,
        Err(e) => AccountState::Unknown {
            message: e.to_string(),
        },
    }
}

/// `Ok(None)` はセッションが拒否された (401/403)。
async fn request_account(base: &str, cookie: &str) -> Result<Option<AccountInfo>, ApiError> {
    let http = reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()?;
    let response = http
        .get(format!("{}/v1/users/me", base.trim_end_matches('/')))
        .header("X-Frontend-Id", "6")
        .header("X-Frontend-Version", "0")
        .header(header::REFERER, "https://www.nicovideo.jp/")
        .header(header::ACCEPT, "application/json")
        .header(header::COOKIE, cookie)
        .send()
        .await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if matches!(status.as_u16(), 401 | 403) {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(ApiError::ServerError {
            status: status.as_u16(),
            message: format!(
                "users/me returned {status}: {}",
                String::from_utf8_lossy(&bytes)
                    .chars()
                    .take(200)
                    .collect::<String>()
            ),
        });
    }
    parse_account(&bytes).map(Some)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const ME: &str = r#"{"meta":{"status":200},"data":{"user":{"id":123,"nickname":"テスト","isPremium":true,"icons":{"small":"https://example.test/s.jpg","large":"https://example.test/l.jpg"}}}}"#;

    #[test]
    fn parses_users_me() {
        let account = parse_account(ME.as_bytes()).unwrap();
        assert_eq!(
            account,
            AccountInfo {
                id: "123".into(),
                nickname: "テスト".into(),
                is_premium: true,
                icon_url: Some("https://example.test/l.jpg".into()),
            }
        );
    }

    #[tokio::test]
    async fn reports_logged_out_valid_and_expired() {
        let empty = SessionStore::empty();
        assert_eq!(
            fetch_account_state_at("http://127.0.0.1:9", &empty).await,
            AccountState::LoggedOut
        );

        let mut server = mockito::Server::new_async().await;
        let ok = server
            .mock("GET", "/v1/users/me")
            .match_header("cookie", "user_session=good")
            .with_status(200)
            .with_body(ME)
            .create_async()
            .await;
        let expired = server
            .mock("GET", "/v1/users/me")
            .match_header("cookie", "user_session=bad")
            .with_status(401)
            .with_body(r#"{"meta":{"status":401,"errorCode":"UNAUTHORIZED"}}"#)
            .create_async()
            .await;

        let good = SessionStore::empty();
        good.set("good".into());
        assert!(matches!(
            fetch_account_state_at(&server.url(), &good).await,
            AccountState::Valid { .. }
        ));
        let bad = SessionStore::empty();
        bad.set("bad".into());
        assert_eq!(
            fetch_account_state_at(&server.url(), &bad).await,
            AccountState::Expired
        );
        ok.assert_async().await;
        expired.assert_async().await;
    }
}
