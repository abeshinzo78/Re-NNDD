//! Niconico nv-comment threads API client.
//!
//! Endpoint: `POST {server}/v1/threads`
//! Reference: `abeshinzo78/NicoCommentDL` `niconico.js::fetchComments`.
//!
//! Returns a flat `Vec<Comment>` aggregated from all threads (owner / main /
//! easy fork). The `is_owner` flag is set when the thread fork is `"owner"`,
//! matching the legacy XML semantics that NNDD users expect.

use async_trait::async_trait;
use reqwest::header;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::auth::SessionStore;
use crate::api::video::NvCommentSetup;
use crate::error::ApiError;

const FRONTEND_ID: &str = "6";
const FRONTEND_VERSION: &str = "0";
const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    /// Stable id from the server (e.g. `"abc-123"`).
    pub id: String,
    pub no: i64,
    pub vpos_ms: i64,
    pub content: String,
    /// Comment commands joined by space — preserves the legacy `mail` field
    /// shape that niconicomments expects when rendering.
    pub mail: String,
    pub commands: Vec<String>,
    pub user_id: Option<String>,
    pub posted_at: Option<String>,
    /// `"owner"`, `"main"`, `"easy"`.
    pub fork: String,
    /// Convenience: `fork == "owner"`.
    pub is_owner: bool,
    pub nicoru_count: Option<i64>,
    pub score: Option<i64>,
}

#[async_trait]
pub trait CommentApi: Send + Sync {
    async fn fetch_comments(&self, setup: &NvCommentSetup) -> Result<Vec<Comment>, ApiError>;
}

pub struct ThreadsClient {
    http: reqwest::Client,
    session: std::sync::Arc<SessionStore>,
}

impl ThreadsClient {
    pub fn new(session: std::sync::Arc<SessionStore>) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .user_agent(BROWSER_UA)
            .gzip(true)
            .build()?;
        Ok(Self { http, session })
    }
}

#[async_trait]
impl CommentApi for ThreadsClient {
    async fn fetch_comments(&self, setup: &NvCommentSetup) -> Result<Vec<Comment>, ApiError> {
        let url = format!("{}/v1/threads", setup.server.trim_end_matches('/'));

        let body = serde_json::json!({
            "params": setup.params,
            "threadKey": setup.thread_key,
            "additionals": {},
        });

        let mut request = self
            .http
            .post(&url)
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-frontend-id", FRONTEND_ID)
            .header("x-frontend-version", FRONTEND_VERSION);
        // Threads endpoint normally uses `credentials: 'omit'`, but if the
        // user has supplied a session we forward it for any login-gated
        // threads variants.
        if let Some(cookie) = self.session.cookie_header() {
            request = request.header(header::COOKIE, cookie);
        }

        let response = request.json(&body).send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;

        if !status.is_success() {
            let detail = String::from_utf8_lossy(&bytes).into_owned();
            return Err(ApiError::ServerError {
                status: status.as_u16(),
                message: format!("threads API {status}: {detail}"),
            });
        }

        let envelope: Value = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::ResponseShape(format!("failed to parse threads body: {e}")))?;
        let meta_status = envelope
            .pointer("/meta/status")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if meta_status != 200 {
            return Err(ApiError::ResponseShape(format!(
                "unexpected meta.status from threads API: {meta_status}"
            )));
        }
        Ok(parse_threads(&envelope))
    }
}

/// Public so unit tests can validate the projection without HTTP.
pub fn parse_threads(envelope: &Value) -> Vec<Comment> {
    let Some(threads) = envelope.pointer("/data/threads").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for thread in threads {
        let fork = thread
            .get("fork")
            .and_then(Value::as_str)
            .unwrap_or("main")
            .to_string();
        let is_owner = fork == "owner";

        let Some(comments) = thread.get("comments").and_then(Value::as_array) else {
            continue;
        };
        for c in comments {
            let Some(parsed) = parse_comment(c, &fork, is_owner) else {
                continue;
            };
            out.push(parsed);
        }
    }
    out
}

fn parse_comment(c: &Value, fork: &str, is_owner: bool) -> Option<Comment> {
    let body = c.get("body").and_then(Value::as_str)?.to_string();
    let no = c.get("no").and_then(Value::as_i64).unwrap_or(0);
    let vpos_ms = c.get("vposMs").and_then(Value::as_i64).unwrap_or(0);
    let id = c
        .get("id")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| format!("{fork}-{no}"));

    let commands: Vec<String> = c
        .get("commands")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let mail = commands.join(" ");

    let user_id = c.get("userId").and_then(|v| {
        v.as_str()
            .map(String::from)
            .or_else(|| v.as_i64().map(|n| n.to_string()))
    });

    Some(Comment {
        id,
        no,
        vpos_ms,
        content: body,
        mail,
        commands,
        user_id,
        posted_at: c.get("postedAt").and_then(Value::as_str).map(String::from),
        fork: fork.to_string(),
        is_owner,
        nicoru_count: c.get("nicoruCount").and_then(Value::as_i64),
        score: c.get("score").and_then(Value::as_i64),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_owner_and_main_threads() {
        let envelope = json!({
            "meta": {"status": 200},
            "data": {
                "threads": [
                    {
                        "id": "owner",
                        "fork": "owner",
                        "comments": [
                            {"id": "o1", "no": 1, "vposMs": 100, "body": "投稿者コメ",
                             "commands": ["white", "small"], "userId": "owner-1",
                             "postedAt": "2024-01-01T00:00:00+09:00"}
                        ]
                    },
                    {
                        "id": "main",
                        "fork": "main",
                        "comments": [
                            {"id": "m1", "no": 1, "vposMs": 1500, "body": "wwwwwww",
                             "commands": [], "userId": "user-1"},
                            {"id": "m2", "no": 2, "vposMs": 3000, "body": "弾幕薄いよ",
                             "commands": ["red", "big"], "userId": "user-2"}
                        ]
                    }
                ]
            }
        });

        let comments = parse_threads(&envelope);
        assert_eq!(comments.len(), 3);

        let owner = &comments[0];
        assert!(owner.is_owner);
        assert_eq!(owner.fork, "owner");
        assert_eq!(owner.mail, "white small");

        let danmaku = &comments[2];
        assert!(!danmaku.is_owner);
        assert_eq!(danmaku.commands, vec!["red", "big"]);
        assert_eq!(danmaku.mail, "red big");
        assert_eq!(danmaku.vpos_ms, 3000);
    }

    #[test]
    fn empty_threads_yields_empty_vec() {
        let envelope = json!({"meta": {"status": 200}, "data": {"threads": []}});
        assert_eq!(parse_threads(&envelope).len(), 0);
    }

    #[test]
    fn missing_data_yields_empty_vec() {
        let envelope = json!({"meta": {"status": 200}});
        assert_eq!(parse_threads(&envelope).len(), 0);
    }
}
