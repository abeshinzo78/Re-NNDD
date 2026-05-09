//! Integration tests for `library/schema` against an on-disk SQLite file.
//! Mirrors `docs/test-lists/phase-1-0-schema.md`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use nndd_next_lib::library::schema::run_migrations;
use rusqlite::{params, Connection};
use tempfile::TempDir;

fn open_temp_db() -> (TempDir, Connection) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("library.db");
    let conn = Connection::open(&path).expect("open sqlite");
    (dir, conn)
}

#[test]
fn migration_enables_wal_and_foreign_keys() {
    let (_dir, mut conn) = open_temp_db();
    run_migrations(&mut conn).expect("migrate");

    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .expect("journal_mode");
    assert_eq!(journal_mode.to_lowercase(), "wal");

    let fk: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .expect("foreign_keys");
    assert_eq!(fk, 1);
}

#[test]
fn fts_trigger_indexes_inserted_comments() {
    let (_dir, mut conn) = open_temp_db();
    run_migrations(&mut conn).expect("migrate");

    conn.execute(
        "INSERT INTO videos (id, title, duration_sec) VALUES ('sm1', 'test', 1)",
        [],
    )
    .expect("insert video");
    conn.execute(
        "INSERT INTO comment_snapshots (video_id, taken_at, is_initial) VALUES ('sm1', 0, 1)",
        [],
    )
    .expect("insert snapshot");
    let snap_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO comments (snapshot_id, no, vpos_ms, content) VALUES (?1, 1, 0, '弾幕薄いよ何やってんの')",
        params![snap_id],
    )
    .expect("insert comment");

    // FTS5 trigram tokenizer requires queries ≥ 3 characters.
    let hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM comments_fts WHERE comments_fts MATCH '弾幕薄'",
            [],
            |r| r.get(0),
        )
        .expect("fts query");
    assert_eq!(hits, 1, "FTS index should pick up the inserted comment");
}

#[test]
fn cascade_deletes_dependent_rows() {
    let (_dir, mut conn) = open_temp_db();
    run_migrations(&mut conn).expect("migrate");

    conn.execute_batch(
        "INSERT INTO videos (id, title, duration_sec) VALUES ('sm2', 'a', 1);
         INSERT INTO tags (video_id, name) VALUES ('sm2', 'タグ');
         INSERT INTO comment_snapshots (video_id, taken_at, is_initial) VALUES ('sm2', 0, 1);
         INSERT INTO play_history (video_id, played_at) VALUES ('sm2', 0);",
    )
    .expect("seed");

    conn.execute("DELETE FROM videos WHERE id = 'sm2'", [])
        .expect("delete video");

    let tag_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM tags WHERE video_id='sm2'", [], |r| {
            r.get(0)
        })
        .expect("tag count");
    let snap_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM comment_snapshots WHERE video_id='sm2'",
            [],
            |r| r.get(0),
        )
        .expect("snapshot count");
    let hist_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM play_history WHERE video_id='sm2'",
            [],
            |r| r.get(0),
        )
        .expect("history count");

    assert_eq!(tag_count, 0);
    assert_eq!(snap_count, 0);
    assert_eq!(hist_count, 0);
}

#[test]
fn idempotent_across_restarts() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("library.db");

    {
        let mut conn = Connection::open(&path).expect("open 1");
        run_migrations(&mut conn).expect("migrate 1");
    }
    {
        let mut conn = Connection::open(&path).expect("open 2");
        run_migrations(&mut conn).expect("migrate 2");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 1);
    }
}
