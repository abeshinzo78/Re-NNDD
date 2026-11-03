//! チャンネルの投稿動画一覧。
//!
//! 以前使っていた `nvapi /v1/channels/{id}/videos` は 2026 年時点で 404 / 403
//! を返し使えない。チャンネルの動画一覧に JSON API は無く、ch.nicovideo.jp の
//! 一覧ページ (サーバー描画の HTML) を読む (NNDD-RE と同じ方針)。
//!
//! 1 ページ 20 件固定。並び替えは `sort` (f=投稿日時, v=再生数, r=コメント数,
//! m=マイリスト数, n=コメント日時, l=再生時間) と `order` (d=降順, a=昇順)。
//! `ch{数値}` の URL はチャンネル独自の名前 (`/chiifuyo/video` など) へ
//! リダイレクトされるが、reqwest はそのまま追う。

use std::sync::OnceLock;

use regex::Regex;

use crate::api::video::html_unescape;

/// 一覧ページ 1 枚分。
#[derive(Debug, Clone, Default)]
pub struct ChannelVideosPage {
    /// 一覧の総件数 (`<var>N</var>件`)。読めなければ 0。
    pub total_count: i64,
    pub items: Vec<ChannelVideo>,
    /// チャンネル名 (`og:site_name`)。
    pub channel_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelVideo {
    pub content_id: String,
    pub title: String,
    pub thumbnail_url: Option<String>,
    pub length_seconds: Option<i64>,
    pub view_counter: Option<i64>,
    pub comment_counter: Option<i64>,
    pub mylist_counter: Option<i64>,
    /// 投稿日時 (RFC 3339、JST)。
    pub start_time: Option<String>,
}

/// フロントの並び替えキー (`fetch_user_videos` の `sort_key` / `sort_order`) を
/// ch.nicovideo.jp の `sort` / `order` に読み替える。知らないキーは投稿日時。
pub fn channel_sort_params(sort_key: &str, sort_order: &str) -> (&'static str, &'static str) {
    let sort = match sort_key {
        "viewCount" => "v",
        "commentCount" => "r",
        "mylistCount" => "m",
        "lastCommentTime" => "n",
        "duration" | "lengthSeconds" => "l",
        _ => "f",
    };
    let order = if sort_order.eq_ignore_ascii_case("asc") {
        "a"
    } else {
        "d"
    };
    (sort, order)
}

/// 一覧ページの URL。`channel_id` は数値 (`2650183`)、`ch` 付き、どちらでもよい。
pub fn channel_videos_url(
    base: &str,
    channel_id: &str,
    page: u32,
    sort_key: &str,
    sort_order: &str,
) -> String {
    let id = channel_id.trim();
    let path_id = if id.chars().all(|c| c.is_ascii_digit()) {
        format!("ch{id}")
    } else {
        id.to_string()
    };
    let (sort, order) = channel_sort_params(sort_key, sort_order);
    format!(
        "{}/{path_id}/video?sort={sort}&order={order}&page={}",
        base.trim_end_matches('/'),
        page.max(1)
    )
}

macro_rules! static_regex {
    ($name:ident, $pattern:expr) => {
        fn $name() -> &'static Regex {
            // The pattern is a compile-time constant — `Regex::new` cannot
            // fail here. The targeted allow keeps the workspace `expect_used`
            // deny active for genuine production code paths.
            #[allow(clippy::expect_used)]
            fn build() -> Regex {
                Regex::new($pattern).expect("static regex compiles")
            }
            static RE: OnceLock<Regex> = OnceLock::new();
            RE.get_or_init(build)
        }
    };
}

