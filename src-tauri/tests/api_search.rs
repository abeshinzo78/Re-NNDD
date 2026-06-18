//! Integration tests for the Snapshot Search API client. The real endpoint
//! is replaced with a `mockito` server.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mockito::Matcher;
use nndd_next_lib::api::search::{NvapiSearchClient, SearchApi, SnapshotSearchClient};
use nndd_next_lib::api::types::{SearchField, SearchQuery, SearchTarget, SortDirection, SortSpec};
use nndd_next_lib::error::ApiError;

const SEARCH_PATH: &str = "/api/v2/snapshot/video/contents/search";
const NVAPI_SEARCH_PATH: &str = "/v2/search/video";

fn ok_body() -> &'static str {
    r#"{
        "meta": {"status": 200, "totalCount": 1, "id": "test-id"},
        "data": [{"contentId": "sm9", "title": "Example", "viewCounter": 1000}]
    }"#
}

#[tokio::test]
async fn parses_successful_response() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", SEARCH_PATH)
        .match_query(Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(ok_body())
        .create_async()
        .await;

    let client = SnapshotSearchClient::with_base_url(&server.url(), "ua/test").unwrap();
    let response = client
        .search(&SearchQuery::new("テスト", vec![SearchTarget::Title]))
        .await
        .expect("search ok");

    assert_eq!(response.meta.status, 200);
    assert_eq!(response.meta.total_count, Some(1));
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.data[0].content_id.as_deref(), Some("sm9"));
    mock.assert_async().await;
}

#[tokio::test]
async fn sends_required_query_params() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", SEARCH_PATH)
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("q".into(), "ゆっくり".into()),
            Matcher::UrlEncoded("targets".into(), "title".into()),
            Matcher::UrlEncoded("_limit".into(), "10".into()),
            Matcher::UrlEncoded("_offset".into(), "0".into()),
        ]))
        .with_status(200)
        .with_body(ok_body())
        .create_async()
        .await;

    let client = SnapshotSearchClient::with_base_url(&server.url(), "ua/test").unwrap();
    client
        .search(&SearchQuery::new("ゆっくり", vec![SearchTarget::Title]))
        .await
        .expect("search ok");
    mock.assert_async().await;
}

#[tokio::test]
async fn maps_400_to_query_parse_error() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", SEARCH_PATH)
        .match_query(Matcher::Any)
        .with_status(400)
        .with_body(r#"{"meta":{"status":400,"id":"x","errorCode":"QUERY_PARSE_ERROR","errorMessage":"bad"}}"#)
        .create_async()
        .await;

    let client = SnapshotSearchClient::with_base_url(&server.url(), "ua/test").unwrap();
    let err = client
        .search(&SearchQuery::new("oops", vec![SearchTarget::Title]))
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::QueryParseError(_)), "got {err:?}");
}

#[tokio::test]
async fn maps_503_to_server_error() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", SEARCH_PATH)
        .match_query(Matcher::Any)
        .with_status(503)
        .with_body(r#"{"meta":{"status":503,"id":"x","errorCode":"MAINTENANCE"}}"#)
        .create_async()
        .await;

    let client = SnapshotSearchClient::with_base_url(&server.url(), "ua/test").unwrap();
    let err = client
        .search(&SearchQuery::new("x", vec![SearchTarget::Title]))
        .await
        .unwrap_err();
    match err {
        ApiError::ServerError { status, .. } => assert_eq!(status, 503),
        other => panic!("expected ServerError, got {other:?}"),
    }
}

#[tokio::test]
async fn maps_429_to_rate_limited() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", SEARCH_PATH)
        .match_query(Matcher::Any)
        .with_status(429)
        .with_body("")
        .create_async()
        .await;

    let client = SnapshotSearchClient::with_base_url(&server.url(), "ua/test").unwrap();
    let err = client
        .search(&SearchQuery::new("x", vec![SearchTarget::Title]))
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::RateLimited));
}

