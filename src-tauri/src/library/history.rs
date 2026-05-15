use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::LibraryError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayHistoryItem {
    pub id: i64,
    pub video_id: String,
    pub played_at: i64,
    pub duration_played_sec: f64,
    pub position_at_close_sec: Option<f64>,
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration_sec: Option<i64>,
}

pub fn record_playback(
    conn: &Connection,
    video_id: &str,
    duration_played_sec: f64,
    position_at_close_sec: Option<f64>,
) -> Result<PlayHistoryItem, LibraryError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| LibraryError::Integrity(e.to_string()))?
        .as_secs() as i64;

    conn.execute(
        "INSERT INTO play_history (video_id, played_at, duration_played_sec, position_at_close_sec) \
         VALUES (?1, ?2, ?3, ?4)",
        params![video_id, now, duration_played_sec, position_at_close_sec],
    )?;

    conn.execute(
        "UPDATE videos SET play_count = play_count + 1, last_played_at = ?1 WHERE id = ?2",
        params![now, video_id],
    )?;

    let id = conn.last_insert_rowid();
    Ok(PlayHistoryItem {
        id,
        video_id: video_id.to_string(),
        played_at: now,
        duration_played_sec,
        position_at_close_sec,
        title: None,
        thumbnail_url: None,
        duration_sec: None,
    })
}

pub fn list_play_history(
    conn: &Connection,
    offset: u32,
    limit: u32,
) -> Result<Vec<PlayHistoryItem>, LibraryError> {
    let limit = limit.min(200);
    let mut stmt = conn.prepare(
        "SELECT h.id, h.video_id, h.played_at, h.duration_played_sec, h.position_at_close_sec, \
                v.title, v.thumbnail_url, v.duration_sec \
         FROM play_history h \
         LEFT JOIN videos v ON v.id = h.video_id \
         ORDER BY h.played_at DESC \
         LIMIT ?1 OFFSET ?2",
    )?;
    let rows = stmt
        .query_map(params![limit, offset], |row| {
            Ok(PlayHistoryItem {
                id: row.get(0)?,
                video_id: row.get(1)?,
                played_at: row.get(2)?,
                duration_played_sec: row.get(3)?,
                position_at_close_sec: row.get(4)?,
                title: row.get(5)?,
                thumbnail_url: row.get(6)?,
                duration_sec: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn clear_play_history(conn: &Connection) -> Result<usize, LibraryError> {
    let affected = conn.execute("DELETE FROM play_history", [])?;
    Ok(affected)
}

pub fn delete_play_history_item(conn: &Connection, id: i64) -> Result<bool, LibraryError> {
    let affected = conn.execute("DELETE FROM play_history WHERE id = ?1", params![id])?;
    Ok(affected > 0)
}
