//! 視聴まわり (nvapi v4 → HLS → AES 鍵 → セグメント、lazy、コメント) を本物の
//! niconico に対して通す。
//!
//! niconico は 2026 年に watch を nvapi v4 へ移し、watch ページの埋め込みも
//! `$watchV4` 形になった。ユニットテストは保存した応答で形を見るだけなので、
//! 「今の niconico で実際に再生まで届くか」はここで確かめる。
//!
//! ログイン経路を見るには `user_session` が要る。資格情報をリポジトリに
//! 置かないため環境変数から受け取り、未設定ならログイン系のテストはスキップする。
//!
//! ```sh
//! NNDD_TEST_COOKIE='user_session=user_session_...' \
//!   cargo test --test niconico_watch_live -- --ignored --nocapture --test-threads=1
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use nndd_next_lib::api::auth::SessionStore;
use nndd_next_lib::api::comment::{CommentApi, ThreadsClient};
use nndd_next_lib::api::video::{quality_candidates, NiconicoWatchClient, WatchPageData};
use nndd_next_lib::error::ApiError;

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// シリーズに入っている動画 (AI実況シリーズの第 1 話)。
const SERIES_VIDEO: &str = "sm41984256";

fn logged_in_session() -> Option<Arc<SessionStore>> {
    let raw = std::env::var("NNDD_TEST_COOKIE").ok()?;
    let store = SessionStore::empty();
    store.set(raw);
    if !store.is_set() {
        return None;
    }
    Some(Arc::new(store))
}

/// master → 最初の media playlist → AES 鍵 → 最初のセグメントまで取る。
/// 戻り値は (variant 数, 鍵の長さ, セグメントの長さ)。
async fn walk_hls(master_url: &str, session: &SessionStore) -> (usize, usize, usize) {
    let http = reqwest::Client::builder().user_agent(UA).build().unwrap();
    let cookie = session
        .cookie_header()
        .expect("domand_bid must be known before touching the CDN");
    let get = |url: String| {
        let http = http.clone();
        let cookie = cookie.clone();
        async move {
            let resp = http
                .get(&url)
                .header("Referer", "https://www.nicovideo.jp/")
                .header("Cookie", cookie)
                .send()
                .await
                .unwrap();
            let status = resp.status();
            let bytes = resp.bytes().await.unwrap();
            assert!(status.is_success(), "GET {url} -> {status}");
            bytes
        }
    };
    let master = String::from_utf8(get(master_url.to_string()).await.to_vec()).unwrap();
    let variants: Vec<&str> = master
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    assert!(!variants.is_empty(), "master has no variants:\n{master}");
    let media = String::from_utf8(get(variants[0].to_string()).await.to_vec()).unwrap();
    let key_uri = media
        .lines()
        .find(|l| l.starts_with("#EXT-X-KEY"))
        .and_then(|l| l.split("URI=\"").nth(1))
        .and_then(|rest| rest.split('"').next())
        .expect("media playlist has an AES key")
        .to_string();
    let key = get(key_uri).await;
    let segment = media
        .lines()
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .expect("media playlist has segments")
        .to_string();
    let seg = get(segment).await;
    (variants.len(), key.len(), seg.len())
}

async fn play_through(
    label: &str,
    page: &WatchPageData,
    client: &NiconicoWatchClient,
    session: &SessionStore,
) {
    let domand = page.domand.as_ref().expect("playable media");
    let outputs: Vec<(String, String)> = quality_candidates(domand)
        .into_iter()
        .map(|c| (c.video_track, c.audio_track))
        .collect();
    assert!(!outputs.is_empty(), "{label}: no quality candidates");
    let hls = client
        .resolve_hls(page, &outputs)
        .await
        .expect("resolve hls");
    let (variants, key_len, seg_len) = walk_hls(&hls.content_url, session).await;
    println!("[{label}] variants={variants} key={key_len}B first segment={seg_len}B");
    assert_eq!(key_len, 16, "{label}: AES-128 key must be 16 bytes");
    assert!(seg_len > 1000, "{label}: segment looks empty");
}

