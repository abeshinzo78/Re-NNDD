//! `plugins` テーブルへの CRUD。`library/settings.rs` のスタイルに準拠。
//!
//! `manifest_json` は plugins テーブルに冗長コピーされるが、これにより
//! フロントは 1 クエリで一覧を構築でき、各プラグインディレクトリを
//! 走査せずに済む (DB が canonical) 。

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::LibraryError;

#[derive(Debug, Clone)]
pub struct PluginRow {
    pub plugin_id: String,
    pub enabled: bool,
    pub version: String,
    pub manifest_json: String,
    pub installed_at: i64,
    pub updated_at: i64,
}

pub fn list_all(conn: &Connection) -> Result<Vec<PluginRow>, LibraryError> {
    let mut stmt = conn.prepare(
        "SELECT plugin_id, enabled, version, manifest_json, installed_at, updated_at \
         FROM plugins ORDER BY plugin_id",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok(PluginRow {
                plugin_id: row.get(0)?,
                enabled: row.get::<_, i64>(1)? != 0,
                version: row.get(2)?,
                manifest_json: row.get(3)?,
                installed_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, plugin_id: &str) -> Result<Option<PluginRow>, LibraryError> {
    let row = conn
        .query_row(
            "SELECT plugin_id, enabled, version, manifest_json, installed_at, updated_at \
             FROM plugins WHERE plugin_id = ?1",
            params![plugin_id],
            |row| {
                Ok(PluginRow {
                    plugin_id: row.get(0)?,
                    enabled: row.get::<_, i64>(1)? != 0,
                    version: row.get(2)?,
                    manifest_json: row.get(3)?,
                    installed_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

pub fn upsert(
    conn: &Connection,
    plugin_id: &str,
    version: &str,
    manifest_json: &str,
    now: i64,
) -> Result<(), LibraryError> {
    conn.execute(
        "INSERT INTO plugins (plugin_id, enabled, version, manifest_json, installed_at, updated_at) \
         VALUES (?1, 0, ?2, ?3, ?4, ?4) \
         ON CONFLICT(plugin_id) DO UPDATE SET \
             version = excluded.version, \
             manifest_json = excluded.manifest_json, \
             updated_at = excluded.updated_at",
        params![plugin_id, version, manifest_json, now],
    )?;
    Ok(())
}

pub fn set_enabled(
    conn: &Connection,
    plugin_id: &str,
    enabled: bool,
    now: i64,
) -> Result<usize, LibraryError> {
    Ok(conn.execute(
        "UPDATE plugins SET enabled = ?2, updated_at = ?3 WHERE plugin_id = ?1",
        params![plugin_id, if enabled { 1 } else { 0 }, now],
    )?)
}

pub fn delete(conn: &Connection, plugin_id: &str) -> Result<usize, LibraryError> {
    Ok(conn.execute(
        "DELETE FROM plugins WHERE plugin_id = ?1",
        params![plugin_id],
    )?)
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
    fn list_all_empty_initially() {
        let conn = setup();
        assert!(list_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn upsert_then_get_round_trips() {
        let conn = setup();
        upsert(&conn, "a", "0.1.0", "{\"k\":1}", 1000).unwrap();
        let row = get(&conn, "a").unwrap().unwrap();
        assert_eq!(row.plugin_id, "a");
        assert_eq!(row.version, "0.1.0");
        assert!(!row.enabled);
        assert_eq!(row.installed_at, 1000);
    }

    #[test]
    fn upsert_overwrites_version_and_updated_at_but_keeps_installed_at() {
        let conn = setup();
        upsert(&conn, "a", "0.1.0", "{}", 1000).unwrap();
        upsert(&conn, "a", "0.2.0", "{}", 2000).unwrap();
        let row = get(&conn, "a").unwrap().unwrap();
        assert_eq!(row.version, "0.2.0");
        assert_eq!(row.installed_at, 1000);
        assert_eq!(row.updated_at, 2000);
    }

    #[test]
    fn set_enabled_toggles() {
        let conn = setup();
        upsert(&conn, "a", "0.1.0", "{}", 1000).unwrap();
        let changed = set_enabled(&conn, "a", true, 1100).unwrap();
        assert_eq!(changed, 1);
        assert!(get(&conn, "a").unwrap().unwrap().enabled);
        set_enabled(&conn, "a", false, 1200).unwrap();
        assert!(!get(&conn, "a").unwrap().unwrap().enabled);
    }

    #[test]
    fn delete_removes_row() {
        let conn = setup();
        upsert(&conn, "a", "0.1.0", "{}", 1000).unwrap();
        let removed = delete(&conn, "a").unwrap();
        assert_eq!(removed, 1);
        assert!(get(&conn, "a").unwrap().is_none());
    }
}
