//! `videos` / `tags` / `comment_snapshots` / `comments` への書き込み。
//!
//! ダウンロード完了時に呼ばれ、メタ・タグ・初期コメスナップショットを
//! 1 トランザクションで原子的に書き込む。FTS5 は INSERT トリガで自動同期される。

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::LibraryError;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VideoRecord {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub uploader_id: Option<String>,
    pub uploader_name: Option<String>,
    pub uploader_type: Option<String>,
    pub category: Option<String>,
    pub duration_sec: i64,
    pub posted_at: Option<i64>,
    pub view_count: Option<i64>,
    pub comment_count: Option<i64>,
    pub mylist_count: Option<i64>,
    pub thumbnail_url: Option<String>,
    /// `app_data_dir` からの相対パス。例: `"videos/sm9/video.mp4"`
    pub video_path: Option<String>,
    pub raw_meta_json: Option<String>,
    /// "1280x720" 形式。yt-dlp info の width/height から作る。Optional。
    pub resolution: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagRecord {
    pub name: String,
    pub is_locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentRecord {
    pub no: i64,
    pub vpos_ms: i64,
    pub content: String,
    pub mail: Option<String>,
    pub user_hash: Option<String>,
    pub is_owner: bool,
    pub posted_at: Option<i64>,
}

/// 取り込みパッケージ。DL 完了時にこの 1 つを書き込めば充分という設計。
pub struct IngestPayload<'a> {
    pub video: &'a VideoRecord,
    pub tags: &'a [TagRecord],
    pub comments: &'a [CommentRecord],
}

fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// DL 完了時の書き込み一式。新規 / 既存どちらでも安全。
///
/// - videos: ON CONFLICT(id) DO UPDATE — 既存行はメタを上書き、`video_path` と
///   `downloaded_at` も更新する
/// - tags(source='official'): いったん全削除してから入れ直す
/// - comment_snapshots + comments: 新規 snapshot を 1 件作って下に詰める。
///   既存 snapshot が無ければ `is_initial = 1` でマークする。
pub fn ingest_downloaded(
    conn: &mut Connection,
    payload: &IngestPayload<'_>,
) -> Result<i64, LibraryError> {
    let tx = conn.transaction()?;
    let now = now_unix();

    upsert_video_with_tx(&tx, payload.video, now)?;
    replace_tags_with_tx(&tx, &payload.video.id, "official", payload.tags)?;
    let snapshot_id = create_snapshot_with_tx(&tx, &payload.video.id, payload.comments, now)?;

    tx.commit()?;
    Ok(snapshot_id)
}

/// 動画ローカルファイル消失時などに `video_path` だけ書き換えたいケース用。
pub fn set_video_path(
    conn: &Connection,
    video_id: &str,
    relative_path: Option<&str>,
) -> Result<usize, LibraryError> {
    let now = now_unix();
    Ok(conn.execute(
        "UPDATE videos SET video_path = ?1, downloaded_at = COALESCE(downloaded_at, ?2) \
         WHERE id = ?3",
        params![relative_path, now, video_id],
    )?)
}

pub fn get_video(
    conn: &Connection,
    video_id: &str,
) -> Result<Option<VideoRecord>, LibraryError> {
    let row = conn
        .query_row(
            "SELECT id, title, description, uploader_id, uploader_name, uploader_type, \
                    category, duration_sec, posted_at, view_count, comment_count, mylist_count, \
                    thumbnail_url, video_path, raw_meta_json, resolution \
             FROM videos WHERE id = ?1",
            params![video_id],
            |row| {
                Ok(VideoRecord {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    uploader_id: row.get(3)?,
                    uploader_name: row.get(4)?,
                    uploader_type: row.get(5)?,
                    category: row.get(6)?,
                    duration_sec: row.get(7)?,
                    posted_at: row.get(8)?,
                    view_count: row.get(9)?,
                    comment_count: row.get(10)?,
                    mylist_count: row.get(11)?,
                    thumbnail_url: row.get(12)?,
                    video_path: row.get(13)?,
                    raw_meta_json: row.get(14)?,
                    resolution: row.get(15)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn upsert_video_with_tx(
    tx: &rusqlite::Transaction<'_>,
    v: &VideoRecord,
    now: i64,
) -> Result<(), LibraryError> {
    tx.execute(
        "INSERT INTO videos \
            (id, title, description, uploader_id, uploader_name, uploader_type, category, \
             duration_sec, posted_at, view_count, comment_count, mylist_count, thumbnail_url, \
             status, status_checked_at, downloaded_at, video_path, raw_meta_json, resolution) \
         VALUES \
            (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 'active', ?14, ?14, ?15, ?16, ?17) \
         ON CONFLICT(id) DO UPDATE SET \
            title = excluded.title, \
            description = excluded.description, \
            uploader_id = excluded.uploader_id, \
            uploader_name = excluded.uploader_name, \
            uploader_type = excluded.uploader_type, \
            category = excluded.category, \
            duration_sec = excluded.duration_sec, \
            posted_at = excluded.posted_at, \
            view_count = excluded.view_count, \
            comment_count = excluded.comment_count, \
            mylist_count = excluded.mylist_count, \
            thumbnail_url = excluded.thumbnail_url, \
            status = 'active', \
            status_checked_at = excluded.status_checked_at, \
            downloaded_at = COALESCE(videos.downloaded_at, excluded.downloaded_at), \
            video_path = COALESCE(excluded.video_path, videos.video_path), \
            raw_meta_json = COALESCE(excluded.raw_meta_json, videos.raw_meta_json), \
            resolution = COALESCE(excluded.resolution, videos.resolution)",
        params![
            v.id,
            v.title,
            v.description,
            v.uploader_id,
            v.uploader_name,
            v.uploader_type,
            v.category,
            v.duration_sec,
            v.posted_at,
            v.view_count,
            v.comment_count,
            v.mylist_count,
            v.thumbnail_url,
            now,
            v.video_path,
            v.raw_meta_json,
            v.resolution,
        ],
    )?;
    Ok(())
}

fn replace_tags_with_tx(
    tx: &rusqlite::Transaction<'_>,
    video_id: &str,
    source: &str,
    tags: &[TagRecord],
) -> Result<(), LibraryError> {
    tx.execute(
        "DELETE FROM tags WHERE video_id = ?1 AND source = ?2",
        params![video_id, source],
    )?;
    for tag in tags {
        tx.execute(
            "INSERT OR IGNORE INTO tags (video_id, name, is_locked, source) \
             VALUES (?1, ?2, ?3, ?4)",
            params![video_id, tag.name, tag.is_locked as i64, source],
        )?;
    }
    Ok(())
}

fn create_snapshot_with_tx(
    tx: &rusqlite::Transaction<'_>,
    video_id: &str,
    comments: &[CommentRecord],
    now: i64,
) -> Result<i64, LibraryError> {
    let existing: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM comment_snapshots WHERE video_id = ?1",
            params![video_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let is_initial: i64 = if existing == 0 { 1 } else { 0 };
    tx.execute(
        "INSERT INTO comment_snapshots (video_id, taken_at, is_initial, comment_count) \
         VALUES (?1, ?2, ?3, ?4)",
        params![video_id, now, is_initial, comments.len() as i64],
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
    Ok(snapshot_id)
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

    fn sample_video() -> VideoRecord {
        VideoRecord {
            id: "sm9".into(),
            title: "テスト動画".into(),
            description: Some("desc".into()),
            uploader_id: Some("user/1".into()),
            uploader_name: Some("投稿者".into()),
            uploader_type: Some("user".into()),
            category: None,
            duration_sec: 320,
            posted_at: Some(1_700_000_000),
            view_count: Some(100),
            comment_count: Some(50),
            mylist_count: Some(5),
            thumbnail_url: Some("https://example.test/t.jpg".into()),
            video_path: Some("videos/sm9/video.mp4".into()),
            raw_meta_json: None,
            resolution: Some("1280x720".into()),
        }
    }

    fn sample_tags() -> Vec<TagRecord> {
        vec![
            TagRecord { name: "VOCALOID".into(), is_locked: true },
            TagRecord { name: "初音ミク".into(), is_locked: false },
        ]
    }

    fn sample_comments() -> Vec<CommentRecord> {
        vec![
            CommentRecord {
                no: 1,
                vpos_ms: 500,
                content: "wwww".into(),
                mail: Some("white".into()),
                user_hash: Some("u1".into()),
                is_owner: false,
                posted_at: Some(1_700_000_010),
            },
            CommentRecord {
                no: 2,
                vpos_ms: 1500,
                content: "弾幕薄いよ".into(),
                mail: Some("red big".into()),
                user_hash: Some("u2".into()),
                is_owner: false,
                posted_at: Some(1_700_000_011),
            },
        ]
    }

    #[test]
    fn ingest_downloaded_inserts_video_tags_and_initial_snapshot() {
        let mut conn = setup();
        let video = sample_video();
        let tags = sample_tags();
        let comments = sample_comments();

        let snap_id = ingest_downloaded(
            &mut conn,
            &IngestPayload {
                video: &video,
                tags: &tags,
                comments: &comments,
            },
        )
        .unwrap();

        // video 行
        let stored = get_video(&conn, "sm9").unwrap().unwrap();
        assert_eq!(stored.title, "テスト動画");
        assert_eq!(stored.video_path.as_deref(), Some("videos/sm9/video.mp4"));

        // tags
        let mut tag_names: Vec<String> = conn
            .prepare("SELECT name FROM tags WHERE video_id = 'sm9' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        tag_names.sort();
        assert_eq!(tag_names, vec!["VOCALOID".to_string(), "初音ミク".to_string()]);

        // snapshot
        let (is_initial, comment_count): (i64, i64) = conn
            .query_row(
                "SELECT is_initial, comment_count FROM comment_snapshots WHERE id = ?1",
                params![snap_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(is_initial, 1);
        assert_eq!(comment_count, 2);

        // comments
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM comments WHERE snapshot_id = ?1",
                params![snap_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);

        // FTS5 が同期されている (trigger)。trigram tokenizer なので 3 文字以上で検索。
        let fts_hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM comments_fts WHERE comments_fts MATCH '弾幕薄'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fts_hits, 1);
    }

    #[test]
    fn ingest_twice_marks_only_first_as_initial() {
        let mut conn = setup();
        let v = sample_video();
        let t = sample_tags();
        let c = sample_comments();

        let s1 = ingest_downloaded(
            &mut conn,
            &IngestPayload { video: &v, tags: &t, comments: &c },
        ).unwrap();
        let s2 = ingest_downloaded(
            &mut conn,
            &IngestPayload { video: &v, tags: &t, comments: &c },
        ).unwrap();
        assert_ne!(s1, s2);

        let initials: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM comment_snapshots WHERE video_id = 'sm9' AND is_initial = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(initials, 1);

        let snapshots: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM comment_snapshots WHERE video_id = 'sm9'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(snapshots, 2);
    }

    #[test]
    fn ingest_replaces_official_tags() {
        let mut conn = setup();
        let v = sample_video();
        ingest_downloaded(
            &mut conn,
            &IngestPayload {
                video: &v,
                tags: &sample_tags(),
                comments: &[],
            },
        )
        .unwrap();
        // Local tag を 1 件混ぜておいて、再 ingest 後にも残ることを確認
        conn.execute(
            "INSERT INTO tags (video_id, name, is_locked, source) VALUES ('sm9', 'マイタグ', 0, 'local')",
            [],
        ).unwrap();
        ingest_downloaded(
            &mut conn,
            &IngestPayload {
                video: &v,
                tags: &[TagRecord { name: "新タグ".into(), is_locked: false }],
                comments: &[],
            },
        )
        .unwrap();

        let names: Vec<(String, String)> = conn
            .prepare("SELECT name, source FROM tags WHERE video_id='sm9' ORDER BY source, name")
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            names,
            vec![
                ("マイタグ".to_string(), "local".to_string()),
                ("新タグ".to_string(), "official".to_string()),
            ]
        );
    }

    #[test]
    fn ingest_upserts_video_metadata() {
        let mut conn = setup();
        let mut v = sample_video();
        ingest_downloaded(
            &mut conn,
            &IngestPayload { video: &v, tags: &[], comments: &[] },
        ).unwrap();

        v.title = "新タイトル".into();
        v.view_count = Some(99999);
        ingest_downloaded(
            &mut conn,
            &IngestPayload { video: &v, tags: &[], comments: &[] },
        ).unwrap();

        let stored = get_video(&conn, "sm9").unwrap().unwrap();
        assert_eq!(stored.title, "新タイトル");
        assert_eq!(stored.view_count, Some(99999));
    }

    #[test]
    fn set_video_path_updates_existing_row() {
        let mut conn = setup();
        let mut v = sample_video();
        v.video_path = None;
        ingest_downloaded(
            &mut conn,
            &IngestPayload { video: &v, tags: &[], comments: &[] },
        ).unwrap();

        set_video_path(&conn, "sm9", Some("videos/sm9/video.mp4")).unwrap();
        let stored = get_video(&conn, "sm9").unwrap().unwrap();
        assert_eq!(stored.video_path.as_deref(), Some("videos/sm9/video.mp4"));
    }
}
