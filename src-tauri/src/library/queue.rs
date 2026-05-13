//! `download_queue` テーブルの CRUD。
//!
//! Phase 1.2 段階1 の骨組み。実 DL ワーカーはまだ無く、UI からキューを覗いて
//! 追加/キャンセルできる状態を作るまでが本モジュールの責務。
//!
//! status enum: `"pending"` / `"downloading"` / `"done"` / `"error"` / `"paused"`
//! - `cancel` は行を物理削除する（`"cancelled"` ステータスは持たない）
//! - 完了行（done/error）は `clear_finished` で一括掃除できる

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::LibraryError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadQueueItem {
    pub id: i64,
    pub video_id: String,
    pub status: String,
    pub progress: f64,
    pub error_message: Option<String>,
    pub scheduled_at: Option<i64>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub retry_count: i64,
}

impl DownloadQueueItem {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            video_id: row.get(1)?,
            status: row.get(2)?,
            progress: row.get(3)?,
            error_message: row.get(4)?,
            scheduled_at: row.get(5)?,
            started_at: row.get(6)?,
            finished_at: row.get(7)?,
            retry_count: row.get(8)?,
        })
    }
}

const SELECT_COLS: &str =
    "id, video_id, status, progress, error_message, scheduled_at, started_at, finished_at, retry_count";

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 新規ジョブを enqueue。`scheduled_at` が `Some` なら予約 DL。
/// 同じ `video_id` の再 enqueue は許可する（ユーザが意図的に再取得する場合）。
pub fn enqueue(
    conn: &Connection,
    video_id: &str,
    scheduled_at: Option<i64>,
) -> Result<DownloadQueueItem, LibraryError> {
    conn.execute(
        "INSERT INTO download_queue (video_id, status, progress, scheduled_at, retry_count) \
         VALUES (?1, 'pending', 0.0, ?2, 0)",
        params![video_id, scheduled_at],
    )?;
    let id = conn.last_insert_rowid();
    get_by_id(conn, id)?
        .ok_or_else(|| LibraryError::Integrity(format!("inserted row {id} could not be re-read")))
}

pub fn get_by_id(conn: &Connection, id: i64) -> Result<Option<DownloadQueueItem>, LibraryError> {
    let row = conn
        .query_row(
            &format!("SELECT {SELECT_COLS} FROM download_queue WHERE id = ?1"),
            params![id],
            DownloadQueueItem::from_row,
        )
        .optional()?;
    Ok(row)
}

