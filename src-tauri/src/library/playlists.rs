use rusqlite::{params, Connection, Row};
use serde::Serialize;

use crate::error::LibraryError;
use crate::library::now_unix_secs;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub source: String,
    pub source_official_id: Option<String>,
    pub imported_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub item_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistItem {
    pub playlist_id: i64,
    pub video_id: String,
    pub position: i64,
    pub added_at: i64,
    pub note: Option<String>,
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration_sec: Option<i64>,
}

const PLAYLIST_SELECT: &str =
    "SELECT p.id, p.name, p.parent_id, p.source, p.source_official_id, \
            p.imported_at, p.created_at, p.updated_at, \
            (SELECT COUNT(*) FROM playlist_items pi WHERE pi.playlist_id = p.id) AS item_count \
     FROM playlists p";

fn playlist_from_row(row: &Row<'_>) -> rusqlite::Result<Playlist> {
    Ok(Playlist {
        id: row.get(0)?,
        name: row.get(1)?,
        parent_id: row.get(2)?,
        source: row.get(3)?,
        source_official_id: row.get(4)?,
        imported_at: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        item_count: row.get(8)?,
    })
}

fn playlist_item_from_row(row: &Row<'_>) -> rusqlite::Result<PlaylistItem> {
    Ok(PlaylistItem {
        playlist_id: row.get(0)?,
        video_id: row.get(1)?,
        position: row.get(2)?,
        added_at: row.get(3)?,
        note: row.get(4)?,
        title: row.get(5)?,
        thumbnail_url: row.get(6)?,
        duration_sec: row.get(7)?,
    })
}

/// プレイリスト本体の `updated_at` を `now` で更新する。アイテムの追加/削除/
/// 並び替えが起きた時に親プレイリストの順序を最新化したい場合に使う。
fn touch_playlist(conn: &Connection, playlist_id: i64, now: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE playlists SET updated_at = ?1 WHERE id = ?2",
        params![now, playlist_id],
    )
}

pub fn list_playlists(conn: &Connection) -> Result<Vec<Playlist>, LibraryError> {
    let mut stmt = conn.prepare(&format!("{PLAYLIST_SELECT} ORDER BY p.updated_at DESC"))?;
    let rows = stmt
        .query_map([], playlist_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn create_playlist(
    conn: &Connection,
    name: &str,
    parent_id: Option<i64>,
) -> Result<Playlist, LibraryError> {
    let now = now_unix_secs();
    conn.execute(
        "INSERT INTO playlists (name, parent_id, source, created_at, updated_at) \
         VALUES (?1, ?2, 'local', ?3, ?3)",
        params![name, parent_id, now],
    )?;
    let id = conn.last_insert_rowid();
    Ok(Playlist {
        id,
        name: name.to_string(),
        parent_id,
        source: "local".into(),
        source_official_id: None,
        imported_at: None,
        created_at: now,
        updated_at: now,
        item_count: 0,
    })
}

pub fn update_playlist(
    conn: &Connection,
    id: i64,
    name: &str,
    parent_id: Option<i64>,
) -> Result<Playlist, LibraryError> {
    let now = now_unix_secs();
    let affected = conn.execute(
        "UPDATE playlists SET name = ?1, parent_id = ?2, updated_at = ?3 WHERE id = ?4",
        params![name, parent_id, now, id],
    )?;
    if affected == 0 {
        return Err(LibraryError::NotFound("playlist"));
    }
    get_playlist(conn, id)
}

pub fn delete_playlist(conn: &Connection, id: i64) -> Result<bool, LibraryError> {
    let affected = conn.execute("DELETE FROM playlists WHERE id = ?1", params![id])?;
    Ok(affected > 0)
}

pub fn get_playlist(conn: &Connection, id: i64) -> Result<Playlist, LibraryError> {
    let mut stmt = conn.prepare(&format!("{PLAYLIST_SELECT} WHERE p.id = ?1"))?;
    stmt.query_row(params![id], playlist_from_row)
        .map_err(|_| LibraryError::NotFound("playlist"))
}

pub fn list_playlist_items(
    conn: &Connection,
    playlist_id: i64,
) -> Result<Vec<PlaylistItem>, LibraryError> {
    let mut stmt = conn.prepare(
        "SELECT pi.playlist_id, pi.video_id, pi.position, pi.added_at, pi.note, \
                v.title, v.thumbnail_url, v.duration_sec \
         FROM playlist_items pi \
         LEFT JOIN videos v ON v.id = pi.video_id \
         WHERE pi.playlist_id = ?1 \
         ORDER BY pi.position ASC",
    )?;
    let rows = stmt
        .query_map(params![playlist_id], playlist_item_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn add_playlist_item(
    conn: &Connection,
    playlist_id: i64,
    video_id: &str,
    position: Option<i64>,
    note: Option<&str>,
) -> Result<PlaylistItem, LibraryError> {
    let now = now_unix_secs();

    let next_pos = match position {
        Some(pos) => pos,
        None => {
            let max: Option<i64> = conn
                .query_row(
                    "SELECT MAX(position) FROM playlist_items WHERE playlist_id = ?1",
                    params![playlist_id],
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            max.unwrap_or(0) + 1
        }
    };

    conn.execute(
        "INSERT OR IGNORE INTO playlist_items (playlist_id, video_id, position, added_at, note) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![playlist_id, video_id, next_pos, now, note],
    )?;

    touch_playlist(conn, playlist_id, now)?;

    Ok(PlaylistItem {
        playlist_id,
        video_id: video_id.to_string(),
        position: next_pos,
        added_at: now,
        note: note.map(String::from),
        title: None,
        thumbnail_url: None,
        duration_sec: None,
    })
}

/// `op` (`playlist_items` への DML) を実行し、影響行が 1 件以上あったときに
/// 親プレイリストの `updated_at` を更新する。戻り値は「実際に行が動いたか」。
fn modify_item_and_touch(
    conn: &Connection,
    playlist_id: i64,
    op: impl FnOnce() -> rusqlite::Result<usize>,
) -> Result<bool, LibraryError> {
    let affected = op()?;
    if affected > 0 {
        touch_playlist(conn, playlist_id, now_unix_secs())?;
    }
    Ok(affected > 0)
}

pub fn remove_playlist_item(
    conn: &Connection,
    playlist_id: i64,
    video_id: &str,
) -> Result<bool, LibraryError> {
    modify_item_and_touch(conn, playlist_id, || {
        conn.execute(
            "DELETE FROM playlist_items WHERE playlist_id = ?1 AND video_id = ?2",
            params![playlist_id, video_id],
        )
    })
}

pub fn reorder_playlist_item(
    conn: &Connection,
    playlist_id: i64,
    video_id: &str,
    new_position: i64,
) -> Result<bool, LibraryError> {
    modify_item_and_touch(conn, playlist_id, || {
        conn.execute(
            "UPDATE playlist_items SET position = ?1 WHERE playlist_id = ?2 AND video_id = ?3",
            params![new_position, playlist_id, video_id],
        )
    })
}
