//! Snapshot Search API v2 client.
//!
//! Endpoint: `https://snapshot.search.nicovideo.jp/api/v2/snapshot/video/contents/search`
//! Spec: <https://site.nicovideo.jp/search-api-docs/snapshot>
//!
//! Note: API is non-commercial use only and requires a User-Agent header.

use async_trait::async_trait;
use reqwest::StatusCode;
use url::Url;

use crate::api::types::{SearchQuery, SearchResponse};
use crate::error::ApiError;

/// Hard cap from the API spec. Going over yields HTTP 400.
pub const MAX_OFFSET: u32 = 100_000;
pub const MAX_LIMIT: u32 = 100;
pub const MAX_CONTEXT_LEN: usize = 40;

const PRODUCTION_BASE: &str = "https://snapshot.search.nicovideo.jp";
const SEARCH_PATH: &str = "/api/v2/snapshot/video/contents/search";

#[async_trait]
pub trait SearchApi: Send + Sync {
    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, ApiError>;
}

/// Production client for the niconico Snapshot Search API.
pub struct SnapshotSearchClient {
    http: reqwest::Client,
    base_url: Url,
    user_agent: String,
}

impl SnapshotSearchClient {
    /// Construct a client pointed at the production endpoint with default UA.
    pub fn new() -> Result<Self, ApiError> {
        let user_agent = default_user_agent();
        Self::with_base_url(PRODUCTION_BASE, &user_agent)
    }

