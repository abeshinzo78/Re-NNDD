//! Application error types.
//!
//! Layering:
//! - [`LibraryError`]: persistence layer (rusqlite, migrations, FTS).
//! - [`ApiError`]: niconico API adapters (transport, parsing, rate limits).
//! - [`AppError`]: surfaced to Tauri command handlers — `Serialize` so it
//!   reaches the frontend as JSON. Built from the layered errors via `From`.

use serde::{Serialize, Serializer};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("migration {version} failed: {message}")]
    Migration { version: u32, message: String },

    #[error("schema integrity violation: {0}")]
    Integrity(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("invalid query: {0}")]
    InvalidQuery(String),

    #[error("query parse error from server (HTTP 400): {0}")]
    QueryParseError(String),

    #[error("rate limited (HTTP 429)")]
    RateLimited,

    #[error("server error (HTTP {status}): {message}")]
    ServerError { status: u16, message: String },

    #[error("unexpected response shape: {0}")]
    ResponseShape(String),

    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("url build error: {0}")]
    UrlBuild(#[from] url::ParseError),

    #[error("json serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("downloader error: {0}")]
    Downloader(String),
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Library(#[from] LibraryError),

    #[error(transparent)]
    Api(#[from] ApiError),

    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;
