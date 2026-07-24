//! Library query engine: search, sort, filter, and aggregate over local videos.
//!
//! All read-only. Write operations stay in [`super::videos`].

use rusqlite::{Connection, OptionalExtension};
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

    /// Tag filter (OR): include videos that have *any* of these tags.
    /// If both `tags` and `tags_any` are specified, the `tags` (AND)
    /// filter is applied first, then `tags_any` is applied on top.
    pub tags_any: Option<Vec<String>>,

    /// Uploader id filter.
    pub uploader_id: Option<String>,

    /// Minimum duration in seconds (inclusive).
    pub min_duration: Option<i64>,

    /// Maximum duration in seconds (inclusive).
    pub max_duration: Option<i64>,

    /// Resolution filter (exact match, e.g. "1280x720").
    pub resolution: Option<String>,

    /// ショート（縦長）動画に限定するフィルタ。None = 絞り込みなし。
    pub is_short: Option<bool>,

    /// Sort column. Defaults to `"downloaded_at"`.
    pub sort_by: Option<String>,

    /// `sort_by = "random"` のときの並びを固定する種。
    ///
    /// 素の `ORDER BY RANDOM()` はリクエストごとに並びが変わるので、ページを
    /// めくると同じ動画が再登場したり抜け落ちたりする。種が与えられた場合は
    /// rowid から決まる決定的な擬似ランダム順にして、ページ間で一貫させる。
    pub random_seed: Option<i64>,

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
    "is_short",
    "random",
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