    /// Construct a client with an explicit base URL — used by tests against
    /// `mockito` and by anyone routing through a local proxy.
    pub fn with_base_url(base_url: &str, user_agent: &str) -> Result<Self, ApiError> {
        let base_url = Url::parse(base_url)?;
        let http = reqwest::Client::builder().user_agent(user_agent).build()?;
        Ok(Self {
            http,
            base_url,
            user_agent: user_agent.to_string(),
        })
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    fn build_url(&self, query: &SearchQuery) -> Result<Url, ApiError> {
        let mut url = self.base_url.join(SEARCH_PATH)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", &query.q);
            pairs.append_pair(
                "targets",
                &query
                    .targets
                    .iter()
                    .map(|t| t.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            );
            if !query.fields.is_empty() {
                pairs.append_pair(
                    "fields",
                    &query
                        .fields
                        .iter()
                        .map(|f| f.as_str())
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
            for clause in &query.filters {
                let key = format!("filters[{}][{}]", clause.field.as_str(), clause.op.as_key());
                pairs.append_pair(&key, &clause.value);
            }
            if let Some(sort) = &query.sort {
                pairs.append_pair("_sort", &sort.to_param());
            }
            pairs.append_pair("_offset", &query.offset.to_string());
            pairs.append_pair("_limit", &query.limit.to_string());
            pairs.append_pair("_context", &query.context);
        }
        Ok(url)
    }
}

#[async_trait]
impl SearchApi for SnapshotSearchClient {
    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, ApiError> {
        validate(query)?;

        let url = self.build_url(query)?;
        tracing::debug!(url = %url, "snapshot search request");

        let response = self.http.get(url).send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;

        match status {
            StatusCode::OK => {
                let body: SearchResponse = serde_json::from_slice(&bytes).map_err(|e| {
                    ApiError::ResponseShape(format!("failed to parse 200 body: {e}"))
                })?;
                Ok(body)
            }
            StatusCode::BAD_REQUEST => {
                let message = extract_error_message(&bytes).unwrap_or_else(|| "bad request".into());
                Err(ApiError::QueryParseError(message))
            }
            StatusCode::TOO_MANY_REQUESTS => Err(ApiError::RateLimited),
            other => {
                let message = extract_error_message(&bytes)
                    .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
                Err(ApiError::ServerError {
                    status: other.as_u16(),
                    message,
                })
            }
        }
    }
}

fn validate(query: &SearchQuery) -> Result<(), ApiError> {
    if query.q.is_empty() {
        return Err(ApiError::InvalidQuery("`q` must not be empty".into()));
    }
    if query.targets.is_empty() {
        return Err(ApiError::InvalidQuery("`targets` must not be empty".into()));
    }
    if query.offset > MAX_OFFSET {
        return Err(ApiError::InvalidQuery(format!(
            "`offset` must be ≤ {MAX_OFFSET} (was {})",
            query.offset
        )));
    }
    if query.limit == 0 || query.limit > MAX_LIMIT {
        return Err(ApiError::InvalidQuery(format!(
            "`limit` must be in 1..={MAX_LIMIT} (was {})",
            query.limit
        )));
    }
    if query.context.is_empty() || query.context.chars().count() > MAX_CONTEXT_LEN {
        return Err(ApiError::InvalidQuery(format!(
            "`context` must be 1..={MAX_CONTEXT_LEN} characters"
        )));
    }
    Ok(())
}

fn extract_error_message(body: &[u8]) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Wrapper {
        meta: Meta,
    }
    #[derive(serde::Deserialize)]
    struct Meta {
        #[serde(rename = "errorMessage")]
        error_message: Option<String>,
        #[serde(rename = "errorCode")]
        error_code: Option<String>,
    }
    let parsed: Wrapper = serde_json::from_slice(body).ok()?;
    parsed
        .meta
        .error_message
        .or(parsed.meta.error_code)
        .filter(|s| !s.is_empty())
}

fn default_user_agent() -> String {
    format!(
        "Re:NNDD/{} (+https://github.com/abeshinzo78/Re-NNDD)",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::api::types::{
        FilterClause, FilterOp, SearchField, SearchTarget, SortDirection, SortSpec,
    };

    fn baseline() -> SearchQuery {
        SearchQuery::new("ゆっくり", vec![SearchTarget::Title])
    }

    #[test]
    fn validate_rejects_empty_query() {
        let mut q = baseline();
        q.q = String::new();
        assert!(matches!(validate(&q), Err(ApiError::InvalidQuery(_))));
    }

    #[test]
    fn validate_rejects_empty_targets() {
        let mut q = baseline();
        q.targets.clear();
        assert!(matches!(validate(&q), Err(ApiError::InvalidQuery(_))));
    }

    #[test]
    fn validate_rejects_oversized_limit() {
        let mut q = baseline();
        q.limit = 101;
        assert!(matches!(validate(&q), Err(ApiError::InvalidQuery(_))));
    }

    #[test]
    fn validate_rejects_oversized_offset() {
        let mut q = baseline();
        q.offset = 100_001;
        assert!(matches!(validate(&q), Err(ApiError::InvalidQuery(_))));
    }

    #[test]
    fn validate_rejects_long_context() {
        let mut q = baseline();
        q.context = "a".repeat(MAX_CONTEXT_LEN + 1);
        assert!(matches!(validate(&q), Err(ApiError::InvalidQuery(_))));
    }

    #[test]
    fn build_url_includes_required_pairs() {
        let client = SnapshotSearchClient::with_base_url("https://example.test", "ua/0").unwrap();
        let q = baseline();
        let url = client.build_url(&q).unwrap();
        let qs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(qs.get("q").map(String::as_str), Some("ゆっくり"));
        assert_eq!(qs.get("targets").map(String::as_str), Some("title"));
        assert_eq!(qs.get("_offset").map(String::as_str), Some("0"));
        assert_eq!(qs.get("_limit").map(String::as_str), Some("10"));
        assert!(qs.contains_key("_context"));
    }

    #[test]
    fn build_url_encodes_filters_with_bracket_syntax() {
        let client = SnapshotSearchClient::with_base_url("https://example.test", "ua/0").unwrap();
        let mut q = baseline();
        q.filters.push(FilterClause {
            field: SearchField::ViewCounter,
            op: FilterOp::Gte,
            value: "1000".into(),
        });
        let url = client.build_url(&q).unwrap();
        let raw = url.as_str();
        // url crate percent-encodes brackets; assert on either form.
        assert!(
            raw.contains("filters[viewCounter][gte]=1000")
                || raw.contains("filters%5BviewCounter%5D%5Bgte%5D=1000"),
            "missing filters clause in {raw}"
        );
    }

    #[test]
    fn build_url_encodes_multi_targets_and_fields() {
        let client = SnapshotSearchClient::with_base_url("https://example.test", "ua/0").unwrap();
        let mut q = baseline();
        q.targets = vec![SearchTarget::Title, SearchTarget::Tags];
        q.fields = vec![SearchField::ContentId, SearchField::ViewCounter];
        let url = client.build_url(&q).unwrap();
        let qs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(qs.get("targets").map(String::as_str), Some("title,tags"));
        assert_eq!(
            qs.get("fields").map(String::as_str),
            Some("contentId,viewCounter")
        );
    }

    #[test]
    fn build_url_includes_sort_param() {
        let client = SnapshotSearchClient::with_base_url("https://example.test", "ua/0").unwrap();
        let mut q = baseline();
        q.sort = Some(SortSpec {
            field: SearchField::ViewCounter,
            direction: SortDirection::Desc,
        });
        let url = client.build_url(&q).unwrap();
        let qs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(qs.get("_sort").map(String::as_str), Some("-viewCounter"));
    }

    #[test]
    fn default_user_agent_starts_with_app_name() {
        assert!(default_user_agent().starts_with("Re:NNDD/"));
    }
}
