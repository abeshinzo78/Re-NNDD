//! Adapters for the niconico API.
//!
//! Per CLAUDE.md, the API is volatile — keep all HTTP details inside this
//! module so that the rest of the app talks to stable trait abstractions.

pub mod account;
pub mod auth;
pub mod channel;
pub mod comment;
pub mod search;
pub mod thumbinfo;
pub mod types;
pub mod video;