/// 全件、新しい順（id desc）。UI 側はキャンセル済み以外の履歴も含めて表示する。
pub fn list_all(conn: &Connection) -> Result<Vec<DownloadQueueItem>, LibraryError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLS} FROM download_queue ORDER BY id DESC"
    ))?;
    let rows = stmt
        .query_map([], DownloadQueueItem::from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// ワーカが拾うべき次の候補を、scheduled_at の早い順 → id 順で返す。
/// `paused` はワーカが拾うべき対象ではないので含めない（手動再開で `pending`
/// に戻ったタイミングで再びここに乗る）。
pub fn list_pending(conn: &Connection) -> Result<Vec<DownloadQueueItem>, LibraryError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLS} FROM download_queue \
         WHERE status = 'pending' \
         ORDER BY COALESCE(scheduled_at, 0) ASC, id ASC"
    ))?;
    let rows = stmt
        .query_map([], DownloadQueueItem::from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// status を遷移させる。`"downloading"` 遷移時は started_at を、`"done"` 遷移
/// 時は finished_at を必ず埋める。
pub fn mark_status(conn: &Connection, id: i64, status: &str) -> Result<usize, LibraryError> {
    let now = now_unix();
    let updated = match status {
        "downloading" => conn.execute(
            "UPDATE download_queue SET status = ?1, started_at = COALESCE(started_at, ?2) \
             WHERE id = ?3",
            params![status, now, id],
        )?,
        "done" => conn.execute(
            "UPDATE download_queue SET status = ?1, finished_at = ?2, progress = 1.0, \
             error_message = NULL WHERE id = ?3",
            params![status, now, id],
        )?,
        _ => conn.execute(
            "UPDATE download_queue SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?,
    };
    Ok(updated)
}

/// 進捗 0.0..=1.0 を更新。範囲外はクランプ。
pub fn update_progress(conn: &Connection, id: i64, progress: f64) -> Result<usize, LibraryError> {
    let p = progress.clamp(0.0, 1.0);
    let updated = conn.execute(
        "UPDATE download_queue SET progress = ?1 WHERE id = ?2",
        params![p, id],
    )?;
    Ok(updated)
}

pub fn mark_error(conn: &Connection, id: i64, message: &str) -> Result<usize, LibraryError> {
    let now = now_unix();
    let updated = conn.execute(
        "UPDATE download_queue SET status = 'error', error_message = ?1, finished_at = ?2 \
         WHERE id = ?3",
        params![message, now, id],
    )?;
    Ok(updated)
}

pub fn increment_retry(conn: &Connection, id: i64) -> Result<usize, LibraryError> {
    Ok(conn.execute(
        "UPDATE download_queue SET retry_count = retry_count + 1 WHERE id = ?1",
        params![id],
    )?)
}

/// 行を物理削除。pending / downloading / paused / done / error いずれでも消す。
/// 実行中ジョブの停止判断は呼び出し側（ワーカ）の責務。
pub fn cancel(conn: &Connection, id: i64) -> Result<usize, LibraryError> {
    Ok(conn.execute("DELETE FROM download_queue WHERE id = ?1", params![id])?)
}

/// 完了/失敗の行を一括掃除。pending / downloading / paused は残す。
pub fn clear_finished(conn: &Connection) -> Result<usize, LibraryError> {
    Ok(conn.execute(
        "DELETE FROM download_queue WHERE status IN ('done', 'error')",
        [],
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
    fn list_all_on_empty_db_is_empty() {
        let conn = setup();
        assert!(list_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn list_pending_on_empty_db_is_empty() {
        let conn = setup();
        assert!(list_pending(&conn).unwrap().is_empty());
    }

    #[test]
    fn enqueue_inserts_row_and_returns_it() {
        let conn = setup();
        let item = enqueue(&conn, "sm9", None).unwrap();
        assert_eq!(item.video_id, "sm9");
        assert_eq!(item.status, "pending");
        assert_eq!(item.progress, 0.0);
        assert_eq!(item.retry_count, 0);
        assert!(item.scheduled_at.is_none());

        let all = list_all(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, item.id);
    }

    #[test]
    fn enqueue_same_video_twice_creates_two_rows() {
        let conn = setup();
        let a = enqueue(&conn, "sm9", None).unwrap();
        let b = enqueue(&conn, "sm9", None).unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(list_all(&conn).unwrap().len(), 2);
    }

    #[test]
    fn enqueue_with_schedule_records_timestamp() {
        let conn = setup();
        let item = enqueue(&conn, "sm9", Some(1_700_000_000)).unwrap();
        assert_eq!(item.scheduled_at, Some(1_700_000_000));
    }

    #[test]
    fn mark_downloading_sets_started_at() {
        let conn = setup();
        let item = enqueue(&conn, "sm9", None).unwrap();
        mark_status(&conn, item.id, "downloading").unwrap();
        let after = get_by_id(&conn, item.id).unwrap().unwrap();
        assert_eq!(after.status, "downloading");
        assert!(after.started_at.is_some());
        assert!(after.finished_at.is_none());
    }

    #[test]
    fn mark_done_sets_finished_at_and_progress_one() {
        let conn = setup();
        let item = enqueue(&conn, "sm9", None).unwrap();
        update_progress(&conn, item.id, 0.42).unwrap();
        mark_status(&conn, item.id, "done").unwrap();
        let after = get_by_id(&conn, item.id).unwrap().unwrap();
        assert_eq!(after.status, "done");
        assert!(after.finished_at.is_some());
        assert_eq!(after.progress, 1.0);
        assert!(after.error_message.is_none());
    }

    #[test]
    fn update_progress_clamps_to_unit_interval() {
        let conn = setup();
        let item = enqueue(&conn, "sm9", None).unwrap();
        update_progress(&conn, item.id, 1.5).unwrap();
        assert_eq!(get_by_id(&conn, item.id).unwrap().unwrap().progress, 1.0);
        update_progress(&conn, item.id, -0.3).unwrap();
        assert_eq!(get_by_id(&conn, item.id).unwrap().unwrap().progress, 0.0);
        update_progress(&conn, item.id, 0.5).unwrap();
        assert_eq!(get_by_id(&conn, item.id).unwrap().unwrap().progress, 0.5);
    }

    #[test]
    fn mark_error_records_status_and_message() {
        let conn = setup();
        let item = enqueue(&conn, "sm9", None).unwrap();
        mark_error(&conn, item.id, "boom").unwrap();
        let after = get_by_id(&conn, item.id).unwrap().unwrap();
        assert_eq!(after.status, "error");
        assert_eq!(after.error_message.as_deref(), Some("boom"));
        assert!(after.finished_at.is_some());
    }

    #[test]
    fn cancel_removes_pending_row() {
        let conn = setup();
        let item = enqueue(&conn, "sm9", None).unwrap();
        let removed = cancel(&conn, item.id).unwrap();
        assert_eq!(removed, 1);
        assert!(list_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn cancel_removes_downloading_row_too() {
        let conn = setup();
        let item = enqueue(&conn, "sm9", None).unwrap();
        mark_status(&conn, item.id, "downloading").unwrap();
        let removed = cancel(&conn, item.id).unwrap();
        assert_eq!(removed, 1);
    }

    #[test]
    fn list_pending_returns_pending_only_in_schedule_order() {
        let conn = setup();
        let a = enqueue(&conn, "sm1", Some(200)).unwrap();
        let b = enqueue(&conn, "sm2", Some(100)).unwrap();
        let c = enqueue(&conn, "sm3", None).unwrap(); // no schedule → 0
        let d = enqueue(&conn, "sm4", Some(300)).unwrap();
        // d を done に
        mark_status(&conn, d.id, "done").unwrap();
        // a を paused に（list_pending では拾わない）
        mark_status(&conn, a.id, "paused").unwrap();

        let pending = list_pending(&conn).unwrap();
        let ids: Vec<i64> = pending.iter().map(|i| i.id).collect();
        // c (scheduled 0/null) → b (100) — d (done) と a (paused) は除外
        assert_eq!(ids, vec![c.id, b.id]);
    }

    #[test]
    fn clear_finished_only_removes_done_or_error_rows() {
        let conn = setup();
        let pending = enqueue(&conn, "sm1", None).unwrap();
        let done = enqueue(&conn, "sm2", None).unwrap();
        let errored = enqueue(&conn, "sm3", None).unwrap();
        mark_status(&conn, done.id, "done").unwrap();
        mark_error(&conn, errored.id, "x").unwrap();

        let removed = clear_finished(&conn).unwrap();
        assert_eq!(removed, 2);
        let all = list_all(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, pending.id);
    }

    #[test]
    fn mark_and_update_on_missing_id_are_noop() {
        let conn = setup();
        assert_eq!(mark_status(&conn, 9999, "downloading").unwrap(), 0);
        assert_eq!(update_progress(&conn, 9999, 0.5).unwrap(), 0);
        assert_eq!(mark_error(&conn, 9999, "x").unwrap(), 0);
        assert_eq!(cancel(&conn, 9999).unwrap(), 0);
    }

    #[test]
    fn increment_retry_bumps_count() {
        let conn = setup();
        let item = enqueue(&conn, "sm9", None).unwrap();
        increment_retry(&conn, item.id).unwrap();
        increment_retry(&conn, item.id).unwrap();
        assert_eq!(get_by_id(&conn, item.id).unwrap().unwrap().retry_count, 2);
    }
}
