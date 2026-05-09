//! `settings` テーブルへの key-value アクセス。
//!
//! - 値は文字列で保存。型は呼び出し側 (フロント) でパースする。
//! - 既定値はフロント側 (`stores/settings.ts`) で定義。DB に未登録なら未指定として扱う。
//! - 全件取得は HashMap で返す。フロントは起動時に 1 回読んで in-memory にキャッシュ。

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::LibraryError;

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>, LibraryError> {
    let v = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(v)
}

pub fn set(conn: &Connection, key: &str, value: &str) -> Result<(), LibraryError> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, key: &str) -> Result<usize, LibraryError> {
    Ok(conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?)
}

pub fn get_all(conn: &Connection) -> Result<HashMap<String, String>, LibraryError> {
    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::library::schema::run_migrations;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        conn
    }

    #[test]
    fn empty_db_returns_none_for_get() {
        let conn = setup();
        assert!(get(&conn, "missing").unwrap().is_none());
        assert!(get_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn set_then_get_round_trips() {
        let conn = setup();
        set(&conn, "playback.resume_enabled", "true").unwrap();
        assert_eq!(
            get(&conn, "playback.resume_enabled").unwrap().as_deref(),
            Some("true")
        );
    }

    #[test]
    fn set_overwrites_existing() {
        let conn = setup();
        set(&conn, "k", "1").unwrap();
        set(&conn, "k", "2").unwrap();
        assert_eq!(get(&conn, "k").unwrap().as_deref(), Some("2"));
    }

    #[test]
    fn delete_removes_row() {
        let conn = setup();
        set(&conn, "k", "1").unwrap();
        let removed = delete(&conn, "k").unwrap();
        assert_eq!(removed, 1);
        assert!(get(&conn, "k").unwrap().is_none());
    }

    #[test]
    fn get_all_returns_everything() {
        let conn = setup();
        set(&conn, "a", "1").unwrap();
        set(&conn, "b", "two").unwrap();
        let map = get_all(&conn).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("a").map(String::as_str), Some("1"));
        assert_eq!(map.get("b").map(String::as_str), Some("two"));
    }
}
