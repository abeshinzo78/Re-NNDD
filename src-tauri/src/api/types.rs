//! DTOs for the niconico Snapshot Search API v2.
//!
//! Specification: <https://site.nicovideo.jp/search-api-docs/snapshot>
//!
//! These are wire-format types. Persistence-domain types live under
//! `library/`; conversion happens at the boundary (not yet implemented in
//! Phase 1.0).

use serde::{Deserialize, Serialize};

/// Snapshot Search v2 request.
///
/// Validation (offset ≤ 100_000, limit ≤ 100, q non-empty, targets non-empty)
/// is enforced by [`crate::api::search::SnapshotSearchClient::search`] before
/// the HTTP call to keep the API server from rejecting predictable mistakes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub targets: Vec<SearchTarget>,
    #[serde(default)]
    pub fields: Vec<SearchField>,
    #[serde(default)]
    pub filters: Vec<FilterClause>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<SortSpec>,
    #[serde(default)]
    pub offset: u32,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default = "default_context")]
    pub context: String,
}

fn default_limit() -> u32 {
    10
}

fn default_context() -> String {
    format!("Re:NNDD/{}", env!("CARGO_PKG_VERSION"))
}

impl SearchQuery {
    pub fn new(q: impl Into<String>, targets: Vec<SearchTarget>) -> Self {
        Self {
            q: q.into(),
            targets,
            fields: Vec::new(),
            filters: Vec::new(),
            sort: None,
            offset: 0,
            limit: default_limit(),
            context: default_context(),
        }
    }
}

/// Search target field. `targets=` parameter values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchTarget {
    Title,
    Description,
    Tags,
    TagsExact,
}

impl SearchTarget {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Description => "description",
            Self::Tags => "tags",
            Self::TagsExact => "tagsExact",
        }
    }
}

/// Selectable / sortable / filterable response field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchField {
    ContentId,
    Title,
    Description,
    UserId,
    ChannelId,
    ViewCounter,
    MylistCounter,
    LikeCounter,
    LengthSeconds,
    ThumbnailUrl,
    StartTime,
    LastResBody,
    CommentCounter,
    LastCommentTime,
    CategoryTags,
    Tags,
    TagsExact,
    Genre,
    GenreKeyword,
    ContentType,
}

impl SearchField {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ContentId => "contentId",
            Self::Title => "title",
            Self::Description => "description",
            Self::UserId => "userId",
            Self::ChannelId => "channelId",
            Self::ViewCounter => "viewCounter",
            Self::MylistCounter => "mylistCounter",
            Self::LikeCounter => "likeCounter",
            Self::LengthSeconds => "lengthSeconds",
            Self::ThumbnailUrl => "thumbnailUrl",
            Self::StartTime => "startTime",
            Self::LastResBody => "lastResBody",
            Self::CommentCounter => "commentCounter",
            Self::LastCommentTime => "lastCommentTime",
            Self::CategoryTags => "categoryTags",
            Self::Tags => "tags",
            Self::TagsExact => "tagsExact",
            Self::Genre => "genre",
            Self::GenreKeyword => "genre.keyword",
            Self::ContentType => "contentType",
        }
    }
}

/// Filter operator. Maps to `filters[field][op]=value` form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterOp {
    /// Exact match. Encodes as `filters[field][0]=value`.
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
}

impl FilterOp {
    /// Index/key fragment used inside `filters[field][...]`.
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Eq => "0",
            Self::Gt => "gt",
            Self::Gte => "gte",
            Self::Lt => "lt",
            Self::Lte => "lte",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterClause {
    pub field: SearchField,
    pub op: FilterOp,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortSpec {
    pub field: SearchField,
    pub direction: SortDirection,
}

impl SortSpec {
    /// Wire format: `+fieldName` (asc) or `-fieldName` (desc).
    pub fn to_param(&self) -> String {
        let prefix = match self.direction {
            SortDirection::Asc => '+',
            SortDirection::Desc => '-',
        };
        format!("{prefix}{}", self.field.as_str())
    }
}

/// Snapshot Search v2 response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub meta: SearchMeta,
    pub data: Vec<SearchHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMeta {
    pub status: u16,
    #[serde(
        default,
        rename = "totalCount",
        skip_serializing_if = "Option::is_none"
    )]
    pub total_count: Option<u64>,
    pub id: String,
    /// Present on error responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// One row from the `data` array. Fields are all optional — the server only
/// returns those requested via `fields=`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_counter: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mylist_counter: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub like_counter: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_res_body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_counter: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_comment_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_tags: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}