/// ユーザ入力を FTS5 の **文字列リテラル** に包む。
///
/// `comments_fts MATCH ?` に生の入力を渡すと FTS5 のクエリ構文として解釈され、
/// `foo-bar` は `no such column: bar`、`AND` / `(` / 未対の `"` は syntax error
/// になる。UI は「コメント本文をそのまま検索する」体なので、常に 1 つの
/// フレーズとして扱う (trigram tokenizer なので部分一致になる)。
///
/// FTS5 の文字列リテラルは二重引用符で囲み、内側の `"` は `""` で表す。
fn fts_phrase(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

/// ランダム順の種を、実際に SQL へ渡す乗数へ変換する。
///
/// 法が素数なので `1..=法-1` の乗数なら `(rowid * 乗数) % 法` は単射になり、
/// 「ページをまたいで同じ動画が出る / 消える」が起きない。ただし乗数が小さい
/// と小さい rowid では剰余が一度も巻き戻らず rowid 昇順のままになるので、
/// Lehmer (MINSTD) で 2 段撹拌したうえで上位半分に寄せて、行数の少ない
/// ライブラリでも順序がばらけるようにする。1 段だと小さい種同士 (1, 7, 42…)
/// が似た乗数に落ちて同じ並びになってしまうため 2 段回す。
fn normalize_random_seed(seed: i64) -> i64 {
    let m = RANDOM_ORDER_MODULUS;
    let first = (seed.rem_euclid(m) + 1) * MINSTD_MULTIPLIER % m;
    let mut a = (first + 1) * MINSTD_MULTIPLIER % m;
    if a == 0 {
        a = MINSTD_MULTIPLIER;
    }
    if a < m / 2 {
        a += m / 2;
    }
    a
}

/// 2^31-1 (メルセンヌ素数)。rowid × 乗数が i64 に収まる範囲でもある。
const RANDOM_ORDER_MODULUS: i64 = 2_147_483_647;

/// MINSTD (Lehmer) の乗数。種の撹拌にだけ使う。
const MINSTD_MULTIPLIER: i64 = 48_271;

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
    pub is_short: bool,
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
                params.push(Box::new(fts_phrase(text)));
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

    if let Some(ref tags_any) = q.tags_any {
        if !tags_any.is_empty() {
            let placeholders: Vec<String> = tags_any
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let p = params.len() + i + 1;
                    format!("?{p}")
                })
                .collect();
            for tag in tags_any {
                params.push(Box::new(tag.clone()));
            }
            sql_where.push(format!(
                "EXISTS (SELECT 1 FROM tags t WHERE t.video_id = v.id AND t.name IN ({}))",
                placeholders.join(",")
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

    if let Some(is_short) = q.is_short {
        let p = params.len() + 1;
        params.push(Box::new(is_short as i64));
        sql_where.push(format!("v.is_short = ?{p}"));
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
    // ランダム順の種はページングの前に積む (LIMIT/OFFSET より前の位置)。
    let order_clause = if sort_by == "random" {
        match q.random_seed {
            // rowid は行ごとに異なり、法 2^31-1 は素数なので
            // `(rowid * seed) % p` は seed ≢ 0 の限り単射 = 並びが重複しない。
            // 同じ種なら毎回同じ順序になるのでページ送りしても取りこぼさない。
            Some(seed) => {
                let seed_p = params.len() + 1;
                params.push(Box::new(normalize_random_seed(seed)));
                format!("ORDER BY ((v.rowid * ?{seed_p}) % {RANDOM_ORDER_MODULUS}), v.id")
            }
            None => "ORDER BY RANDOM()".to_string(),
        }
    } else {
        format!("ORDER BY v.{sort_by} {order_sql}, v.id DESC")
    };

    let lim_p = params.len() + 1;
    let off_p = params.len() + 2;

    let data_sql = format!(
        "SELECT v.id, v.title, v.description, v.uploader_id, v.uploader_name, \
                v.uploader_type, v.category, v.duration_sec, v.posted_at, v.view_count, \
                v.comment_count, v.mylist_count, v.thumbnail_url, v.video_path, v.resolution, \
                v.downloaded_at, v.play_count, v.last_played_at, v.is_short \
         FROM videos v \
         {where_clause} \
         {order_clause} \
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
                is_short: row.get::<_, i64>(18)? != 0,
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

/// 動画 ID で 1 件取得する。プラグインの `library.get` action 用。
/// 存在しない / video_path が無い (= まだローカルに DL されていない) 行は
/// `None` を返す。
pub fn get_video_by_id(
    conn: &Connection,
    id: &str,
) -> Result<Option<LibraryVideoRow>, LibraryError> {
    let mut row: Option<LibraryVideoRow> = conn
        .query_row(
            "SELECT v.id, v.title, v.description, v.uploader_id, v.uploader_name, \
                    v.uploader_type, v.category, v.duration_sec, v.posted_at, v.view_count, \
                    v.comment_count, v.mylist_count, v.thumbnail_url, v.video_path, v.resolution, \
                    v.downloaded_at, v.play_count, v.last_played_at, v.is_short \
             FROM videos v \
             WHERE v.id = ?1 AND v.video_path IS NOT NULL",
            rusqlite::params![id],
            |r| {
                Ok(LibraryVideoRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    description: r.get(2)?,
                    uploader_id: r.get(3)?,
                    uploader_name: r.get(4)?,
                    uploader_type: r.get(5)?,
                    category: r.get(6)?,
                    duration_sec: r.get(7)?,
                    posted_at: r.get(8)?,
                    view_count: r.get(9)?,
                    comment_count: r.get(10)?,
                    mylist_count: r.get(11)?,
                    thumbnail_url: r.get(12)?,
                    video_path: r.get(13)?,
                    resolution: r.get(14)?,
                    downloaded_at: r.get(15)?,
                    play_count: r.get::<_, Option<i64>>(16)?.unwrap_or(0),
                    last_played_at: r.get(17)?,
                    tags: Vec::new(),
                    local_thumbnail_path: None,
                    is_short: r.get::<_, i64>(18)? != 0,
                })
            },
        )
        .optional()?;
    if let Some(ref mut v) = row {
        let mut stmt = conn.prepare("SELECT name FROM tags WHERE video_id = ?1 ORDER BY name")?;
        let tags: Vec<String> = stmt
            .query_map(rusqlite::params![v.id], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        v.tags = tags;
    }
    Ok(row)
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
// Comment full-text search
// ---------------------------------------------------------------------------

/// A single comment match from a library-wide comment search.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentSearchHit {
    pub video_id: String,
    pub video_title: String,
    pub comment_no: i64,
    pub vpos_ms: i64,
    pub content: String,
    pub user_hash: Option<String>,
    pub posted_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentSearchResult {
    pub items: Vec<CommentSearchHit>,
    pub total_count: i64,
    pub offset: u32,
    pub limit: u32,
}

/// Search local library comments via FTS5 trigram index.
///
/// `query` must be ≥ 3 characters (FTS5 trigram tokenizer requirement).
/// Returns comments from downloaded videos, newest snapshot per video.
pub fn search_comments(
    conn: &Connection,
    query: &str,
    offset: u32,
    limit: u32,
) -> Result<CommentSearchResult, LibraryError> {
    let limit = limit.min(200);
    // 入力はコメント本文の literal 検索。FTS5 構文として解釈させない。
    let match_arg = fts_phrase(query);

    let count_sql = "\
        SELECT COUNT(*) \
        FROM comments_fts fts \
        JOIN comments c ON c.id = fts.rowid \
        JOIN comment_snapshots cs ON cs.id = c.snapshot_id \
        JOIN videos v ON v.id = cs.video_id \
        WHERE v.video_path IS NOT NULL \
          AND comments_fts MATCH ?1";

    let total_count: i64 = conn.query_row(count_sql, [&match_arg], |row| row.get(0))?;

    let data_sql = "\
        SELECT v.id, v.title, c.no, c.vpos_ms, c.content, c.user_hash, c.posted_at \
        FROM comments_fts fts \
        JOIN comments c ON c.id = fts.rowid \
        JOIN comment_snapshots cs ON cs.id = c.snapshot_id \
        JOIN videos v ON v.id = cs.video_id \
        WHERE v.video_path IS NOT NULL \
          AND comments_fts MATCH ?1 \
        ORDER BY c.posted_at DESC \
        LIMIT ?2 OFFSET ?3";

    let mut stmt = conn.prepare(data_sql)?;
    let items: Vec<CommentSearchHit> = stmt
        .query_map(rusqlite::params![match_arg, limit, offset], |row| {
            Ok(CommentSearchHit {
                video_id: row.get(0)?,
                video_title: row.get(1)?,
                comment_no: row.get(2)?,
                vpos_ms: row.get(3)?,
                content: row.get(4)?,
                user_hash: row.get(5)?,
                posted_at: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(CommentSearchResult {
        items,
        total_count,
        offset,
        limit,
    })
}

// ---------------------------------------------------------------------------
// Uploader listing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploaderInfo {
    pub uploader_id: String,
    pub uploader_name: Option<String>,
    pub video_count: i64,
    pub total_duration_sec: i64,
    /// `"user"` / `"channel"`。オンライン検索へ条件を持ち出す時に
    /// `userId` と `channelId` のどちらで絞るかがこれで決まる。
    pub uploader_type: Option<String>,
}

/// List all uploaders with video counts, ordered by count desc.
pub fn list_uploaders(conn: &Connection, limit: u32) -> Result<Vec<UploaderInfo>, LibraryError> {
    let limit = limit.min(200);
    let mut stmt = conn.prepare(
        "SELECT uploader_id, uploader_name, COUNT(*) AS cnt, \
                COALESCE(SUM(duration_sec), 0) AS total_dur, \
                MAX(uploader_type) AS uploader_type \
         FROM videos \
         WHERE video_path IS NOT NULL AND uploader_id IS NOT NULL \
         GROUP BY uploader_id \
         ORDER BY cnt DESC \
         LIMIT ?1",
    )?;
    let rows: Vec<UploaderInfo> = stmt
        .query_map(rusqlite::params![limit], |row| {
            Ok(UploaderInfo {
                uploader_id: row.get(0)?,
                uploader_name: row.get(1)?,
                video_count: row.get(2)?,
                total_duration_sec: row.get(3)?,
                uploader_type: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

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
            is_short: false,
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

    #[test]
    fn filter_by_tags_any_returns_union() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                tags_any: Some(vec!["初音ミク".into(), "ゲーム".into()]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 2);
        let ids: Vec<&str> = result.items.iter().map(|v| v.id.as_str()).collect();
        assert!(ids.contains(&"sm1"));
        assert!(ids.contains(&"sm4"));
    }

    #[test]
    fn filter_by_tags_and_tags_any_combined() {
        let mut conn = setup();
        seed_library(&mut conn);
        // AND: VOCALOID, OR: 初音ミク or 鏡音リン
        let result = query_videos(
            &conn,
            &LibraryQuery {
                tags: Some(vec!["VOCALOID".into()]),
                tags_any: Some(vec!["初音ミク".into(), "鏡音リン".into()]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 2);
        let ids: Vec<&str> = result.items.iter().map(|v| v.id.as_str()).collect();
        assert!(ids.contains(&"sm1"));
        assert!(ids.contains(&"sm2"));
    }

    #[test]
    fn random_sort_returns_all_videos() {
        let mut conn = setup();
        seed_library(&mut conn);
        let result = query_videos(
            &conn,
            &LibraryQuery {
                sort_by: Some("random".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.total_count, 5);
        assert_eq!(result.items.len(), 5);
        // All IDs should be present, just in a random order.
        let ids: std::collections::HashSet<&str> =
            result.items.iter().map(|v| v.id.as_str()).collect();
        assert!(ids.contains("sm1"));
        assert!(ids.contains("sm2"));
        assert!(ids.contains("sm3"));
        assert!(ids.contains("sm4"));
        assert!(ids.contains("sm5"));
    }

    #[test]
    fn search_comments_finds_match() {
        let mut conn = setup();
        seed_library(&mut conn);
        // "弾幕です" was inserted as a comment for sm1
        let result = search_comments(&conn, "弾幕で", 0, 10).unwrap();
        assert_eq!(result.total_count, 1);
        assert_eq!(result.items[0].video_id, "sm1");
        assert_eq!(result.items[0].content, "これはすごい弾幕です");
    }

    #[test]
    fn search_comments_empty_library() {
        let conn = setup();
        let result = search_comments(&conn, "何か", 0, 10).unwrap();
        assert_eq!(result.total_count, 0);
        assert!(result.items.is_empty());
    }

    #[test]
    fn seeded_random_sort_is_stable_across_pages() {
        let mut conn = setup();
        seed_library(&mut conn);

        let page = |offset: u32| {
            query_videos(
                &conn,
                &LibraryQuery {
                    sort_by: Some("random".into()),
                    random_seed: Some(20_260_724),
                    limit: Some(2),
                    offset: Some(offset),
                    ..Default::default()
                },
            )
            .unwrap()
            .items
            .into_iter()
            .map(|v| v.id)
            .collect::<Vec<_>>()
        };

        // 同じ種ならページをまたいでも重複・欠落なく全件を辿れる。
        let mut seen: Vec<String> = Vec::new();
        for offset in [0, 2, 4] {
            seen.extend(page(offset));
        }
        let unique: std::collections::HashSet<&String> = seen.iter().collect();
        assert_eq!(seen.len(), 5);
        assert_eq!(unique.len(), 5, "ページ間で重複した: {seen:?}");

        // 同じ種の再実行は同じ順序。
        assert_eq!(page(0), page(0));
    }

    #[test]
    fn random_seed_edge_values_still_order_deterministically() {
        let mut conn = setup();
        seed_library(&mut conn);
        // 種 0 / 法の倍数 / 負値でも「全行が同じキー」にはならず全件返る。
        let ids = |seed: i64| {
            query_videos(
                &conn,
                &LibraryQuery {
                    sort_by: Some("random".into()),
                    random_seed: Some(seed),
                    ..Default::default()
                },
            )
            .unwrap()
            .items
            .into_iter()
            .map(|v| v.id)
            .collect::<Vec<_>>()
        };
        for seed in [0, RANDOM_ORDER_MODULUS, -1, i64::MIN] {
            assert_eq!(ids(seed).len(), 5, "seed={seed}");
        }
        assert_eq!(ids(0), ids(RANDOM_ORDER_MODULUS));
    }

    #[test]
    fn different_seeds_produce_different_orders() {
        let mut conn = setup();
        seed_library(&mut conn);
        let ids = |seed: i64| {
            query_videos(
                &conn,
                &LibraryQuery {
                    sort_by: Some("random".into()),
                    random_seed: Some(seed),
                    ..Default::default()
                },
            )
            .unwrap()
            .items
            .into_iter()
            .map(|v| v.id)
            .collect::<Vec<_>>()
        };
        // 小さい種でも rowid 昇順のままにならない (乗数を撹拌しているため)。
        let downloaded_order = query_videos(&conn, &LibraryQuery::default())
            .unwrap()
            .items
            .into_iter()
            .map(|v| v.id)
            .collect::<Vec<_>>();
        let orders: Vec<Vec<String>> = [1, 7, 999, 20_260_724].iter().map(|s| ids(*s)).collect();
        assert!(
            orders.iter().any(|o| *o != downloaded_order),
            "どの種でも既定順と同じ並びになっている: {orders:?}"
        );
        assert!(
            orders.windows(2).any(|w| w[0] != w[1]),
            "種を変えても並びが変わらない: {orders:?}"
        );
    }

    #[test]
    fn fts_syntax_characters_are_searched_literally() {
        let mut conn = setup();
        seed_library(&mut conn);
        // FTS5 のクエリ構文として解釈されると、以下はいずれも SQLite の
        // エラーになっていた (`no such column: bar` / syntax error 等)。
        for q in ["foo-bar", "AND", "(paren", "\"quote", "うぽつ OR ", "a*b"] {
            let comments = search_comments(&conn, q, 0, 10);
            assert!(
                comments.is_ok(),
                "search_comments({q:?}) が失敗: {comments:?}"
            );
            let videos = query_videos(
                &conn,
                &LibraryQuery {
                    q: Some(q.to_string()),
                    ..Default::default()
                },
            );
            assert!(videos.is_ok(), "query_videos(q={q:?}) が失敗: {videos:?}");
        }
    }

    #[test]
    fn fts_phrase_quotes_and_escapes() {
        assert_eq!(fts_phrase("foo-bar"), "\"foo-bar\"");
        assert_eq!(fts_phrase("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn literal_search_still_matches_stored_comments() {
        let mut conn = setup();
        seed_library(&mut conn);
        // 引用符で包んでもフレーズ (部分一致) 検索として機能する。
        let result = search_comments(&conn, "すごい弾幕", 0, 10).unwrap();
        assert_eq!(result.total_count, 1);
        assert_eq!(result.items[0].video_id, "sm1");
    }

    #[test]
    fn list_uploaders_reports_uploader_type() {
        let mut conn = setup();
        seed_library(&mut conn);
        // シードは uploader_type を入れないので、ここで channel に上書きする。
        conn.execute(
            "UPDATE videos SET uploader_type = 'channel' WHERE uploader_id = 'u3'",
            [],
        )
        .unwrap();
        let uploaders = list_uploaders(&conn, 50).unwrap();
        let u3 = uploaders.iter().find(|u| u.uploader_id == "u3").unwrap();
        assert_eq!(u3.uploader_type.as_deref(), Some("channel"));
        let u2 = uploaders.iter().find(|u| u.uploader_id == "u2").unwrap();
        assert_eq!(u2.uploader_type, None);
    }

    #[test]
    fn list_uploaders_returns_counts() {
        let mut conn = setup();
        seed_library(&mut conn);
        let uploaders = list_uploaders(&conn, 50).unwrap();
        // sm1 was re-ingested with ..VideoRecord::default() which set
        // uploader_id to None, so u1 only has sm2.
        assert_eq!(uploaders.len(), 3);
        let u1 = uploaders.iter().find(|u| u.uploader_id == "u1").unwrap();
        assert_eq!(u1.video_count, 1); // sm2 only (sm1 uploader_id was cleared)
        let u2 = uploaders.iter().find(|u| u.uploader_id == "u2").unwrap();
        assert_eq!(u2.video_count, 2); // sm3 + sm5
        let u3 = uploaders.iter().find(|u| u.uploader_id == "u3").unwrap();
        assert_eq!(u3.video_count, 1);
    }
}
