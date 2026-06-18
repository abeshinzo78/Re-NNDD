//! Snapshot Search API v2 client.
//!
//! Endpoint: `https://snapshot.search.nicovideo.jp/api/v2/snapshot/video/contents/search`
//! Spec: <https://site.nicovideo.jp/search-api-docs/snapshot>
//!
//! Note: API is non-commercial use only and requires a User-Agent header.

use async_trait::async_trait;
use reqwest::{header, StatusCode};
use serde_json::Value;
use url::Url;

use crate::api::types::{
    SearchField, SearchHit, SearchMeta, SearchQuery, SearchResponse, SearchTarget, SortDirection,
    SortSpec,
};
use crate::error::ApiError;

/// Hard cap from the API spec. Going over yields HTTP 400.
pub const MAX_OFFSET: u32 = 100_000;
pub const MAX_LIMIT: u32 = 100;
pub const MAX_CONTEXT_LEN: usize = 40;

const PRODUCTION_BASE: &str = "https://snapshot.search.nicovideo.jp";
const SEARCH_PATH: &str = "/api/v2/snapshot/video/contents/search";

/// nvapi (the API niconico's own web client uses) base + search path.
const NVAPI_BASE: &str = "https://nvapi.nicovideo.jp";
const NVAPI_SEARCH_PATH: &str = "/v2/search/video";
/// Niconico browser frontend signature. nvapi rejects requests without it.
const NV_FRONTEND_ID: &str = "6";
const NV_FRONTEND_VERSION: &str = "0";
/// nvapi is UA-sniffy; a `reqwest/…` UA gets empty arrays back.
const NV_BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

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

/// Production client for niconico's internal `nvapi /v2/search/video`.
///
/// This is the search endpoint the niconico web client calls. Compared with
/// the public Snapshot Search API it indexes more content (e.g. shorts),
/// exposes a native popularity sort (`hot`), and — when given the logged-in
/// `user_session` cookie — returns results that respect the viewer's account
/// (sensitive content visibility, etc.). It speaks the same [`SearchQuery`] /
/// [`SearchResponse`] vocabulary so callers can swap it for
/// [`SnapshotSearchClient`] transparently; the snapshot-specific notions
/// (`targets`, `_context`) are mapped or ignored as documented below.
pub struct NvapiSearchClient {
    http: reqwest::Client,
    base_url: Url,
    /// Optional `Cookie:` header value (e.g. `user_session=…`). When set, the
    /// search is performed as the logged-in user.
    cookie: Option<String>,
}

impl NvapiSearchClient {
    /// Construct a client pointed at the production endpoint. `cookie` is the
    /// `Cookie:` header value to attach (typically `SessionStore::cookie_header`).
    pub fn new(cookie: Option<String>) -> Result<Self, ApiError> {
        Self::with_base_url(NVAPI_BASE, cookie)
    }

    /// Construct a client with an explicit base URL — used by tests against
    /// `mockito`.
    pub fn with_base_url(base_url: &str, cookie: Option<String>) -> Result<Self, ApiError> {
        let base_url = Url::parse(base_url)?;
        let http = reqwest::Client::builder()
            .user_agent(NV_BROWSER_UA)
            .gzip(true)
            .build()?;
        Ok(Self {
            http,
            base_url,
            cookie,
        })
    }

    fn build_url(&self, query: &SearchQuery) -> Result<Url, ApiError> {
        let mut url = self.base_url.join(NVAPI_SEARCH_PATH)?;
        // `validate_nvapi` guarantees `limit >= 1`, so this never divides by
        // zero. nvapi is page-based (1-origin) while the snapshot query is
        // offset-based, so translate.
        let page = query.offset / query.limit + 1;
        let (sort_key, sort_order) = map_sort(query.sort.as_ref());
        {
            let mut pairs = url.query_pairs_mut();
            // The snapshot `targets` concept (title/description/tags) has no
            // direct nvapi analogue: `keyword` already matches across
            // title/tags/description. The one meaningful mapping is an
            // exact-tag search. The form's default target is `title`, so key
            // off *presence* of `tagsExact` (not it being the sole target) —
            // otherwise toggling 「タグ完全一致」 on top of the default would
            // silently fall through to a broad keyword search.
            if query.targets.contains(&SearchTarget::TagsExact) {
                pairs.append_pair("tag", &query.q);
            } else {
                pairs.append_pair("keyword", &query.q);
            }
            pairs.append_pair("sortKey", sort_key);
            pairs.append_pair("sortOrder", sort_order);
            pairs.append_pair("pageSize", &query.limit.to_string());
            pairs.append_pair("page", &page.to_string());
            // Mirror the web client: surface sensitive content as masked
            // rather than dropping it from the result set.
            pairs.append_pair("sensitiveContents", "mask");
        }
        Ok(url)
    }
}

