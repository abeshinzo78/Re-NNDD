//! 公式 Android アプリ (9.19.0) の解析で見つけた経路を、本物の niconico に通す。
//!
//! - 検索サジェスト (`sug.search.nicovideo.jp`) とコンテンツツリーはログイン不要
//! - ストーリーボード・視聴者 NG 設定・再生位置の保存は `user_session` が要る
//!
//! 資格情報はリポジトリに置かないので環境変数から受け取り、未設定ならログイン系の
//! テストはスキップする。**再生位置のテストはそのアカウントの sm9 の再生位置を
//! 書き換える** (視聴履歴に sm9 が残る)。
//!
//! ```sh
//! NNDD_TEST_COOKIE='user_session=user_session_...' \
//!   cargo test --test niconico_apk_features_live -- --ignored --nocapture --test-threads=1
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use nndd_next_lib::api::auth::SessionStore;
use nndd_next_lib::api::contents_tree::ContentsTreeClient;
use nndd_next_lib::api::ng::ViewerNgClient;
use nndd_next_lib::api::suggest::SuggestClient;
use nndd_next_lib::api::video::NiconicoWatchClient;
use nndd_next_lib::api::watch_history::WatchHistoryClient;

fn logged_in_session() -> Option<Arc<SessionStore>> {
    let raw = std::env::var("NNDD_TEST_COOKIE").ok()?;
    let store = SessionStore::empty();
    store.set(raw);
    if !store.is_set() {
        return None;
    }
    Some(Arc::new(store))
}

#[tokio::test]
#[ignore = "外部ネットワーク (sug.search.nicovideo.jp) が要る"]
async fn search_suggestions_return_candidates() {
    let client = SuggestClient::new().unwrap();
    let candidates = client.suggest("初音").await.expect("suggest");
    eprintln!("candidates: {candidates:?}");
    assert!(!candidates.is_empty());
    assert!(candidates.iter().any(|c| c.starts_with("初音")));
}

#[tokio::test]
#[ignore = "外部ネットワーク (nvapi.nicovideo.jp) が要る"]
async fn contents_tree_lists_parents_and_children() {
    let client = ContentsTreeClient::new(None).unwrap();
    let sm9 = client.fetch("sm9").await.expect("sm9 tree");
    assert!(!sm9.children.is_empty(), "sm9 has many child works");
    assert!(sm9.children.len() <= 15);
    assert!(sm9.children.iter().all(|v| !v.id.is_empty()));

    // 親作品を登録している動画 (RTA の part2 → part1)。
    let child = client.fetch("sm32302688").await.expect("tree with parents");
    assert!(child.parents.iter().any(|v| v.id == "sm32300945"));

    // 削除済みは空配列で返る。
    let deleted = client.fetch("sm1").await.expect("deleted video");
    assert!(deleted.parents.is_empty() && deleted.children.is_empty());
}

#[tokio::test]
#[ignore = "外部ネットワーク + NNDD_TEST_COOKIE が要る"]
async fn storyboard_is_issued_for_logged_in_sessions() {
    // ゲストでは出せない (問い合わせもしない)。
    let guest = NiconicoWatchClient::new(Arc::new(SessionStore::empty())).unwrap();
    assert_eq!(
        guest.fetch_storyboard("sm9", None, None).await.unwrap(),
        None
    );

    let Some(session) = logged_in_session() else {
        eprintln!("NNDD_TEST_COOKIE 未設定のためスキップ");
        return;
    };
    let watch = NiconicoWatchClient::new(Arc::clone(&session)).unwrap();
    let page = watch.fetch_watch_v4("sm9").await.expect("watch v4");
    assert!(!page.is_guest, "the session must be valid");
    assert!(page.is_storyboard_available);
    let bid_before = session.domand_bid();

    // 再生時と同じ鍵で発行する (HLS を出した後でも通る)。
    let key = page.domand.as_ref().unwrap().access_right_key.clone();
    let sb = watch
        .fetch_storyboard("sm9", Some(&key), page.watch_track_id.as_deref())
        .await
        .expect("storyboard")
        .expect("sm9 has a storyboard");
    eprintln!(
        "storyboard: {}x{} {}x{} every {} ms, {} sheets",
        sb.thumbnail_width,
        sb.thumbnail_height,
        sb.columns,
        sb.rows,
        sb.interval_ms,
        sb.images.len()
    );
    assert!(sb.columns > 0 && sb.rows > 0 && sb.interval_ms > 0);
    assert!(!sb.images.is_empty());

    // スプライトは Cookie も Referer も無しで取れる (WebView の <img> に直接渡す前提)。
    let http = reqwest::Client::new();
    let resp = http.get(&sb.images[0].url).send().await.unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(ct.starts_with("image/"), "{ct}");

    // 鍵が通らない時は視聴データを取り直す。その時も再生中の domand_bid は変えない。
    let again = watch
        .fetch_storyboard("sm9", Some("not-a-valid-key"), None)
        .await
        .expect("fallback")
        .expect("storyboard via refetch");
    assert!(!again.images.is_empty());
    assert_eq!(session.domand_bid(), bid_before);
}

#[tokio::test]
#[ignore = "外部ネットワーク + NNDD_TEST_COOKIE が要る"]
async fn viewer_ng_settings_can_be_read() {
    let guest = ViewerNgClient::new(Arc::new(SessionStore::empty())).unwrap();
    assert!(guest.fetch().await.is_err());

    let Some(session) = logged_in_session() else {
        eprintln!("NNDD_TEST_COOKIE 未設定のためスキップ");
        return;
    };
    let list = ViewerNgClient::new(session)
        .unwrap()
        .fetch()
        .await
        .expect("viewer NG");
    eprintln!(
        "viewer NG: revision {:?}, {} items",
        list.revision,
        list.items.len()
    );
    assert!(list
        .items
        .iter()
        .all(|i| !i.kind.is_empty() && !i.source.is_empty()));
}

#[tokio::test]
#[ignore = "外部ネットワーク + NNDD_TEST_COOKIE が要る (sm9 の再生位置を書き換える)"]
async fn playback_position_is_saved_for_logged_in_sessions() {
    let guest = WatchHistoryClient::new(Arc::new(SessionStore::empty())).unwrap();
    assert!(guest.save_playback_position("sm9", 10.0).await.is_err());

    let Some(session) = logged_in_session() else {
        eprintln!("NNDD_TEST_COOKIE 未設定のためスキップ");
        return;
    };
    let client = WatchHistoryClient::new(session).unwrap();
    client
        .save_playback_position("sm9", 30.5)
        .await
        .expect("PUT playback-position");
    // 見終わった扱い (Web プレイヤーは終了時に 0 を送る)。
    client
        .save_playback_position("sm9", 0.0)
        .await
        .expect("PUT playback-position 0");
}
