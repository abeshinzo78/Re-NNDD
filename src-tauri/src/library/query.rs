//! Library query engine: search, sort, filter, and aggregate over local videos.
//!
//! All read-only. Write operations stay in [`super::videos`].

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::LibraryError;

// ---------------------------------------------------------------------------
// Query parameters (front-end → Rust)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryQuery {
    /// Free-text search. Matches against video title (LIKE), tag names, and
    /// FTS5-indexed comments (trigram). Empty / None → no text filter.
    pub q: Option<String>,

    /// Tag filter: only include videos that have *all* of these tags.
    pub tags: Option<Vec<String>>,

    /// Uploader id filter.
    pub uploader_id: Option<String>,

    /// Minimum duration in seconds (inclusive).
    pub min_duration: Option<i64>,

    /// Maximum duration in seconds (inclusive).
    pub max_duration: Option<i64>,

    /// Resolution filter (exact match, e.g. "1280x720").
    pub resolution: Option<String>,

    /// Sort column. Defaults to `"downloaded_at"`.
    pub sort_by: Option<String>,

    /// `"asc"` or `"desc"`. Defaults to `"desc"`.
    pub sort_order: Option<String>,

    /// Pagination offset. Defaults to 0.
    pub offset: Option<u32>,

    /// Page size. Defaults to 100. Capped at 500.
    pub limit: Option<u32>,
}

/// Allowed sort columns — whitelist to prevent SQL injection.
const ALLOWED_SORT: &[&str] = &[
    "title",
    "downloaded_at",
    "posted_at",
    "view_count",
    "duration_sec",
    "play_count",
    "last_played_at",
    "mylist_count",
    "comment_count",
];

fn validate_sort(sort_by: &str) -> Result<(), LibraryError> {
    if ALLOWED_SORT.contains(&sort_by) {
        Ok(())
    } else {
        Err(LibraryError::Integrity(format!(
            "invalid sort_by: {sort_by:?}"
        )))
    }
}