static_regex!(
    item_regex,
    r#"(?s)<a[^>]*class="[^"]*\bwatchLink\b[^"]*"[^>]*href="https?://www\.nicovideo\.jp/watch/([a-z]{2}\d+)"[^>]*>(.*?)</a>"#
);
static_regex!(img_regex, r#"<img[^>]*\ssrc="([^"]+)""#);
static_regex!(
    length_regex,
    r#"data-style="videoLength"[^>]*>\s*([0-9:]+)\s*<"#
);
static_regex!(title_regex, r"(?s)<h3[^>]*>(.*?)</h3>");
static_regex!(count_regex, r"<dd>\s*([\d,]+)\s*</dd>");
static_regex!(
    date_regex,
    r"(\d{4})/(\d{2})/(\d{2})\([^)]*\)\s*(\d{2}):(\d{2})"
);
static_regex!(total_regex, r"<var>\s*([\d,]+)\s*</var>\s*件");
static_regex!(
    site_name_regex,
    r#"<meta[^>]*property="og:site_name"[^>]*content="([^"]*)""#
);
static_regex!(tag_regex, r"<[^>]*>");
static_regex!(space_regex, r"\s+");

fn parse_number(raw: &str) -> Option<i64> {
    raw.replace(',', "").parse().ok()
}

/// `23:40` / `1:02:03` を秒に。
fn parse_length(raw: &str) -> Option<i64> {
    let mut total = 0i64;
    for part in raw.split(':') {
        total = total
            .checked_mul(60)?
            .checked_add(part.parse::<i64>().ok()?)?;
    }
    Some(total)
}

fn clean_text(raw: &str) -> String {
    let without_tags = tag_regex().replace_all(raw, " ");
    let unescaped = html_unescape(&without_tags);
    space_regex()
        .replace_all(unescaped.trim(), " ")
        .into_owned()
}

/// 一覧ページの HTML を読む。動画が 1 件も読めなくてもエラーにはしない
/// (呼び出し側が総件数と合わせて判断する)。
pub fn parse_channel_videos_html(html: &str) -> ChannelVideosPage {
    let mut items = Vec::new();
    for cap in item_regex().captures_iter(html) {
        let (Some(id), Some(block)) = (cap.get(1), cap.get(2)) else {
            continue;
        };
        let block = block.as_str();
        let title = title_regex()
            .captures(block)
            .and_then(|c| c.get(1))
            .map(|m| clean_text(m.as_str()))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| id.as_str().to_string());
        let mut counts = count_regex()
            .captures_iter(block)
            .filter_map(|c| c.get(1).and_then(|m| parse_number(m.as_str())));
        let view_counter = counts.next();
        let comment_counter = counts.next();
        let mylist_counter = counts.next();
        let start_time = date_regex().captures_iter(block).last().and_then(|c| {
            let part = |i: usize| c.get(i).map(|m| m.as_str());
            Some(format!(
                "{}-{}-{}T{}:{}:00+09:00",
                part(1)?,
                part(2)?,
                part(3)?,
                part(4)?,
                part(5)?
            ))
        });
        items.push(ChannelVideo {
            content_id: id.as_str().to_string(),
            title,
            thumbnail_url: img_regex()
                .captures(block)
                .and_then(|c| c.get(1))
                .map(|m| html_unescape(m.as_str())),
            length_seconds: length_regex()
                .captures(block)
                .and_then(|c| c.get(1))
                .and_then(|m| parse_length(m.as_str())),
            view_counter,
            comment_counter,
            mylist_counter,
            start_time,
        });
    }
    let total_count = total_regex()
        .captures(html)
        .and_then(|c| c.get(1))
        .and_then(|m| parse_number(m.as_str()))
        .unwrap_or(items.len() as i64);
    let channel_name = site_name_regex()
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| html_unescape(m.as_str()).trim().to_string())
        .filter(|s| !s.is_empty());
    ChannelVideosPage {
        total_count,
        items,
        channel_name,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const PAGE: &str = include_str!("../../tests/fixtures/niconico/channel_videos_ch2650183.html");

    #[test]
    fn parses_the_channel_video_list() {
        let page = parse_channel_videos_html(PAGE);
        assert_eq!(page.total_count, 4);
        assert_eq!(page.items.len(), 4);
        assert!(page
            .channel_name
            .as_deref()
            .unwrap()
            .contains("チート付与魔術師"));

        let first = &page.items[0];
        assert_eq!(first.content_id, "so46975082");
        assert!(first.title.contains("第4話"), "{}", first.title);
        assert!(!first.title.contains('<'));
        assert_eq!(first.length_seconds, Some(23 * 60 + 40));
        assert!(first
            .thumbnail_url
            .as_deref()
            .unwrap()
            .starts_with("https://nicovideo.cdn.nimg.jp/thumbnails/46975082/"));
        assert!(first.view_counter.unwrap() > 0);
        assert!(first.comment_counter.unwrap() > 0);
        assert!(first.mylist_counter.is_some());
        assert_eq!(
            first.start_time.as_deref(),
            Some("2026-10-31T00:30:00+09:00")
        );
        // 新しい順に並んでいる。
        assert!(page.items.iter().all(|v| v.content_id.starts_with("so")));
    }

    #[test]
    fn builds_list_urls() {
        assert_eq!(
            channel_videos_url("https://ch.nicovideo.jp", "2650183", 2, "viewCount", "desc"),
            "https://ch.nicovideo.jp/ch2650183/video?sort=v&order=d&page=2"
        );
        assert_eq!(
            channel_videos_url(
                "https://ch.nicovideo.jp/",
                "ch2650183",
                0,
                "registeredAt",
                "asc"
            ),
            "https://ch.nicovideo.jp/ch2650183/video?sort=f&order=a&page=1"
        );
        assert_eq!(channel_sort_params("mylistCount", "desc"), ("m", "d"));
        assert_eq!(channel_sort_params("unknown", "x"), ("f", "d"));
    }

    #[test]
    fn parses_lengths() {
        assert_eq!(parse_length("23:40"), Some(1420));
        assert_eq!(parse_length("1:02:03"), Some(3723));
        assert_eq!(parse_length("x"), None);
    }

    #[test]
    fn empty_page_yields_no_items() {
        let page = parse_channel_videos_html("<html><body>0件</body></html>");
        assert!(page.items.is_empty());
        assert_eq!(page.total_count, 0);
    }
}
