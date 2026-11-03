//! `ext.nicovideo.jp/api/getthumbinfo/{id}` — 旧来からある軽量な XML。
//!
//! タグ (ロック状態付き) をここから読む。watch データ (nvapi v4) より軽く、
//! 副作用も無いので、ランキングの NG 判定のように多数の動画のタグをまとめて
//! 取りたい時に向く。削除・非公開などは `status="fail"` で返る。

use std::sync::OnceLock;

use regex::Regex;

use crate::api::video::html_unescape;

/// `getthumbinfo` に付ける `Accept`。2026-10 時点で `application/xml` や
/// `text/xml` だけを指定すると 406 Not Acceptable が返る (`*/*` を含めれば 200)。
pub const ACCEPT: &str = "application/xml, text/xml, */*;q=0.8";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbTag {
    pub name: String,
    pub is_locked: bool,
}

fn status_regex() -> &'static Regex {
    // The pattern is a compile-time constant — `Regex::new` cannot fail here.
    #[allow(clippy::expect_used)]
    fn build() -> Regex {
        Regex::new(r#"<nicovideo_thumb_response[^>]*\bstatus="([a-z]+)""#)
            .expect("static regex compiles")
    }
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(build)
}

fn tags_block_regex() -> &'static Regex {
    // The pattern is a compile-time constant — `Regex::new` cannot fail here.
    #[allow(clippy::expect_used)]
    fn build() -> Regex {
        Regex::new(r#"(?s)<tags\b([^>]*)>(.*?)</tags>"#).expect("static regex compiles")
    }
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(build)
}

fn tag_regex() -> &'static Regex {
    // The pattern is a compile-time constant — `Regex::new` cannot fail here.
    #[allow(clippy::expect_used)]
    fn build() -> Regex {
        Regex::new(r#"(?s)<tag\b([^>]*)>(.*?)</tag>"#).expect("static regex compiles")
    }
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(build)
}

/// タグ一覧を読む。応答が `status="ok"` でなければ `None`。
///
/// `<tags domain="jp">` があればそれを、無ければ最初の `<tags>` を使う。
pub fn parse_thumbinfo_tags(xml: &str) -> Option<Vec<ThumbTag>> {
    let status = status_regex().captures(xml)?.get(1)?.as_str();
    if status != "ok" {
        return None;
    }
    let blocks: Vec<(&str, &str)> = tags_block_regex()
        .captures_iter(xml)
        .filter_map(|c| Some((c.get(1)?.as_str(), c.get(2)?.as_str())))
        .collect();
    let body = blocks
        .iter()
        .find(|(attrs, _)| attrs.contains(r#"domain="jp""#))
        .or_else(|| blocks.first())
        .map(|(_, body)| *body)
        .unwrap_or("");
    Some(
        tag_regex()
            .captures_iter(body)
            .filter_map(|c| {
                let attrs = c.get(1)?.as_str();
                let name = html_unescape(c.get(2)?.as_str()).trim().to_string();
                if name.is_empty() {
                    return None;
                }
                Some(ThumbTag {
                    name,
                    is_locked: attrs.contains(r#"lock="1""#),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn reads_tags_with_lock_state() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<nicovideo_thumb_response status="ok"><thumb><video_id>sm9</video_id>
<tags domain="jp"><tag lock="1">陰陽師</tag><tag lock="1">公式</tag><tag>3月6日投稿動画</tag><tag>R&amp;B</tag></tags>
</thumb></nicovideo_thumb_response>"#;
        let tags = parse_thumbinfo_tags(xml).unwrap();
        assert_eq!(tags.len(), 4);
        assert_eq!(
            tags[0],
            ThumbTag {
                name: "陰陽師".into(),
                is_locked: true
            }
        );
        assert!(!tags[2].is_locked);
        assert_eq!(tags[3].name, "R&B");
    }

    #[test]
    fn failure_responses_yield_none() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<nicovideo_thumb_response status="fail"><error><code>NOT_FOUND</code></error></nicovideo_thumb_response>"#;
        assert_eq!(parse_thumbinfo_tags(xml), None);
        assert_eq!(parse_thumbinfo_tags("<html></html>"), None);
    }

    #[test]
    fn ok_without_tags_is_an_empty_list() {
        let xml =
            r#"<nicovideo_thumb_response status="ok"><thumb></thumb></nicovideo_thumb_response>"#;
        assert_eq!(parse_thumbinfo_tags(xml), Some(Vec::new()));
    }
}