#[async_trait]
impl SearchApi for NvapiSearchClient {
    async fn search(&self, query: &SearchQuery) -> Result<SearchResponse, ApiError> {
        validate_nvapi(query)?;

        let url = self.build_url(query)?;
        tracing::debug!(url = %url, authed = self.cookie.is_some(), "nvapi search request");

        let mut request = self
            .http
            .get(url)
            .header("X-Frontend-Id", NV_FRONTEND_ID)
            .header("X-Frontend-Version", NV_FRONTEND_VERSION)
            .header(header::REFERER, "https://www.nicovideo.jp/")
            .header(header::ACCEPT, "application/json");
        if let Some(cookie) = &self.cookie {
            request = request.header(header::COOKIE, cookie);
        }

        let response = request.send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;

        match status {
            StatusCode::OK => parse_nvapi_response(&bytes),
            StatusCode::BAD_REQUEST => {
                let message = extract_nvapi_error(&bytes).unwrap_or_else(|| "bad request".into());
                Err(ApiError::QueryParseError(message))
            }
            StatusCode::TOO_MANY_REQUESTS => Err(ApiError::RateLimited),
            other => {
                let message = extract_nvapi_error(&bytes)
                    .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
                Err(ApiError::ServerError {
                    status: other.as_u16(),
                    message,
                })
            }
        }
    }
}

/// Map a snapshot [`SortSpec`] to nvapi's `(sortKey, sortOrder)` pair.
/// Unknown/absent sorts fall back to nvapi's native popularity (`hot`), which
/// requires `sortOrder=none`.
fn map_sort(sort: Option<&SortSpec>) -> (&'static str, &'static str) {
    let Some(spec) = sort else {
        return ("hot", "none");
    };
    let key = match spec.field {
        SearchField::ViewCounter => "viewCount",
        SearchField::CommentCounter => "commentCount",
        SearchField::MylistCounter => "mylistCount",
        SearchField::LikeCounter => "likeCount",
        SearchField::StartTime => "registeredAt",
        SearchField::LengthSeconds => "duration",
        SearchField::LastCommentTime => "lastCommentTime",
        // Anything else has no nvapi sort key — use the default popularity sort.
        _ => return ("hot", "none"),
    };
    let order = match spec.direction {
        SortDirection::Asc => "asc",
        SortDirection::Desc => "desc",
    };
    (key, order)
}

fn validate_nvapi(query: &SearchQuery) -> Result<(), ApiError> {
    if query.q.trim().is_empty() {
        return Err(ApiError::InvalidQuery("`q` must not be empty".into()));
    }
    if query.limit == 0 || query.limit > MAX_LIMIT {
        return Err(ApiError::InvalidQuery(format!(
            "`limit` must be in 1..={MAX_LIMIT} (was {})",
            query.limit
        )));
    }
    Ok(())
}