#[tokio::test]
#[ignore = "外部ネットワーク (nicovideo.jp) が要る"]
async fn guest_can_play_through_v4_preissued_hls() {
    let session = Arc::new(SessionStore::empty());
    let client = NiconicoWatchClient::new(Arc::clone(&session)).unwrap();
    let mut page = client.fetch_watch_data("sm9").await.expect("watch v4");
    assert!(page.is_guest);
    assert!(page.preissued_hls.is_some(), "v4 should pre-issue HLS");
    assert!(session.domand_bid().is_some(), "v4 should set domand_bid");
    client.enrich_with_lazy(&mut page).await;
    let owner = page.owner.clone().expect("owner");
    assert_eq!(owner.id.as_deref(), Some("4"));
    play_through("guest sm9", &page, &client, &session).await;

    let comments = ThreadsClient::new(Arc::clone(&session))
        .unwrap()
        .fetch_comments(page.nv_comment.as_ref().expect("nvComment"))
        .await
        .expect("comments");
    println!("[guest sm9] comments={}", comments.len());
    assert!(comments.len() > 100);
}

#[tokio::test]
#[ignore = "外部ネットワーク (nicovideo.jp) が要る"]
async fn guest_access_rights_fallback_works_without_preissued_hls() {
    // HTML 埋め込み経路 (v4 API が使えない時) を通す: 事前発行が無いので
    // access-rights を watchTrackId で叩き、Cookie は付けない。
    use nndd_next_lib::api::video::WatchApi;
    let session = Arc::new(SessionStore::empty());
    let client = NiconicoWatchClient::new(Arc::clone(&session)).unwrap();
    let page = client.fetch_watch_page("sm9").await.expect("watch page");
    assert!(page.is_guest);
    assert!(page.preissued_hls.is_none());
    play_through("guest html sm9", &page, &client, &session).await;
}

#[tokio::test]
#[ignore = "外部ネットワーク + NNDD_TEST_COOKIE が要る"]
async fn logged_in_playback_lazy_series_and_comments() {
    let Some(session) = logged_in_session() else {
        eprintln!("NNDD_TEST_COOKIE 未設定のためスキップ");
        return;
    };
    let client = NiconicoWatchClient::new(Arc::clone(&session)).unwrap();
    let mut page = client
        .fetch_watch_data(SERIES_VIDEO)
        .await
        .expect("watch v4");
    assert!(!page.is_guest, "the session should be accepted (expired?)");
    assert!(page.viewer.is_some());
    client.enrich_with_lazy(&mut page).await;
    let series = page.series.clone().expect("series from lazy");
    println!(
        "[user] series={} {:?} next={:?}",
        series.id,
        series.title,
        series.next_video.as_ref().map(|v| &v.id)
    );
    assert_eq!(series.id, 425178);
    assert!(page.genre.is_some());
    play_through("user", &page, &client, &session).await;

    let comments = ThreadsClient::new(Arc::clone(&session))
        .unwrap()
        .fetch_comments_patiently(page.nv_comment.as_ref().unwrap())
        .await
        .expect("comments");
    println!("[user] comments={}", comments.len());
    assert!(!comments.is_empty());
}

#[tokio::test]
#[ignore = "外部ネットワーク + NNDD_TEST_COOKIE が要る"]
async fn expired_session_falls_back_to_guest_keys_without_breaking_playback() {
    // 失効した user_session を持っていても v4 はゲスト扱いで 200 を返す。
    // その鍵に user_session を付けると access-rights が 400 になるので、
    // ゲスト判定が効いていることを確かめる。
    let session = Arc::new(SessionStore::empty());
    session.set(
        "user_session_1_0000000000000000000000000000000000000000000000000000000000000000".into(),
    );
    let client = NiconicoWatchClient::new(Arc::clone(&session)).unwrap();
    let mut page = client.fetch_watch_data("sm9").await.expect("watch v4");
    assert!(page.is_guest, "invalid sessions come back as guest data");
    // 事前発行を使わず access-rights 経路を通す。
    page.preissued_hls = None;
    play_through("expired session", &page, &client, &session).await;
}

#[tokio::test]
#[ignore = "外部ネットワーク (nicovideo.jp) が要る"]
async fn channel_video_owner_and_refusals() {
    let session = Arc::new(SessionStore::empty());
    let client = NiconicoWatchClient::new(Arc::clone(&session)).unwrap();
    let mut page = client
        .fetch_watch_data("so46975082")
        .await
        .expect("watch v4");
    client.enrich_with_lazy(&mut page).await;
    let owner = page.owner.expect("owner");
    assert_eq!(owner.kind, "channel");
    assert_eq!(owner.id.as_deref(), Some("2650183"));

    let err = client.fetch_watch_data("sm1").await.unwrap_err();
    println!("[sm1] {err}");
    assert!(
        matches!(err, ApiError::WatchRefused { status: 403, .. }),
        "{err:?}"
    );
    let err = client.fetch_watch_data("sm999999999999").await.unwrap_err();
    assert!(
        matches!(err, ApiError::WatchRefused { status: 404, .. }),
        "{err:?}"
    );
}