fn sort_order(order: &str) -> Result<&'static str, LibraryError> {
    match order {
        "asc" => Ok("ASC"),
        "desc" => Ok("DESC"),
        _ => Err(LibraryError::Integrity(format!(
            "invalid sort_order: {order:?}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Result types (Rust → front-end)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryVideoRow {
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
    pub video_path: Option<String>,
    pub resolution: Option<String>,
    pub downloaded_at: Option<i64>,
    pub play_count: i64,
    pub last_played_at: Option<i64>,
    /// Tags attached to this video (all sources).
    pub tags: Vec<String>,
    #[serde(default)]
    pub local_thumbnail_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub items: Vec<LibraryVideoRow>,
    pub total_count: i64,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    pub total_videos: i64,
    pub total_duration_sec: i64,
    pub total_comments: i64,
    pub unique_uploaders: i64,
    pub unique_tags: i64,
    /// Top-N tags ordered by frequency. Limited to 50.
    pub top_tags: Vec<TagCount>,
    /// Resolution distribution: "1280x720" → count.
    pub resolution_distribution: Vec<ResolutionCount>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCount {
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionCount {
    pub resolution: String,
    pub count: i64,
}

// ---------------------------------------------------------------------------
// Query execution
// ---------------------------------------------------------------------------

/// Execute a library query. Returns matching videos + total count (ignoring
/// pagination) so the front-end can show "N 件中 1–50" style navigation.
pub fn query_videos(conn: &Connection, q: &LibraryQuery) -> Result<QueryResult, LibraryError> {
    let sort_by = q.sort_by.as_deref().unwrap_or("downloaded_at");
    let sort_dir = q.sort_order.as_deref().unwrap_or("desc");
    validate_sort(sort_by)?;
    let order_sql = sort_order(sort_dir)?;

    let offset = q.offset.unwrap_or(0);
    let limit = q.limit.unwrap_or(100).min(500);

    // Build WHERE clause and param list.
    let mut sql_where = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    sql_where.push("v.video_path IS NOT NULL".to_string());

    if let Some(ref text) = q.q {
        if !text.is_empty() {
            let pattern = format!("%{text}%");
            let fts_ok = text.chars().count() >= 3;
            let p = params.len() + 1;
            params.push(Box::new(pattern.clone()));
            if fts_ok {
                let p2 = params.len() + 1;
                params.push(Box::new(text.clone()));
                sql_where.push(format!(
                    "(v.title LIKE ?{p} OR EXISTS (\
                       SELECT 1 FROM tags t WHERE t.video_id = v.id AND t.name LIKE ?{p}\
                     ) OR EXISTS (\
                       SELECT 1 FROM comments_fts fts \
                       JOIN comments c ON c.id = fts.rowid \
                       JOIN comment_snapshots cs ON cs.id = c.snapshot_id \
                       WHERE cs.video_id = v.id AND comments_fts MATCH ?{p2}\
                     ))",
                ));
            } else {
                sql_where.push(format!(
                    "(v.title LIKE ?{p} OR EXISTS (\
                       SELECT 1 FROM tags t WHERE t.video_id = v.id AND t.name LIKE ?{p}\
                     ))",
                ));
            }
        }
    }

    if let Some(ref tags) = q.tags {
        for tag in tags {
            let p = params.len() + 1;
            params.push(Box::new(tag.clone()));
            sql_where.push(format!(
                "EXISTS (SELECT 1 FROM tags t WHERE t.video_id = v.id AND t.name = ?{p})",
            ));
        }
    }

    if let Some(ref uid) = q.uploader_id {
        let p = params.len() + 1;
        params.push(Box::new(uid.clone()));
        sql_where.push(format!("v.uploader_id = ?{p}"));
    }

    if let Some(min) = q.min_duration {
        let p = params.len() + 1;
        params.push(Box::new(min));
        sql_where.push(format!("v.duration_sec >= ?{p}"));
    }
    if let Some(max) = q.max_duration {
        let p = params.len() + 1;
        params.push(Box::new(max));
        sql_where.push(format!("v.duration_sec <= ?{p}"));
    }

    if let Some(ref res) = q.resolution {
        let p = params.len() + 1;
        params.push(Box::new(res.clone()));
        sql_where.push(format!("v.resolution = ?{p}"));
    }

    let where_clause = format!("WHERE {}", sql_where.join(" AND "));

    // Count query.
    let count_sql = format!("SELECT COUNT(*) FROM videos v {where_clause}");
    let total_count: i64 = conn.query_row(
        &count_sql,
        rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
        |row| row.get(0),
    )?;

    // Data query with pagination — rebuild params to add limit/offset.
    let lim_p = params.len() + 1;
    let off_p = params.len() + 2;
    let data_sql = format!(
        "SELECT v.id, v.title, v.description, v.uploader_id, v.uploader_name, \
                v.uploader_type, v.category, v.duration_sec, v.posted_at, v.view_count, \
                v.comment_count, v.mylist_count, v.thumbnail_url, v.video_path, v.resolution, \
                v.downloaded_at, v.play_count, v.last_played_at \
         FROM videos v \
         {where_clause} \
         ORDER BY v.{sort_by} {order_sql}, v.id DESC \
         LIMIT ?{lim_p} OFFSET ?{off_p}",
    );

    params.push(Box::new(limit));
    params.push(Box::new(offset));

    let mut stmt = conn.prepare(&data_sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows: Vec<LibraryVideoRow> = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(LibraryVideoRow {
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
                resolution: row.get(14)?,
                downloaded_at: row.get(15)?,
                play_count: row.get::<_, Option<i64>>(16)?.unwrap_or(0),
                last_played_at: row.get(17)?,
                tags: Vec::new(),
                local_thumbnail_path: None,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // Batch-fetch tags.
    let mut items = rows;
    let ids: Vec<&str> = items.iter().map(|v| v.id.as_str()).collect();
    if !ids.is_empty() {
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let tag_sql = format!(
            "SELECT video_id, name FROM tags WHERE video_id IN ({placeholders}) ORDER BY video_id, name"
        );
        let mut tag_stmt = conn.prepare(&tag_sql)?;
        let tag_params: Vec<&dyn rusqlite::types::ToSql> = ids
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let mut by_video: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        let tag_rows = tag_stmt.query_map(tag_params.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in tag_rows.flatten() {
            by_video.entry(r.0).or_default().push(r.1);
        }
        for item in items.iter_mut() {
            if let Some(t) = by_video.remove(&item.id) {
                item.tags = t;
            }
        }
    }

    Ok(QueryResult {
        items,
        total_count,
        offset,
        limit,
    })
}

/// Return aggregate statistics about the local library.
pub fn get_stats(conn: &Connection) -> Result<LibraryStats, LibraryError> {
    let total_videos: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM videos WHERE video_path IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let total_duration_sec: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(duration_sec), 0) FROM videos WHERE video_path IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let total_comments: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(comment_count), 0) FROM videos WHERE video_path IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let unique_uploaders: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT uploader_id) FROM videos WHERE video_path IS NOT NULL AND uploader_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let unique_tags: i64 = conn
        .query_row("SELECT COUNT(DISTINCT name) FROM tags", [], |r| r.get(0))
        .unwrap_or(0);

    // Top 50 tags by frequency.
    let mut tag_stmt = conn.prepare(
        "SELECT name, COUNT(*) AS cnt FROM tags GROUP BY name ORDER BY cnt DESC, name LIMIT 50",
    )?;
    let top_tags: Vec<TagCount> = tag_stmt
        .query_map([], |row| {
            Ok(TagCount {
                name: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    // Resolution distribution.
    let mut res_stmt = conn.prepare(
        "SELECT resolution, COUNT(*) AS cnt \
         FROM videos WHERE video_path IS NOT NULL AND resolution IS NOT NULL \
         GROUP BY resolution ORDER BY cnt DESC",
    )?;
    let resolution_distribution: Vec<ResolutionCount> = res_stmt
        .query_map([], |row| {
            Ok(ResolutionCount {
                resolution: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(LibraryStats {
        total_videos,
        total_duration_sec,
        total_comments,
        unique_uploaders,
        unique_tags,
        top_tags,
        resolution_distribution,
    })
}

/// Fetch all distinct tag names in the library, ordered alphabetically.
pub fn list_all_tags(conn: &Connection) -> Result<Vec<String>, LibraryError> {
    let mut stmt = conn.prepare("SELECT DISTINCT name FROM tags ORDER BY name")?;
    let tags: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tags)
}

/// Fetch all distinct resolutions in the library.
pub fn list_resolutions(conn: &Connection) -> Result<Vec<String>, LibraryError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT resolution FROM videos \
         WHERE video_path IS NOT NULL AND resolution IS NOT NULL \
         ORDER BY resolution",
    )?;
    let resolutions: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(resolutions)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::library::schema::run_migrations;
    use crate::library::videos::{
        ingest_downloaded, CommentRecord, IngestPayload, TagRecord, VideoRecord,
    };

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        conn
    }

    fn video(
        id: &str,
        title: &str,
        duration: i64,
        uploader_id: Option<&str>,
        resolution: Option<&str>,
        posted_at: Option<i64>,
        view_count: Option<i64>,
    ) -> VideoRecord {
        VideoRecord {
            id: id.to_string(),
            title: title.to_string(),
            description: None,
            uploader_id: uploader_id.map(String::from),
            uploader_name: None,
            uploader_type: None,
            category: None,
            duration_sec: duration,
            posted_at,
            view_count,
            comment_count: None,
            mylist_count: None,
            thumbnail_url: None,
            video_path: Some(format!("videos/{id}/video.mp4")),
            raw_meta_json: None,
            resolution: resolution.map(String::from),
        }
    }

    fn seed_library(conn: &mut Connection) {
        let videos = [
            video(
                "sm1",
                "初音ミクの動画",
                200,
                Some("u1"),
                Some("1280x720"),
                Some(1_700_000_000),
                Some(1000),
            ),
            video(
                "sm2",
                "鏡音リンの楽曲",
                180,
                Some("u1"),
                Some("1920x1080"),
                Some(1_700_000_100),
                Some(500),
            ),
            video(
                "sm3",
                "演奏してみた",
                300,
                Some("u2"),
                Some("1280x720"),
                Some(1_700_000_200),
                Some(200),
            ),
            video(
                "sm4",
                "ゲーム実況プレイ",
                3600,
                Some("u3"),
                Some("3840x2160"),
                Some(1_700_000_300),
                Some(9999),
            ),
            video(
                "sm5",
                "歌ってみた",
                240,
                Some("u2"),
                None,
                Some(1_700_000_400),
                Some(300),
            ),
        ];
        let tag_sets: [Vec<TagRecord>; 5] = [
            vec![
                TagRecord {
                    name: "VOCALOID".into(),
                    is_locked: false,
                },
                TagRecord {
                    name: "初音ミク".into(),
                    is_locked: true,
                },
            ],
            vec![
                TagRecord {
                    name: "VOCALOID".into(),
                    is_locked: false,
                },
                TagRecord {
                    name: "鏡音リン".into(),
                    is_locked: false,
                },
            ],
            vec![TagRecord {
                name: "演奏してみた".into(),
                is_locked: false,
            }],
            vec![
                TagRecord {
                    name: "ゲーム".into(),
                    is_locked: false,
                },
                TagRecord {
                    name: "実況".into(),
                    is_locked: false,
                },
            ],
            vec![
                TagRecord {
                    name: "歌ってみた".into(),
                    is_locked: false,
                },
                TagRecord {
                    name: "VOCALOID".into(),
                    is_locked: false,
                },
            ],
        ];
        for (v, tags) in videos.iter().zip(tag_sets.iter()) {
            ingest_downloaded(
                conn,
                &IngestPayload {
                    video: v,
                    tags,
                    comments: &[],
                },
            )
            .unwrap();
        }

        // Add comments for sm1 to test FTS search.
        // Must pass the correct video_path and tags so the upsert doesn't
        // clear them (replace_tags_with_tx deletes official tags first).
        let sm1_for_comment = VideoRecord {
            id: "sm1".into(),
            title: "初音ミクの動画".into(),
            video_path: Some("videos/sm1/video.mp4".into()),
            duration_sec: 200,
            ..VideoRecord::default()
        };
        ingest_downloaded(
            conn,
            &IngestPayload {
                video: &sm1_for_comment,
                tags: &[
                    TagRecord {
                        name: "VOCALOID".into(),
                        is_locked: false,
                    },
                    TagRecord {
                        name: "初音ミク".into(),
                        is_locked: true,
                    },
                ],
                comments: &[CommentRecord {
                    no: 1,
                    vpos_ms: 0,
                    content: "これはすごい弾幕です".into(),
                    mail: None,
                    user_hash: Some("h1".into()),
                    is_owner: false,
                    posted_at: Some(1_700_000_010),
                }],
            },
        )
        .unwrap();
    }

    #[test]
    fn empty_library_returns_empty() {
        let conn = setup();
        let result = query_videos(&conn, &LibraryQuery::default()).unwrap();
        assert_eq!(result.total_count, 0);
        assert!(result.items.is_empty());
    }

    #[test]
    fn default_query_returns_all_videos() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(&conn, &LibraryQuery::default()).unwrap();
        assert_eq!(result.total_count, 5);
        assert_eq!(result.items.len(), 5);
    }

    #[test]
    fn sort_by_title_asc() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                sort_by: Some("title".into()),
                sort_order: Some("asc".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let titles: Vec<&str> = result.items.iter().map(|v| v.title.as_str()).collect();
        // Game実況プレイ → 歌ってみた → 鏡音リンの楽曲 → 初音ミクの動画 → 演奏してみた (unicode order)
        assert!(
            titles.windows(2).all(|w| w[0] <= w[1]),
            "titles should be sorted asc: {titles:?}"
        );
    }

    #[test]
    fn sort_by_duration_desc() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                sort_by: Some("duration_sec".into()),
                sort_order: Some("desc".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let durations: Vec<i64> = result.items.iter().map(|v| v.duration_sec).collect();
        assert_eq!(durations, vec![3600, 300, 240, 200, 180]);
    }

    #[test]
    fn pagination_works() {
        let mut conn = setup();
        seed_library(&mut conn);
        let page1 = query_videos(
            &conn,
            &LibraryQuery {
                offset: Some(0),
                limit: Some(2),
                sort_by: Some("duration_sec".into()),
                sort_order: Some("asc".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page1.total_count, 5);
        assert_eq!(page1.items.len(), 2);
        assert_eq!(page1.items[0].duration_sec, 180);
        assert_eq!(page1.items[1].duration_sec, 200);

        let page2 = query_videos(
            &conn,
            &LibraryQuery {
                offset: Some(2),
                limit: Some(2),
                sort_by: Some("duration_sec".into()),
                sort_order: Some("asc".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page2.items.len(), 2);
        assert_eq!(page2.items[0].duration_sec, 240);
    }

    #[test]
    fn filter_by_tag() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                tags: Some(vec!["VOCALOID".into()]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 3);
        let ids: Vec<&str> = result.items.iter().map(|v| v.id.as_str()).collect();
        assert!(ids.contains(&"sm1"));
        assert!(ids.contains(&"sm2"));
        assert!(ids.contains(&"sm5"));
    }

    #[test]
    fn filter_by_multiple_tags_is_and() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                tags: Some(vec!["VOCALOID".into(), "初音ミク".into()]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 1);
        assert_eq!(result.items[0].id, "sm1");
    }

    #[test]
    fn filter_by_uploader() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                uploader_id: Some("u2".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 2);
        let ids: Vec<&str> = result.items.iter().map(|v| v.id.as_str()).collect();
        assert!(ids.contains(&"sm3"));
        assert!(ids.contains(&"sm5"));
    }

    #[test]
    fn filter_by_duration_range() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                min_duration: Some(200),
                max_duration: Some(300),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 3); // sm1(200), sm3(300), sm5(240)
    }

    #[test]
    fn filter_by_resolution() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                resolution: Some("1280x720".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 2);
        let ids: Vec<&str> = result.items.iter().map(|v| v.id.as_str()).collect();
        assert!(ids.contains(&"sm1"));
        assert!(ids.contains(&"sm3"));
    }

    #[test]
    fn text_search_by_title() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                q: Some("ミク".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 1);
        assert_eq!(result.items[0].id, "sm1");
    }

    #[test]
    fn text_search_by_tag() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                q: Some("ゲーム".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 1);
        assert_eq!(result.items[0].id, "sm4");
    }

    #[test]
    fn text_search_by_comment_fts() {
        let mut conn = setup();
        seed_library(&mut conn);
        // "弾幕です" ≥ 3 chars → FTS5
        let result = query_videos(
            &conn,
            &LibraryQuery {
                q: Some("弾幕で".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 1);
        assert_eq!(result.items[0].id, "sm1");
    }

    #[test]
    fn short_query_skips_fts() {
        let mut conn = setup();
        seed_library(&mut conn);
        // 2-char query should still work (LIKE-only, no FTS)
        let result = query_videos(
            &conn,
            &LibraryQuery {
                q: Some("ミ".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 1);
    }

    #[test]
    fn combined_filters() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                tags: Some(vec!["VOCALOID".into()]),
                min_duration: Some(200),
                resolution: Some("1280x720".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 1);
        assert_eq!(result.items[0].id, "sm1");
    }

    #[test]
    fn tags_are_populated_in_results() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                sort_by: Some("title".into()),
                sort_order: Some("asc".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let sm1 = result.items.iter().find(|v| v.id == "sm1").unwrap();
        let mut tag_names = sm1.tags.clone();
        tag_names.sort();
        assert_eq!(tag_names, vec!["VOCALOID", "初音ミク"]);
    }

    #[test]
    fn limit_capped_at_500() {
        let conn = setup();
        let result = query_videos(
            &conn,
            &LibraryQuery {
                limit: Some(99999),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.limit, 500);
    }

    #[test]
    fn invalid_sort_column_rejected() {
        let conn = setup();
        let result = query_videos(
            &conn,
            &LibraryQuery {
                sort_by: Some("DROP TABLE videos".into()),
                ..Default::default()
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn stats_on_seeded_library() {
        let mut conn = setup();
        seed_library(&mut conn);
        let stats = get_stats(&conn).unwrap();
        assert_eq!(stats.total_videos, 5);
        assert_eq!(stats.total_duration_sec, 200 + 180 + 300 + 3600 + 240);
        assert_eq!(stats.unique_uploaders, 3);
        assert!(!stats.top_tags.is_empty());
        assert!(!stats.resolution_distribution.is_empty());
        // 1280x720 appears for sm1 and sm3
        let hd = stats
            .resolution_distribution
            .iter()
            .find(|r| r.resolution == "1280x720")
            .unwrap();
        assert_eq!(hd.count, 2);
    }

    #[test]
    fn stats_on_empty_library() {
        let conn = setup();
        let stats = get_stats(&conn).unwrap();
        assert_eq!(stats.total_videos, 0);
        assert_eq!(stats.total_duration_sec, 0);
        assert_eq!(stats.total_comments, 0);
        assert!(stats.top_tags.is_empty());
    }

    #[test]
    fn list_all_tags_returns_distinct_sorted() {
        let mut conn = setup();
        seed_library(&mut conn);
        let tags = list_all_tags(&conn).unwrap();
        assert!(tags.contains(&"VOCALOID".to_string()));
        assert!(tags.contains(&"初音ミク".to_string()));
        assert!(tags.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn list_resolutions_returns_distinct_sorted() {
        let mut conn = setup();
        seed_library(&mut conn);
        let res = list_resolutions(&conn).unwrap();
        assert_eq!(res, vec!["1280x720", "1920x1080", "3840x2160"]);
    }

    #[test]
    fn query_with_no_matching_filter_returns_empty() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                tags: Some(vec!["存在しないタグ".into()]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 0);
        assert!(result.items.is_empty());
    }
}
