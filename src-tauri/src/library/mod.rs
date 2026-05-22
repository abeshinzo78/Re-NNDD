//! Persistence layer. Owns the SQLite database, migrations, and CRUD over
//! library entities (videos, tags, comments, NG rules, …).

pub mod db;
pub mod history;
pub mod playlists;
pub mod query;
pub mod queue;
pub mod schema;
pub mod settings;
pub mod snapshots;
pub mod videos;

/// 現在時刻を UNIX 秒で返す。クロックが UNIX_EPOCH より前にずれている (= 環境の
/// 時計設定異常) ケースは 0 にフォールバックする — `started_at` 等の単調性は
/// 失われるが「以前のレコードより新しい」という最低限の保証は守れる。
pub(crate) fn now_unix_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