/// Project an nvapi `data.items[]` entry onto the snapshot [`SearchHit`] shape
/// so the rest of the app (and the frontend cards) need not care which engine
/// produced the row.
fn nvapi_item_to_hit(v: &Value) -> Option<SearchHit> {
    let content_id = v.get("id").and_then(Value::as_str).map(String::from);
    // Skip rows without a usable content id — they can't be played or DL'd.
    content_id.as_ref()?;

    let count = v.get("count");
    let count_of = |k: &str| count.and_then(|c| c.get(k)).and_then(Value::as_i64);

    let thumbnail_url = v
        .get("thumbnail")
        .and_then(|t| {
            // Prefer the stable `url`; `listingUrl` can be a signed URL that
            // expires (mirrors build_user_video_item / ranking ordering).
            t.get("url")
                .or_else(|| t.get("listingUrl"))
                .or_else(|| t.get("middleUrl"))
                .or_else(|| t.get("largeUrl"))
        })
        .and_then(Value::as_str)
        .map(String::from);

    let owner = v.get("owner");
    let owner_type = owner
        .and_then(|o| o.get("ownerType"))
        .and_then(Value::as_str);
    // owner.id comes back as a number or a numeric string depending on the
    // endpoint version — accept both.
    let owner_id = owner.and_then(|o| o.get("id")).and_then(|x| {
        x.as_i64()
            .or_else(|| x.as_str().and_then(|s| s.parse::<i64>().ok()))
    });
    let (user_id, channel_id) = match owner_type {
        Some("channel") => (None, owner_id),
        _ => (owner_id, None),
    };

    Some(SearchHit {
        content_id,
        title: v.get("title").and_then(Value::as_str).map(String::from),
        description: v
            .get("shortDescription")
            .and_then(Value::as_str)
            .map(String::from),
        user_id,
        channel_id,
        view_counter: count_of("view"),
        mylist_counter: count_of("mylist"),
        like_counter: count_of("like"),
        length_seconds: v.get("duration").and_then(Value::as_i64),
        thumbnail_url,
        start_time: v
            .get("registeredAt")
            .and_then(Value::as_str)
            .map(String::from),
        last_res_body: None,
        comment_counter: count_of("comment"),
        last_comment_time: None,
        category_tags: None,
        // nvapi search rows don't carry the tag list; leave it unset.
        tags: None,
        genre: None,
        content_type: v
            .get("contentType")
            .and_then(Value::as_str)
            .map(String::from),
    })
}

fn parse_nvapi_response(bytes: &[u8]) -> Result<SearchResponse, ApiError> {
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|e| ApiError::ResponseShape(format!("failed to parse nvapi body: {e}")))?;
    let data = root
        .get("data")
        .ok_or_else(|| ApiError::ResponseShape("nvapi response missing `data`".into()))?;
    let total_count = data.get("totalCount").and_then(Value::as_u64);
    let items = data.get("items").and_then(Value::as_array);
    let hits: Vec<SearchHit> = items
        .map(|arr| arr.iter().filter_map(nvapi_item_to_hit).collect())
        .unwrap_or_default();
    let id = root
        .pointer("/meta/id")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| "nvapi".to_string());

    Ok(SearchResponse {
        meta: SearchMeta {
            status: StatusCode::OK.as_u16(),
            total_count,
            id,
            error_code: None,
            error_message: None,
        },
        data: hits,
    })
}

