//! nvapi (`https://nvapi.nicovideo.jp`) を叩く時の共通部品。
//!
//! nvapi は `X-Frontend-Id` / `X-Frontend-Version` が無いと 400 を返し、UA が
//! `reqwest/…` のままだと空の結果を返すことがある。応答は `{ meta, data }` の
//! 封筒で、失敗時は `meta.errorCode` に理由が入る。
//!
//! 既存のモジュール (`video` / `search` / `account` など) はそれぞれ同じ定数を
//! 持っているが、ここでは新しく足したクライアント (NG 設定・視聴履歴・コンテンツ
//! ツリー) だけがこれを使う。

use std::time::Duration;

use reqwest::{header, StatusCode};
use serde_json::Value;

use crate::error::ApiError;

pub const NVAPI_BASE: &str = "https://nvapi.nicovideo.jp";
/// niconico の Web クライアントと同じ frontend 識別子。
pub const FRONTEND_ID: &str = "6";
pub const FRONTEND_VERSION: &str = "0";
/// 書き込み系 (再生位置の保存など) は `X-Request-With` にこの origin を要求する。
pub const NICOVIDEO_ORIGIN: &str = "https://www.nicovideo.jp";
pub const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// nvapi 用の HTTP クライアント (ブラウザ UA・gzip・タイムアウト付き)。
pub fn build_http() -> Result<reqwest::Client, ApiError> {
    Ok(reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .gzip(true)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()?)
}

/// `base` の末尾の `/` を落とす (テストで mockito の URL を渡す時のため)。
pub fn trim_base(base: &str) -> String {
    base.trim_end_matches('/').to_string()
}

/// nvapi へ送る共通ヘッダーを付ける。
pub fn with_frontend_headers(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    request
        .header("X-Frontend-Id", FRONTEND_ID)
        .header("X-Frontend-Version", FRONTEND_VERSION)
        .header(header::REFERER, format!("{NICOVIDEO_ORIGIN}/"))
        .header(header::ACCEPT, "application/json")
}

/// `{ meta: { errorCode } }` からコードだけ取り出す。
pub fn error_code(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    value
        .pointer("/meta/errorCode")
        .and_then(Value::as_str)
        .map(String::from)
}

/// 非 2xx の応答を [`ApiError`] にする。429 は [`ApiError::RateLimited`]。
pub fn status_error(label: &str, status: StatusCode, body: &[u8]) -> ApiError {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return ApiError::RateLimited;
    }
    let detail = match error_code(body) {
        Some(code) => code,
        None => String::from_utf8_lossy(body).chars().take(200).collect(),
    };
    ApiError::ServerError {
        status: status.as_u16(),
        message: format!("{label} returned {status}: {detail}"),
    }
}

/// 成功応答の本文から `data` を取り出す。`data` が object でなければ形の誤り。
pub fn parse_data(label: &str, body: &[u8]) -> Result<Value, ApiError> {
    let mut root: Value = serde_json::from_slice(body)
        .map_err(|e| ApiError::ResponseShape(format!("{label}: JSON parse failed: {e}")))?;
    match root.get_mut("data").map(Value::take) {
        Some(data @ Value::Object(_)) => Ok(data),
        _ => Err(ApiError::ResponseShape(format!(
            "{label}: response has no data object"
        ))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn maps_statuses_to_errors() {
        assert!(matches!(
            status_error("x", StatusCode::TOO_MANY_REQUESTS, b""),
            ApiError::RateLimited
        ));
        let err = status_error(
            "users/me",
            StatusCode::UNAUTHORIZED,
            br#"{"meta":{"status":401,"errorCode":"UNAUTHORIZED"}}"#,
        );
        match err {
            ApiError::ServerError { status, message } => {
                assert_eq!(status, 401);
                assert!(message.contains("UNAUTHORIZED"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn extracts_the_data_object() {
        let data = parse_data("x", br#"{"meta":{"status":200},"data":{"a":1}}"#).unwrap();
        assert_eq!(data["a"], 1);
        assert!(parse_data("x", br#"{"meta":{"status":200}}"#).is_err());
        assert!(parse_data("x", br#"{"data":[]}"#).is_err());
        assert!(parse_data("x", b"not json").is_err());
    }
}
