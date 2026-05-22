//! コメントスナップショット運用: 一覧・コメント取得・削除・ノート編集・再取得。
//!
//! [`super::videos::ingest_downloaded`] が DL 時に作る初期スナップショットに加え、
//! ユーザが複数スナップショットを明示的に管理できるようにする。

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::LibraryError;
use crate::library::videos::CommentRecord;

use super::now_unix_secs;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentSnapshotRow {
    pub id: i64,
    pub video_id: String,
    pub taken_at: i64,
    pub is_initial: bool,
    pub comment_count: i64,
    pub note: Option<String>,
}

/// 指定動画の全スナップショットを `taken_at DESC, id DESC` で取得。
pub fn list_snapshots(
    conn: &Connection,
    video_id: &str,
) -> Result<Vec<CommentSnapshotRow>, LibraryError> {
    let mut stmt = conn.prepare(
        "SELECT id, video_id, taken_at, is_initial, comment_count, note \
         FROM comment_snapshots WHERE video_id = ?1 \
         ORDER BY taken_at DESC, id DESC",
    )?;
    let rows: Vec<CommentSnapshotRow> = stmt
        .query_map(params![video_id], |row| {
            Ok(CommentSnapshotRow {
                id: row.get(0)?,
                video_id: row.get(1)?,
                taken_at: row.get(2)?,
                is_initial: row.get::<_, i64>(3)? != 0,
                comment_count: row.get(4)?,
                note: row.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// 単一スナップショットのメタデータを取得。
pub fn get_snapshot(
    conn: &Connection,
    snapshot_id: i64,
) -> Result<Option<CommentSnapshotRow>, LibraryError> {
    conn.query_row(
        "SELECT id, video_id, taken_at, is_initial, comment_count, note \
         FROM comment_snapshots WHERE id = ?1",
        params![snapshot_id],
        |row| {
            Ok(CommentSnapshotRow {
                id: row.get(0)?,
                video_id: row.get(1)?,
                taken_at: row.get(2)?,
                is_initial: row.get::<_, i64>(3)? != 0,
                comment_count: row.get(4)?,
                note: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// スナップショットに含まれるコメントを `vpos_ms ASC` で取得。
pub fn get_snapshot_comments(
    conn: &Connection,
    snapshot_id: i64,
) -> Result<Vec<CommentRecord>, LibraryError> {
    let mut stmt = conn.prepare(
        "SELECT no, vpos_ms, content, mail, user_hash, is_owner, posted_at \
         FROM comments WHERE snapshot_id = ?1 ORDER BY vpos_ms ASC",
    )?;
    let rows: Vec<CommentRecord> = stmt
        .query_map(params![snapshot_id], |row| {
            Ok(CommentRecord {
                no: row.get(0)?,
                vpos_ms: row.get(1)?,
                content: row.get(2)?,
                mail: row.get(3)?,
                user_hash: row.get(4)?,
                is_owner: row.get::<_, i64>(5)? != 0,
                posted_at: row.get(6)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// スナップショットを削除する。CASCADE で関連コメントも自動削除。
pub fn delete_snapshot(conn: &Connection, snapshot_id: i64) -> Result<bool, LibraryError> {
    let deleted = conn.execute(
        "DELETE FROM comment_snapshots WHERE id = ?1",
        params![snapshot_id],
    )?;
    Ok(deleted > 0)
}

/// スナップショットの note を更新する。None でクリア。
pub fn update_snapshot_note(
    conn: &Connection,
    snapshot_id: i64,
    note: Option<&str>,
) -> Result<bool, LibraryError> {
    let updated = conn.execute(
        "UPDATE comment_snapshots SET note = ?1 WHERE id = ?2",
        params![note, snapshot_id],
    )?;
    Ok(updated > 0)
}

/// 新規スナップショットを作成し、コメントを書き込む。
/// 既存スナップショットが無ければ `is_initial=1`、あれば `is_initial=0`。
/// `video_id` が videos テーブルに存在しない場合、外部キー制約でエラーになる。
pub fn take_snapshot(
    conn: &mut Connection,
    video_id: &str,
    comments: &[CommentRecord],
    note: Option<&str>,
) -> Result<i64, LibraryError> {
    let tx = conn.transaction()?;

    let existing: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM comment_snapshots WHERE video_id = ?1",
            params![video_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let is_initial: i64 = i64::from(existing == 0);

    tx.execute(
        "INSERT INTO comment_snapshots (video_id, taken_at, is_initial, comment_count, note) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            video_id,
            now_unix_secs(),
            is_initial,
            comments.len() as i64,
            note,
        ],
    )?;
    let snapshot_id = tx.last_insert_rowid();

    for c in comments {
        tx.execute(
            "INSERT INTO comments \
                (snapshot_id, no, vpos_ms, content, mail, user_hash, is_owner, posted_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                snapshot_id,
                c.no,
                c.vpos_ms,
                c.content,
                c.mail,
                c.user_hash,
                c.is_owner as i64,
                c.posted_at,
            ],
        )?;
    }
    tx.commit()?;
    Ok(snapshot_id)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::library::schema::run_migrations;
    use crate::library::videos::{ingest_downloaded, CommentRecord, IngestPayload, VideoRecord};

    fn setup() -> (Connection, VideoRecord) {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();

        let video = VideoRecord {
            id: "sm9".into(),
            title: "テスト動画".into(),
            description: None,
            uploader_id: Some("u1".into()),
            uploader_name: Some("投稿者".into()),
            uploader_type: None,
            category: None,
            duration_sec: 120,
            posted_at: Some(1_700_000_000),
            view_count: Some(100),
            comment_count: Some(10),
            mylist_count: None,
            thumbnail_url: None,
            video_path: Some("videos/sm9/video.mp4".into()),
            raw_meta_json: None,
            resolution: None,
        };

        ingest_downloaded(
            &mut conn,
            &IngestPayload {
                video: &video,
                tags: &[],
                comments: &[CommentRecord {
                    no: 1,
                    vpos_ms: 500,
                    content: "最初のコメント".into(),
                    mail: None,
                    user_hash: Some("u1".into()),
                    is_owner: false,
                    posted_at: Some(1_700_000_010),
                }],
            },
        )
        .unwrap();

        (conn, video)
    }

    fn sample_comments() -> Vec<CommentRecord> {
        vec![
            CommentRecord {
                no: 1,
                vpos_ms: 1000,
                content: "テスト弾幕".into(),
                mail: Some("red".into()),
                user_hash: Some("h1".into()),
                is_owner: false,
                posted_at: Some(1_700_000_100),
            },
            CommentRecord {
                no: 2,
                vpos_ms: 3000,
                content: "2つめ".into(),
                mail: None,
                user_hash: Some("h2".into()),
                is_owner: true,
                posted_at: Some(1_700_000_200),
            },
        ]
    }

    #[test]
    fn list_snapshots_empty() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        let snapshots = list_snapshots(&conn, "sm9").unwrap();
        assert!(snapshots.is_empty());
    }

    #[test]
    fn list_snapshots_returns_in_order() {
        let (mut conn, video) = setup();
        // setup already created 1 snapshot via ingest_downloaded
        // add another via take_snapshot
        let _s2 = take_snapshot(&mut conn, &video.id, &sample_comments(), Some("再取得")).unwrap();

        let snapshots = list_snapshots(&conn, "sm9").unwrap();
        assert_eq!(snapshots.len(), 2);
        // taken_at DESC なので新しい方 (is_initial=false) が先頭
        assert!(!snapshots[0].is_initial);
        assert_eq!(snapshots[0].note.as_deref(), Some("再取得"));
        assert!(snapshots[1].is_initial);
    }

    #[test]
    fn list_snapshots_does_not_mix_video_ids() {
        let (mut conn, _video) = setup();

        // sm10 を作ってスナップショットを追加
        conn.execute(
            "INSERT INTO videos (id, title, duration_sec, status, video_path) \
             VALUES ('sm10', '別動画', 60, 'active', 'videos/sm10/video.mp4')",
            [],
        )
        .unwrap();
        take_snapshot(&mut conn, "sm10", &[], None).unwrap();

        let snapshots = list_snapshots(&conn, "sm9").unwrap();
        // sm9 の snapshot だけが返る
        assert_eq!(snapshots.len(), 1);
    }

    #[test]
    fn get_snapshot_found() {
        let (conn, _video) = setup();
        let snapshots = list_snapshots(&conn, "sm9").unwrap();
        let id = snapshots[0].id;

        let found = get_snapshot(&conn, id).unwrap().unwrap();
        assert_eq!(found.video_id, "sm9");
        assert!(found.is_initial);
    }

    #[test]
    fn get_snapshot_not_found() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        assert!(get_snapshot(&conn, 99999).unwrap().is_none());
    }

    #[test]
    fn get_snapshot_comments_ordered() {
        let (mut conn, video) = setup();
        let s2 = take_snapshot(&mut conn, &video.id, &sample_comments(), None).unwrap();

        let comments = get_snapshot_comments(&conn, s2).unwrap();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].vpos_ms, 1000);
        assert_eq!(comments[1].vpos_ms, 3000);
    }

    #[test]
    fn get_snapshot_comments_empty_snapshot() {
        let (mut conn, video) = setup();
        let s2 = take_snapshot(&mut conn, &video.id, &[], None).unwrap();
        let comments = get_snapshot_comments(&conn, s2).unwrap();
        assert!(comments.is_empty());
    }

    #[test]
    fn get_snapshot_comments_bad_id() {
        let (conn, _video) = setup();
        let comments = get_snapshot_comments(&conn, 99999).unwrap();
        assert!(comments.is_empty());
    }

    #[test]
    fn delete_snapshot_cascades_comments() {
        let (mut conn, video) = setup();
        let s2 = take_snapshot(&mut conn, &video.id, &sample_comments(), None).unwrap();

        let deleted = delete_snapshot(&conn, s2).unwrap();
        assert!(deleted);

        let comments = get_snapshot_comments(&conn, s2).unwrap();
        assert!(comments.is_empty());

        // video はまだ残っている
        let snapshots = list_snapshots(&conn, "sm9").unwrap();
        assert_eq!(snapshots.len(), 1);
    }

    #[test]
    fn delete_snapshot_not_found() {
        let (conn, _video) = setup();
        assert!(!delete_snapshot(&conn, 99999).unwrap());
    }

    #[test]
    fn update_snapshot_note_set() {
        let (mut conn, video) = setup();
        let s2 = take_snapshot(&mut conn, &video.id, &[], None).unwrap();

        let updated = update_snapshot_note(&conn, s2, Some("メモ書き")).unwrap();
        assert!(updated);

        let snap = get_snapshot(&conn, s2).unwrap().unwrap();
        assert_eq!(snap.note.as_deref(), Some("メモ書き"));
    }

    #[test]
    fn update_snapshot_note_clear() {
        let (mut conn, video) = setup();
        let s2 = take_snapshot(&mut conn, &video.id, &[], Some("消すメモ")).unwrap();

        let updated = update_snapshot_note(&conn, s2, None).unwrap();
        assert!(updated);

        let snap = get_snapshot(&conn, s2).unwrap().unwrap();
        assert!(snap.note.is_none());
    }

    #[test]
    fn update_snapshot_note_not_found() {
        let (conn, _video) = setup();
        assert!(!update_snapshot_note(&conn, 99999, Some("x")).unwrap());
    }

    #[test]
    fn take_snapshot_first_is_initial() {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();

        // video が存在しないと外部キー制約で失敗するため、video を先に作る
        conn.execute(
            "INSERT INTO videos (id, title, duration_sec, status, video_path) \
             VALUES ('new_vid', 'New', 10, 'active', 'videos/new_vid/video.mp4')",
            [],
        )
        .unwrap();

        let sid = take_snapshot(&mut conn, "new_vid", &sample_comments(), Some("first")).unwrap();
        let snap = get_snapshot(&conn, sid).unwrap().unwrap();
        assert!(snap.is_initial);
        assert_eq!(snap.comment_count, 2);
        assert_eq!(snap.note.as_deref(), Some("first"));
    }

    #[test]
    fn take_snapshot_second_is_not_initial() {
        let (mut conn, video) = setup();
        // 最初は ingest_downloaded で is_initial=true
        let s2 = take_snapshot(&mut conn, &video.id, &sample_comments(), None).unwrap();
        let snap = get_snapshot(&conn, s2).unwrap().unwrap();
        assert!(!snap.is_initial);
    }

    #[test]
    fn take_snapshot_fts_sync() {
        let (mut conn, video) = setup();
        let _s2 = take_snapshot(
            &mut conn,
            &video.id,
            &[CommentRecord {
                no: 1,
                vpos_ms: 0,
                content: "弾幕テストですね".into(),
                mail: None,
                user_hash: None,
                is_owner: false,
                posted_at: None,
            }],
            None,
        )
        .unwrap();

        let fts_hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM comments_fts WHERE comments_fts MATCH '弾幕テ'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(fts_hits > 0, "FTS5 should be synced via trigger");
    }
}