fn extract_nvapi_error(body: &[u8]) -> Option<String> {
    let parsed: Value = serde_json::from_slice(body).ok()?;
    parsed
        .pointer("/meta/errorCode")
        .and_then(Value::as_str)
        .map(String::from)
        .filter(|s| !s.is_empty())
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

    #[test]
    fn nvapi_validate_rejects_blank_query() {
        let mut q = baseline();
        q.q = "   ".into();
        assert!(matches!(validate_nvapi(&q), Err(ApiError::InvalidQuery(_))));
    }

    #[test]
    fn nvapi_validate_rejects_oversized_limit() {
        let mut q = baseline();
        q.limit = MAX_LIMIT + 1;
        assert!(matches!(validate_nvapi(&q), Err(ApiError::InvalidQuery(_))));
    }

    #[test]
    fn nvapi_map_sort_defaults_to_hot() {
        assert_eq!(map_sort(None), ("hot", "none"));
        // A field with no nvapi key also falls back to hot.
        assert_eq!(
            map_sort(Some(&SortSpec {
                field: SearchField::Genre,
                direction: SortDirection::Desc,
            })),
            ("hot", "none")
        );
    }

    #[test]
    fn nvapi_map_sort_translates_known_fields() {
        assert_eq!(
            map_sort(Some(&SortSpec {
                field: SearchField::ViewCounter,
                direction: SortDirection::Desc,
            })),
            ("viewCount", "desc")
        );
        assert_eq!(
            map_sort(Some(&SortSpec {
                field: SearchField::StartTime,
                direction: SortDirection::Asc,
            })),
            ("registeredAt", "asc")
        );
    }

    #[test]
    fn nvapi_build_url_uses_keyword_and_paging() {
        let client = NvapiSearchClient::with_base_url("https://example.test", None).unwrap();
        let mut q = baseline();
        q.limit = 20;
        q.offset = 40; // page 3 at pageSize 20
        q.sort = Some(SortSpec {
            field: SearchField::ViewCounter,
            direction: SortDirection::Desc,
        });
        let url = client.build_url(&q).unwrap();
        let qs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(url.path(), "/v2/search/video");
        assert_eq!(qs.get("keyword").map(String::as_str), Some("ゆっくり"));
        assert_eq!(qs.get("sortKey").map(String::as_str), Some("viewCount"));
        assert_eq!(qs.get("sortOrder").map(String::as_str), Some("desc"));
        assert_eq!(qs.get("pageSize").map(String::as_str), Some("20"));
        assert_eq!(qs.get("page").map(String::as_str), Some("3"));
        assert!(!qs.contains_key("tag"));
    }

    #[test]
    fn nvapi_build_url_uses_tag_when_exact_tag_present() {
        let client = NvapiSearchClient::with_base_url("https://example.test", None).unwrap();
        // The default form target is `title`; toggling tagsExact on top of it
        // must still produce an exact-tag search, not a broad keyword search.
        let mut q = baseline();
        q.targets = vec![SearchTarget::Title, SearchTarget::TagsExact];
        let url = client.build_url(&q).unwrap();
        let qs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(qs.get("tag").map(String::as_str), Some("ゆっくり"));
        assert!(!qs.contains_key("keyword"));
    }

    #[test]
    fn nvapi_item_to_hit_maps_fields() {
        let item = serde_json::json!({
            "id": "sm9",
            "title": "Example",
            "registeredAt": "2007-03-06T00:33:00+09:00",
            "duration": 320,
            "shortDescription": "説明",
            "count": {"view": 1000, "comment": 50, "mylist": 5, "like": 9},
            "thumbnail": {"url": "https://example.test/thumb.jpg"},
            "owner": {"ownerType": "user", "id": "12345", "name": "投稿者"}
        });
        let hit = nvapi_item_to_hit(&item).expect("maps");
        assert_eq!(hit.content_id.as_deref(), Some("sm9"));
        assert_eq!(hit.title.as_deref(), Some("Example"));
        assert_eq!(hit.view_counter, Some(1000));
        assert_eq!(hit.comment_counter, Some(50));
        assert_eq!(hit.mylist_counter, Some(5));
        assert_eq!(hit.like_counter, Some(9));
        assert_eq!(hit.length_seconds, Some(320));
        assert_eq!(hit.user_id, Some(12345));
        assert_eq!(hit.channel_id, None);
        assert_eq!(
            hit.thumbnail_url.as_deref(),
            Some("https://example.test/thumb.jpg")
        );
        assert_eq!(hit.start_time.as_deref(), Some("2007-03-06T00:33:00+09:00"));
    }

    #[test]
    fn nvapi_item_to_hit_routes_channel_owner_to_channel_id() {
        let item = serde_json::json!({
            "id": "so123",
            "title": "Channel video",
            "owner": {"ownerType": "channel", "id": 67890}
        });
        let hit = nvapi_item_to_hit(&item).expect("maps");
        assert_eq!(hit.channel_id, Some(67890));
        assert_eq!(hit.user_id, None);
    }

    #[test]
    fn nvapi_item_to_hit_skips_rows_without_id() {
        let item = serde_json::json!({ "title": "no id" });
        assert!(nvapi_item_to_hit(&item).is_none());
    }
}
