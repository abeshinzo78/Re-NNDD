//! Integration tests for the Snapshot Search API client. The real endpoint
//! is replaced with a `mockito` server.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use mockito::Matcher;
use nndd_next_lib::api::search::{SearchApi, SnapshotSearchClient};
use nndd_next_lib::api::types::{SearchQuery, SearchTarget};
use nndd_next_lib::error::ApiError;

const SEARCH_PATH: &str = "/api/v2/snapshot/video/contents/search";

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