#[tokio::test]
async fn validates_before_calling_server() {
    // No mock registered — if validation fails to short-circuit, the call
    // would error with Transport (connection refused) on a freshly bound URL.
    let client = SnapshotSearchClient::with_base_url("http://127.0.0.1:1", "ua/test").unwrap();
    let mut q = SearchQuery::new("x", vec![SearchTarget::Title]);
    q.limit = 200;
    let err = client.search(&q).await.unwrap_err();
    assert!(matches!(err, ApiError::InvalidQuery(_)), "got {err:?}");
}

// ===================== nvapi search client =====================

fn nvapi_ok_body() -> &'static str {
    r#"{
        "meta": {"status": 200},
        "data": {
            "totalCount": 42,
            "hasNext": true,
            "items": [
                {
                    "id": "sm9",
                    "title": "Example",
                    "registeredAt": "2007-03-06T00:33:00+09:00",
                    "duration": 320,
                    "count": {"view": 1000, "comment": 50, "mylist": 5, "like": 9},
                    "thumbnail": {"url": "https://example.test/thumb.jpg"},
                    "owner": {"ownerType": "user", "id": "12345", "name": "投稿者"}
                }
            ]
        }
    }"#
}

#[tokio::test]
async fn nvapi_parses_and_maps_response() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", NVAPI_SEARCH_PATH)
        .match_query(Matcher::Any)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(nvapi_ok_body())
        .create_async()
        .await;

    let client = NvapiSearchClient::with_base_url(&server.url(), None).unwrap();
    let response = client
        .search(&SearchQuery::new("テスト", vec![SearchTarget::Title]))
        .await
        .expect("search ok");

    assert_eq!(response.meta.status, 200);
    assert_eq!(response.meta.total_count, Some(42));
    assert_eq!(response.data.len(), 1);
    let hit = &response.data[0];
    assert_eq!(hit.content_id.as_deref(), Some("sm9"));
    assert_eq!(hit.view_counter, Some(1000));
    assert_eq!(hit.like_counter, Some(9));
    assert_eq!(hit.length_seconds, Some(320));
    assert_eq!(hit.user_id, Some(12345));
    assert_eq!(
        hit.thumbnail_url.as_deref(),
        Some("https://example.test/thumb.jpg")
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn nvapi_sends_keyword_sort_and_cookie() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", NVAPI_SEARCH_PATH)
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("keyword".into(), "ゆっくり".into()),
            Matcher::UrlEncoded("sortKey".into(), "viewCount".into()),
            Matcher::UrlEncoded("sortOrder".into(), "desc".into()),
            Matcher::UrlEncoded("pageSize".into(), "20".into()),
            Matcher::UrlEncoded("page".into(), "1".into()),
        ]))
        .match_header("cookie", "user_session=abc")
        .match_header("x-frontend-id", "6")
        .with_status(200)
        .with_body(nvapi_ok_body())
        .create_async()
        .await;

    let client =
        NvapiSearchClient::with_base_url(&server.url(), Some("user_session=abc".into())).unwrap();
    let mut q = SearchQuery::new("ゆっくり", vec![SearchTarget::Title]);
    q.limit = 20;
    q.sort = Some(SortSpec {
        field: SearchField::ViewCounter,
        direction: SortDirection::Desc,
    });
    client.search(&q).await.expect("search ok");
    mock.assert_async().await;
}

#[tokio::test]
async fn nvapi_maps_400_to_query_parse_error() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", NVAPI_SEARCH_PATH)
        .match_query(Matcher::Any)
        .with_status(400)
        .with_body(r#"{"meta":{"status":400,"errorCode":"INVALID_PARAMETER"}}"#)
        .create_async()
        .await;

    let client = NvapiSearchClient::with_base_url(&server.url(), None).unwrap();
    let err = client
        .search(&SearchQuery::new("oops", vec![SearchTarget::Title]))
        .await
        .unwrap_err();
    match err {
        ApiError::QueryParseError(msg) => assert_eq!(msg, "INVALID_PARAMETER"),
        other => panic!("expected QueryParseError, got {other:?}"),
    }
}
