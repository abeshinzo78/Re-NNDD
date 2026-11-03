//! Niconico session/cookie storage and password login.
//!
//! Two ways to seed [`SessionStore`]:
//! - `login_with_password` — POST to `account.nicovideo.jp` and lift
//!   `user_session` out of `Set-Cookie`.
//! - `set` — accept a pasted `user_session` value (2FA / SSO fallback).
//!
//! The cookie is persisted to the `settings` SQLite table under the key
//! `auth.user_session` so that it survives app restarts. The in-memory
//! copy is the authoritative live state; the on-disk copy is a mirror
//! that is kept in sync by `set` / `clear`.

use parking_lot::RwLock;
use reqwest::header::SET_COOKIE;
use reqwest::redirect::Policy;

use crate::error::ApiError;
use crate::library::settings;

const SETTINGS_KEY: &str = "auth.user_session";

/// `value` を trim して、空なら `None`, それ以外は `Some(trimmed)` を `slot` に
/// 書き込む。Cookie 系のフィールドが空白/空文字渡しでクリアされる挙動を一箇所
/// にまとめるためのヘルパ。
fn write_trimmed(slot: &RwLock<Option<String>>, value: String) {
    let trimmed = value.trim().to_owned();
    *slot.write() = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    };
}

const LOGIN_URL: &str = "https://account.nicovideo.jp/api/v1/login?site=niconico";
const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

#[derive(Default)]
pub struct SessionStore {
    user_session: RwLock<Option<String>>,
    /// Domand binding cookie (`domand_bid`). niconico's CloudFront / Lambda
    /// rejects HLS playlist + segment requests with HTTP 403 unless this
    /// cookie matches the value that was issued alongside the signed URL.
    /// Set automatically on every successful `access-rights/hls` call.
    domand_bid: RwLock<Option<String>>,
}

impl SessionStore {
    pub const fn empty() -> Self {
        Self {
            user_session: RwLock::new(None),
            domand_bid: RwLock::new(None),
        }
    }

    /// Load the persisted `user_session` from the settings table into memory.
    /// Call once during app startup, before the store is handed to commands.
    pub fn load_from_db(&self, conn: &rusqlite::Connection) {
        if let Ok(Some(value)) = settings::get(conn, SETTINGS_KEY) {
            // 旧版は貼り付けを無加工で保存していたので `user_session=…` の形で
            // 入っていることがある。読み込み時にも値だけへ正規化する。
            if let Some(normalized) = extract_user_session(&value) {
                *self.user_session.write() = Some(normalized);
                tracing::info!("restored user_session from persistent storage");
            }
        }
    }

    fn persist_to_db(&self, conn: &rusqlite::Connection) {
        let guard = self.user_session.read();
        if let Err(e) = match guard.as_deref() {
            Some(val) => settings::set(conn, SETTINGS_KEY, val),
            None => settings::delete(conn, SETTINGS_KEY).map(|_| ()),
        } {
            tracing::warn!(error = %e, "failed to persist session to db");
        }
    }

    pub fn set_with_conn(&self, value: String, conn: &rusqlite::Connection) {
        self.set(value);
        self.persist_to_db(conn);
    }

    pub fn clear_with_conn(&self, conn: &rusqlite::Connection) {
        self.clear();
        self.persist_to_db(conn);
    }

    /// `user_session` を設定する。値だけでなく `user_session=…` や Cookie ヘッダー
    /// 丸ごとの貼り付けも受け付け、値の部分だけを保存する ([`extract_user_session`])。
    ///
    /// `domand_bid` は発行時のセッション (ゲスト / ログイン) に紐付いていて、
    /// ゲストで得た bid ではログイン中に発行された HLS の鍵が 403 になる (実測)。
    /// セッションが変わったら捨てて、次の watch / access-rights で取り直させる。
    pub fn set(&self, value: String) {
        let normalized = extract_user_session(&value).unwrap_or_default();
        let changed = self.user_session.read().as_deref() != Some(normalized.as_str());
        write_trimmed(&self.user_session, normalized);
        if changed {
            *self.domand_bid.write() = None;
        }
    }

    pub fn clear(&self) {
        *self.user_session.write() = None;
        *self.domand_bid.write() = None;
    }

    pub fn get(&self) -> Option<String> {
        self.user_session.read().clone()
    }

