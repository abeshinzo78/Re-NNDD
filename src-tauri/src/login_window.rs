//! アプリ内のブラウザ (WebView) で niconico にログインし、`user_session` を受け取る。
//!
//! niconico のログインは 2026 年に Cloudflare Turnstile 付きの SPA
//! (`account.nicovideo.jp/spa/login/…` → `api.id.nicovideo.jp`) へ移り、
//! メールアドレスとパスワードを POST するだけのログインはできなくなった
//! (`account.nicovideo.jp/api/v1/login` は 404)。そこで本物のログインページを
//! 別ウィンドウで開き、ログインが済んで WebView の Cookie に `user_session`
//! が入ったら取り出して保存する。二段階認証や外部アカウントでのログインも
//! そのページ上でそのまま行える。
//!
//! このウィンドウが読み込むのは外部 (remote) のページで、capability も付けて
//! いないので、Tauri の ACL によりアプリのコマンドは一切呼べない。

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::api::account::{fetch_account_state, AccountInfo, AccountState};
use crate::api::auth::SessionStore;
use crate::error::{AppError, Result};
use crate::library::db::LibraryHandle;

const LABEL: &str = "niconico-login";
const LOGIN_URL: &str = "https://account.nicovideo.jp/login?site=niconico&next_url=%2F";
/// `user_session` は `.nicovideo.jp` に付くので、この URL の Cookie を見ればよい。
const COOKIE_URL: &str = "https://www.nicovideo.jp/";
const POLL_INTERVAL: Duration = Duration::from_millis(800);
/// 放置されたウィンドウを待ち続けない。
const TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoginWindowResult {
    /// ログインできた。`account` は確認できた時だけ入る。
    Success { account: Option<AccountInfo> },
    /// ログインせずにウィンドウが閉じられた。
    Canceled,
    /// 時間切れ (ウィンドウは閉じた)。
    TimedOut,
}

fn parse_url(raw: &str) -> Result<tauri::Url> {
    raw.parse()
        .map_err(|e| AppError::Other(format!("invalid url {raw}: {e}")))
}

/// WebView の Cookie から有効そうな `user_session` を探す。
fn read_user_session(window: &tauri::WebviewWindow) -> Option<String> {
    let url = parse_url(COOKIE_URL).ok()?;
    let cookies = match window.cookies_for_url(url) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(error = %e, "reading login window cookies failed");
            return None;
        }
    };
    cookies
        .iter()
        .find(|c| c.name() == "user_session" && !c.value().is_empty() && c.value() != "deleted")
        .map(|c| c.value().to_string())
}

/// 前回のログインで残ったセッション Cookie を WebView から消す。残っていると
/// フォームが出ずに、そのまま古い (別アカウントかもしれない) セッションで
/// 閉じてしまう。サーバー側のセッションには触れない (ログアウトはしない)。
fn forget_session_cookies(window: &tauri::WebviewWindow) {
    let Ok(url) = parse_url(COOKIE_URL) else {
        return;
    };
    let Ok(cookies) = window.cookies_for_url(url) else {
        return;
    };
    for cookie in cookies {
        if matches!(cookie.name(), "user_session" | "user_session_secure") {
            if let Err(e) = window.delete_cookie(cookie) {
                tracing::debug!(error = %e, "could not delete a stale session cookie");
            }
        }
    }
}

/// niconico のログインページを別ウィンドウで開き、ログインが済むまで待つ。
///
/// 成功すると `user_session` を保存してウィンドウを閉じる。ウィンドウが
/// 閉じられたら `Canceled`。
#[tauri::command]
pub async fn open_login_window(
    app: tauri::AppHandle,
    store: State<'_, Arc<SessionStore>>,
    library: State<'_, Arc<LibraryHandle>>,
) -> Result<LoginWindowResult> {
    if let Some(existing) = app.get_webview_window(LABEL) {
        let _ = existing.set_focus();
        return Err(AppError::Other(
            "ログイン用のウィンドウは既に開いています".into(),
        ));
    }

    let window =
        WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::External(parse_url("about:blank")?))
            .title("ニコニコにログイン — Re:NNDD")
            .inner_size(520.0, 800.0)
            .min_inner_size(400.0, 560.0)
            .center()
            .focused(true)
            // 素の http や独自スキームへは遷移させない。
            .on_navigation(|url| matches!(url.scheme(), "https" | "about"))
            .build()
            .map_err(|e| {
                AppError::Other(format!("ログイン用のウィンドウを開けませんでした: {e}"))
            })?;

    forget_session_cookies(&window);
    window
        .navigate(parse_url(LOGIN_URL)?)
        .map_err(|e| AppError::Other(format!("ログインページを開けませんでした: {e}")))?;

    let started = Instant::now();
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        let Some(window) = app.get_webview_window(LABEL) else {
            tracing::info!("login window closed before logging in");
            return Ok(LoginWindowResult::Canceled);
        };
        if started.elapsed() > TIMEOUT {
            let _ = window.close();
            return Ok(LoginWindowResult::TimedOut);
        }
        let Some(value) = read_user_session(&window) else {
            continue;
        };

        // ログイン途中の一時的な Cookie を拾わないよう、実際に通るか確かめる。
        let probe = SessionStore::empty();
        probe.set(value.clone());
        let account = match fetch_account_state(&probe).await {
            AccountState::Valid { account } => Some(account),
            // 通信エラーで確かめられなかった。Cookie はあるので保存はする。
            AccountState::Unknown { message } => {
                tracing::warn!(%message, "could not verify the new session; saving it anyway");
                None
            }
            AccountState::Expired | AccountState::LoggedOut => continue,
        };

        {
            let conn = library.lock().await;
            store.set_with_conn(value, &conn);
        }
        tracing::info!("logged in through the login window");
        let _ = window.close();
        return Ok(LoginWindowResult::Success { account });
    }
}
