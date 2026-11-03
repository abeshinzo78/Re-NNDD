//! 一覧・補助 API (チャンネル動画一覧、getthumbinfo のタグ、`/v1/users/me`) を
//! 本物の niconico に対して確かめる。
//!
//! チャンネル動画一覧は JSON API が廃止されて HTML を読んでいるので、
//! ページの作りが変わったらまずここで落ちる。
//!
//! ```sh
//! NNDD_TEST_COOKIE='user_session=user_session_...' \
//!   cargo test --test niconico_lists_live -- --ignored --nocapture --test-threads=1
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nndd_next_lib::api::account::{fetch_account_state, AccountState};
use nndd_next_lib::api::auth::SessionStore;
use nndd_next_lib::api::channel::{channel_videos_url, parse_channel_videos_html};
use nndd_next_lib::api::thumbinfo::{parse_thumbinfo_tags, ACCEPT as THUMBINFO_ACCEPT};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

async fn get_text(url: &str) -> String {
    get_text_accepting(url, "*/*").await
}

async fn get_text_accepting(url: &str, accept: &str) -> String {
    let http = reqwest::Client::builder().user_agent(UA).build().unwrap();
    let resp = http.get(url).header("Accept", accept).send().await.unwrap();
    assert!(resp.status().is_success(), "GET {url} -> {}", resp.status());
    resp.text().await.unwrap()
}

/// ランキング (アニメ) からチャンネル動画を 1 本探し、その投稿元の ID を返す。
async fn some_channel_id() -> String {
    let body = get_text(
        "https://www.nicovideo.jp/ranking/genre/zc49b03a?term=24h&page=1&responseType=json",
    )
    .await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let items = json
        .pointer("/data/response/$getTeibanRanking/data/items")
        .and_then(|v| v.as_array())
        .expect("ranking items");
    items
        .iter()
        .filter_map(|it| it.get("owner"))
        .find(|o| o.get("ownerType").and_then(|t| t.as_str()) == Some("channel"))
        .and_then(|o| o.get("id").and_then(|v| v.as_str()))
        .map(|id| id.trim_start_matches("ch").to_string())
        .expect("a channel video in the anime ranking")
}

#[tokio::test]
#[ignore = "外部ネットワーク (ch.nicovideo.jp) が要る"]
async fn channel_video_list_is_scrapable() {
    let channel = std::env::var("NNDD_TEST_CHANNEL").unwrap_or(some_channel_id().await);
    let url = channel_videos_url(
        "https://ch.nicovideo.jp",
        &channel,
        1,
        "registeredAt",
        "desc",
    );
    let page = parse_channel_videos_html(&get_text(&url).await);
    println!(
        "[ch{channel}] {:?} total={} first={:?}",
        page.channel_name,
        page.total_count,
        page.items
            .first()
            .map(|v| (&v.content_id, &v.title, v.view_counter))
    );
    assert!(!page.items.is_empty(), "no videos parsed from {url}");
    assert!(page.total_count >= page.items.len() as i64);
    for v in &page.items {
        assert!(v.content_id.len() > 2);
        assert!(!v.title.is_empty());
        assert!(v.length_seconds.is_some(), "{v:?}");
        assert!(v.start_time.is_some(), "{v:?}");
        assert!(v.thumbnail_url.is_some(), "{v:?}");
    }

    // 再生数順に並べ替えられること。
    let url = channel_videos_url("https://ch.nicovideo.jp", &channel, 1, "viewCount", "desc");
    let by_views = parse_channel_videos_html(&get_text(&url).await);
    let views: Vec<i64> = by_views
        .items
        .iter()
        .filter_map(|v| v.view_counter)
        .collect();
    assert!(
        views.windows(2).all(|w| w[0] >= w[1]),
        "not sorted by views: {views:?}"
    );
}

#[tokio::test]
#[ignore = "外部ネットワーク (ch.nicovideo.jp) が要る"]
async fn channel_video_list_pages_through_large_channels() {
    // 20 件を超える一覧の 2 ページ目が別の動画になること。
    let channel = std::env::var("NNDD_TEST_LARGE_CHANNEL").unwrap_or_else(|_| "2632720".into());
    let first = parse_channel_videos_html(
        &get_text(&channel_videos_url(
            "https://ch.nicovideo.jp",
            &channel,
            1,
            "registeredAt",
            "desc",
        ))
        .await,
    );
    println!(
        "[ch{channel}] total={} page1={}",
        first.total_count,
        first.items.len()
    );
    if first.total_count <= first.items.len() as i64 {
        eprintln!("ch{channel} has a single page; skipping the paging check");
        return;
    }
    let second = parse_channel_videos_html(
        &get_text(&channel_videos_url(
            "https://ch.nicovideo.jp",
            &channel,
            2,
            "registeredAt",
            "desc",
        ))
        .await,
    );
    assert!(!second.items.is_empty());
    assert!(second
        .items
        .iter()
        .all(|v| first.items.iter().all(|f| f.content_id != v.content_id)));
}

#[tokio::test]
#[ignore = "外部ネットワーク (ext.nicovideo.jp) が要る"]
async fn thumbinfo_tags_carry_lock_state() {
    // アプリと同じ Accept で取る (XML だけを受け付けると 406 になる)。
    let xml = get_text_accepting(
        "https://ext.nicovideo.jp/api/getthumbinfo/sm9",
        THUMBINFO_ACCEPT,
    )
    .await;
    let tags = parse_thumbinfo_tags(&xml).expect("ok response");
    println!("[sm9] {} tags", tags.len());
    assert!(tags.iter().any(|t| t.name == "陰陽師" && t.is_locked));
    assert!(tags.iter().any(|t| !t.is_locked));
    let deleted = get_text_accepting(
        "https://ext.nicovideo.jp/api/getthumbinfo/sm1",
        THUMBINFO_ACCEPT,
    )
    .await;
    assert_eq!(parse_thumbinfo_tags(&deleted), None);
}

#[tokio::test]
#[ignore = "外部ネットワーク + NNDD_TEST_COOKIE が要る"]
async fn account_state_tells_valid_and_expired_sessions_apart() {
    let bogus = SessionStore::empty();
    bogus.set(
        "user_session_1_0000000000000000000000000000000000000000000000000000000000000000".into(),
    );
    assert_eq!(fetch_account_state(&bogus).await, AccountState::Expired);

    let Ok(raw) = std::env::var("NNDD_TEST_COOKIE") else {
        eprintln!("NNDD_TEST_COOKIE 未設定のためスキップ");
        return;
    };
    let store = SessionStore::empty();
    store.set(raw);
    match fetch_account_state(&store).await {
        AccountState::Valid { account } => {
            println!(
                "[users/me] premium={} nickname_len={}",
                account.is_premium,
                account.nickname.chars().count()
            );
            assert!(!account.id.is_empty());
        }
        other => panic!("expected a valid session, got {other:?}"),
    }
}