    pub fn is_set(&self) -> bool {
        self.user_session.read().is_some()
    }

    /// `user_session` だけの `Cookie:` 値。`domand_bid` を混ぜたくない
    /// nvapi 呼び出し (watch v4 など、応答で bid を発行し直させたいもの) 用。
    pub fn session_cookie(&self) -> Option<String> {
        self.get().map(|s| format!("user_session={s}"))
    }

    pub fn set_domand_bid(&self, value: String) {
        write_trimmed(&self.domand_bid, value);
    }

    pub fn domand_bid(&self) -> Option<String> {
        self.domand_bid.read().clone()
    }

    /// Build a `Cookie:` header value for niconico requests, or `None`
    /// when no auth state is configured (anonymous mode).
    pub fn cookie_header(&self) -> Option<String> {
        let mut parts = Vec::with_capacity(2);
        if let Some(s) = self.get() {
            parts.push(format!("user_session={s}"));
        }
        if let Some(b) = self.domand_bid() {
            parts.push(format!("domand_bid={b}"));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("; "))
        }
    }
}

/// 貼り付けられた文字列から `user_session` の値を取り出す。
///
/// 受け付ける形:
/// - 値そのもの (`user_session_123_abc…`)
/// - `user_session=…` (前後に別の Cookie や `Cookie:` が付いていてもよい)
///
/// 区切り文字 (`;` `=` 空白) を含まない文字列は値そのものとみなす。旧版は
/// 入力をそのまま保存していたので、その挙動との互換を保つため。
pub fn extract_user_session(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let without_prefix = trimmed
        .strip_prefix("Cookie:")
        .or_else(|| trimmed.strip_prefix("cookie:"))
        .unwrap_or(trimmed)
        .trim();
    if without_prefix.is_empty() {
        return None;
    }
    for part in without_prefix.split([';', ',']) {
        if let Some(value) = part.trim().strip_prefix("user_session=") {
            let value = value.trim();
            if !value.is_empty() && value != "deleted" {
                return Some(value.to_string());
            }
        }
    }
    if without_prefix.contains(['=', ';', ' ', '\t', '\n']) {
        return None;
    }
    Some(without_prefix.to_string())
}

/// Outcome of a password-based login attempt. The success branch carries
/// the freshly-extracted `user_session` token; the caller is responsible
/// for handing it to [`SessionStore::set`].
#[derive(Debug, Clone)]
pub enum LoginOutcome {
    Success {
        user_session: String,
    },
    Mfa {
        mfa_session: Option<String>,
    },
    InvalidCredentials,
    /// ログイン API 自体が無くなっている (niconico は 2026 年に Turnstile 付きの
    /// SPA ログインへ移行し、`account.nicovideo.jp/api/v1/login` は 404 を返す)。
    /// パスワードの誤りと区別して、ブラウザでのログインへ案内するために使う。
    Unsupported {
        status: u16,
    },
}

/// 旧ログイン API が消えていることを示すステータスか。
fn login_endpoint_gone(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 404 | 405 | 410)
}

/// POST email + password to niconico's account endpoint and inspect the
/// resulting cookies. Redirects are *not* followed so that the original
/// `Set-Cookie` headers are visible.
pub async fn login_with_password(email: &str, password: &str) -> Result<LoginOutcome, ApiError> {
    if email.is_empty() || password.is_empty() {
        return Err(ApiError::InvalidQuery(
            "email and password must be provided".into(),
        ));
    }

    let client = reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .redirect(Policy::none())
        .build()?;

    let form = [("mail_tel", email), ("password", password)];
    let response = client.post(LOGIN_URL).form(&form).send().await?;
    if login_endpoint_gone(response.status()) {
        tracing::warn!(status = %response.status(), "password login endpoint is gone");
        return Ok(LoginOutcome::Unsupported {
            status: response.status().as_u16(),
        });
    }

    let mut user_session: Option<String> = None;
    let mut mfa_session: Option<String> = None;
    for header_value in response.headers().get_all(SET_COOKIE) {
        let raw = match header_value.to_str() {
            Ok(s) => s,
            Err(_) => continue,
        };
        if let Some(rest) = raw.strip_prefix("user_session=") {
            let value = rest.split(';').next().unwrap_or("");
            // niconico clears the cookie on failure with `user_session=deleted`.
            if !value.is_empty() && value != "deleted" {
                user_session = Some(value.to_string());
            }
        } else if let Some(rest) = raw.strip_prefix("mfa_session=") {
            let value = rest.split(';').next().unwrap_or("");
            if !value.is_empty() {
                mfa_session = Some(value.to_string());
            }
        }
    }

    if let Some(token) = user_session {
        return Ok(LoginOutcome::Success {
            user_session: token,
        });
    }
    if mfa_session.is_some() {
        return Ok(LoginOutcome::Mfa { mfa_session });
    }
    // niconico typically sets a `user_session=deleted` Set-Cookie on
    // wrong credentials. The `Location` redirect target also signals
    // failure — but the cookie absence is a sufficient indicator here.
    Ok(LoginOutcome::InvalidCredentials)
}

