//! Versioned migrations for the library SQLite database.
//!
//! Migrations are append-only; never edit a shipped migration. To change
//! schema, add `m{NNN}_*.sql` and a matching entry to [`MIGRATIONS`].

use rusqlite::{params, Connection};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::LibraryError;

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

/// All migrations in order. Append-only.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("schema/m001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "video_resolution",
        sql: include_str!("schema/m002_video_resolution.sql"),
    },
];

/// Apply pending migrations. Idempotent — safe to call on every startup.
pub fn run_migrations(conn: &mut Connection) -> Result<(), LibraryError> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    ensure_version_table(conn)?;
    let current = current_version(conn)?;

    for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)
            .map_err(|e| LibraryError::Migration {
                version: migration.version,
                message: e.to_string(),
            })?;
        tx.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
            params![migration.version, now_unix()?],
        )?;
        tx.commit()?;
        tracing::info!(
            version = migration.version,
            name = migration.name,
            "applied migration"
        );
    }

    Ok(())
}

fn ensure_version_table(conn: &Connection) -> Result<(), LibraryError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version    INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
         );",
    )?;
    Ok(())
}

fn current_version(conn: &Connection) -> Result<u32, LibraryError> {
    let v: Option<u32> = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .ok()
        .flatten();
    Ok(v.unwrap_or(0))
}

fn now_unix() -> Result<i64, LibraryError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .map_err(|e| LibraryError::Integrity(format!("system clock before unix epoch: {e}")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn open_memory() -> Connection {
        Connection::open_in_memory().expect("in-memory sqlite must open")
    }

    #[test]
    fn migrates_empty_db_to_latest() {
        let mut conn = open_memory();
        run_migrations(&mut conn).expect("migrations apply");

        let v: u32 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .expect("version recorded");
        assert_eq!(v, MIGRATIONS.last().expect("≥1 migration").version);
    }

    #[test]
    fn second_run_is_noop() {
        let mut conn = open_memory();
        run_migrations(&mut conn).expect("first run");
        let count_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .expect("count");

        run_migrations(&mut conn).expect("second run");
        let count_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .expect("count");

        assert_eq!(count_before, count_after);
    }

    #[test]
    fn all_expected_tables_exist() {
        let mut conn = open_memory();
        run_migrations(&mut conn).expect("migrate");

        let names = [
            "videos",
            "tags",
            "comment_snapshots",
            "comments",
            "comments_fts",
            "playlists",
            "playlist_items",
            "play_history",
            "ng_rules",
            "download_queue",
            "settings",
            "schema_version",
        ];
        for name in names {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
                    [name],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            assert!(exists > 0, "table {name} should exist");
        }
    }
}
