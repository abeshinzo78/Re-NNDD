//! ニコニコ側の視聴履歴 (`nvapi /v2/users/me/watch/history/*`)。
//!
//! 今は再生位置の保存だけを持つ:
//! `PUT /v2/users/me/watch/history/playback-position` (JSON `{ videoId, seconds }`)。
//! 公式 Android アプリ (9.19.0) の dex にある経路 (同アプリの「レジューム再生
//! (続きから再生)」) で、Web プレイヤーもログイン中かつ `okReason == PURELY` の時に
//! 一時停止・ページ離脱で送っている (見終わった時は 0)。
//!
//! 実測 (2026-11):
//! - 本文のキーは `videoId` (`watchId` だと 400 INVALID_PARAMETER)。
//! - `X-Request-With` が無いと 400、ゲスト (Cookie なし) は 401。
//! - 保存した位置は視聴データの `player.initialPlayback.positionSec` に返るが、
//!   プレミアム会員以外は null (ニコニコ側の続きから再生はプレミアム機能)。

use std::sync::Arc;

use reqwest::header;

use crate::api::auth::SessionStore;
use crate::api::nvapi;
use crate::error::ApiError;

/// 位置の上限。これより長い動画は無いので、壊れた値は送らない。
const MAX_SECONDS: f64 = 60.0 * 60.0 * 24.0 * 7.0;

pub struct WatchHistoryClient {
    http: reqwest::Client,
    nvapi_base: String,
    session: Arc<SessionStore>,
}

impl WatchHistoryClient {
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

    /// 再生位置 (秒) をニコニコの視聴履歴へ保存する。ログインしていなければ送らない。
    pub async fn save_playback_position(
        &self,
        video_id: &str,
        seconds: f64,
    ) -> Result<(), ApiError> {
        crate::api::video::ensure_valid_video_id(video_id)?;
        if !seconds.is_finite() || !(0.0..=MAX_SECONDS).contains(&seconds) {
            return Err(ApiError::InvalidQuery(format!(
                "playback position out of range: {seconds}"
            )));
        }
        let Some(cookie) = self.session.session_cookie() else {
            return Err(ApiError::InvalidQuery(
                "ログインしていないため再生位置をニコニコに保存できません".into(),
            ));
        };
        let url = format!(
            "{}/v2/users/me/watch/history/playback-position",
            self.nvapi_base
        );
        // 小数 3 桁に丸める (Web プレイヤーは currentTime をそのまま送る)。
        let seconds = (seconds * 1000.0).round() / 1000.0;
        let response = nvapi::with_frontend_headers(self.http.put(&url))
            .header(header::CONTENT_TYPE, "application/json")
            .header("X-Request-With", nvapi::NICOVIDEO_ORIGIN)
            .header(header::COOKIE, cookie)
            .json(&serde_json::json!({ "videoId": video_id, "seconds": seconds }))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.bytes().await.unwrap_or_default();
            return Err(nvapi::status_error("playback-position", status, &body));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn logged_in() -> Arc<SessionStore> {
        let session = SessionStore::empty();
        session.set("user_session_1_abc".into());
        session.set_domand_bid("bid".into());
        Arc::new(session)
    }

    #[tokio::test]
    async fn puts_the_position_like_the_web_player() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("PUT", "/v2/users/me/watch/history/playback-position")
            .match_header("x-frontend-id", "6")
            .match_header("x-frontend-version", "0")
            .match_header("x-request-with", "https://www.nicovideo.jp")
            // domand_bid は付けない
            .match_header("cookie", "user_session=user_session_1_abc")
            .match_body(mockito::Matcher::Json(
                serde_json::json!({ "videoId": "sm9", "seconds": 42.123 }),
            ))
            .with_status(200)
            .with_body(r#"{"meta":{"status":200}}"#)
            .create_async()
            .await;
        let client = WatchHistoryClient::with_base(&server.url(), logged_in()).unwrap();
        client
            .save_playback_position("sm9", 42.12345)
            .await
            .expect("saved");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn refuses_without_a_login_or_with_bad_input() {
        let server = mockito::Server::new_async().await;
        let guest =
            WatchHistoryClient::with_base(&server.url(), Arc::new(SessionStore::empty())).unwrap();
        assert!(guest.save_playback_position("sm9", 10.0).await.is_err());

        let client = WatchHistoryClient::with_base(&server.url(), logged_in()).unwrap();
        for bad in [-1.0, f64::NAN, f64::INFINITY, MAX_SECONDS + 1.0] {
            assert!(client.save_playback_position("sm9", bad).await.is_err());
        }
        assert!(client.save_playback_position("../x", 1.0).await.is_err());
    }

    #[tokio::test]
    async fn surfaces_the_error_code() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("PUT", "/v2/users/me/watch/history/playback-position")
            .with_status(401)
            .with_body(r#"{"meta":{"status":401,"errorCode":"UNAUTHORIZED"}}"#)
            .create_async()
            .await;
        let client = WatchHistoryClient::with_base(&server.url(), logged_in()).unwrap();
        let err = client.save_playback_position("sm9", 1.0).await.unwrap_err();
        assert!(err.to_string().contains("UNAUTHORIZED"), "{err}");
    }
}