/// Complete MFA login with one-time password and mfa_session cookie.
pub async fn login_mfa(
    mfa_session: &str,
    one_time_password: &str,
) -> Result<LoginOutcome, ApiError> {
    if mfa_session.is_empty() || one_time_password.is_empty() {
        return Err(ApiError::InvalidQuery(
            "mfa_session and one_time_password must be provided".into(),
        ));
    }

    let client = reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .redirect(Policy::none())
        .build()?;

    let form = [("oneTimePassword", one_time_password)];
    let response = client
        .post(LOGIN_URL)
        .header("Cookie", format!("mfa_session={mfa_session}"))
        .form(&form)
        .send()
        .await?;
    if login_endpoint_gone(response.status()) {
        tracing::warn!(status = %response.status(), "MFA login endpoint is gone");
        return Ok(LoginOutcome::Unsupported {
            status: response.status().as_u16(),
        });
    }

    let mut user_session: Option<String> = None;
    for header_value in response.headers().get_all(SET_COOKIE) {
        let raw = match header_value.to_str() {
            Ok(s) => s,
            Err(_) => continue,
        };
        if let Some(rest) = raw.strip_prefix("user_session=") {
            let value = rest.split(';').next().unwrap_or("");
            if !value.is_empty() && value != "deleted" {
                user_session = Some(value.to_string());
            }
        }
    }

    if let Some(token) = user_session {
        return Ok(LoginOutcome::Success {
            user_session: token,
        });
    }
    Ok(LoginOutcome::InvalidCredentials)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_session() {
        let store = SessionStore::empty();
        assert!(!store.is_set());
        store.set("abc".into());
        assert_eq!(store.get().as_deref(), Some("abc"));
        assert_eq!(store.cookie_header().as_deref(), Some("user_session=abc"));
    }

    #[test]
    fn empty_string_clears() {
        let store = SessionStore::empty();
        store.set("abc".into());
        store.set("   ".into());
        assert!(!store.is_set());
    }

    #[test]
    fn extracts_user_session_from_pasted_forms() {
        let value = "user_session_123_abcdef";
        assert_eq!(extract_user_session(value).as_deref(), Some(value));
        assert_eq!(
            extract_user_session(&format!("  user_session={value}  ")).as_deref(),
            Some(value)
        );
        assert_eq!(
            extract_user_session(&format!(
                "Cookie: nicosid=1.2; user_session={value}; domand_bid=x"
            ))
            .as_deref(),
            Some(value)
        );
        assert_eq!(extract_user_session("user_session=deleted"), None);
        assert_eq!(extract_user_session("nicosid=1.2; other=3"), None);
        assert_eq!(extract_user_session("   "), None);
    }

    #[test]
    fn set_normalizes_cookie_pairs() {
        let store = SessionStore::empty();
        store.set("user_session=user_session_1_x".into());
        assert_eq!(store.get().as_deref(), Some("user_session_1_x"));
        assert_eq!(
            store.session_cookie().as_deref(),
            Some("user_session=user_session_1_x")
        );
    }

    #[test]
    fn changing_the_session_drops_the_domand_bid() {
        let store = SessionStore::empty();
        store.set_domand_bid("guest-bid".into());
        store.set("user_session_1_x".into());
        assert_eq!(
            store.domand_bid(),
            None,
            "ゲストの bid を持ち越してはいけない"
        );

        store.set_domand_bid("user-bid".into());
        store.set("user_session_1_x".into());
        assert_eq!(
            store.domand_bid().as_deref(),
            Some("user-bid"),
            "同じセッションの再設定では捨てない"
        );
    }
}
