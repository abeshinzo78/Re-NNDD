//! Niconico session/cookie storage and password login.
//!
//! Two ways to seed [`SessionStore`]:
//! - `login_with_password` — POST to `account.nicovideo.jp` and lift
//!   `user_session` out of `Set-Cookie`.
//! - `set` — accept a pasted `user_session` value (2FA / SSO fallback).
//!
//! The cookie is held in memory only. CLAUDE.md mandates OS keyring
//! (`tauri-plugin-stronghold` / `keyring`) for persistent storage; that
//! lands when the auth flow matures (we do not want to ship plaintext
//! credentials to disk in the meantime).

use parking_lot::RwLock;
use reqwest::header::SET_COOKIE;
use reqwest::redirect::Policy;

use crate::error::ApiError;

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

    pub fn set(&self, value: String) {
        let trimmed = value.trim().to_owned();
        if trimmed.is_empty() {
            *self.user_session.write() = None;
        } else {
            *self.user_session.write() = Some(trimmed);
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

    pub fn set_domand_bid(&self, value: String) {
        let trimmed = value.trim().to_owned();
        if trimmed.is_empty() {
            *self.domand_bid.write() = None;
        } else {
            *self.domand_bid.write() = Some(trimmed);
        }
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

/// Outcome of a password-based login attempt. The success branch carries
/// the freshly-extracted `user_session` token; the caller is responsible
/// for handing it to [`SessionStore::set`].
#[derive(Debug, Clone)]
pub enum LoginOutcome {
    Success { user_session: String },
    Mfa { mfa_session: Option<String> },
    InvalidCredentials,
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
}
