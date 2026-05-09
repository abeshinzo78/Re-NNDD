//! HLS m3u8 パーサ。
//!
//! 我々が niconico Domand から受け取るのは **MultiVariantPlaylist (master)** と
//! **MediaPlaylist (variant)** の 2 段構成。Domand は CMAF / fMP4 セグメント
//! (`*.cmfv`) を返し、暗号化されている場合は `EXT-X-KEY:METHOD=AES-128`、
//! init segment は `EXT-X-MAP:URI=...,BYTERANGE=...` で示される。
//!
//! 実装方針:
//! - hls.js 互換のフルパースは目指さない。Domand が出すサブセットだけを
//!   正確に拾う
//! - URI は base URL で resolve して常に絶対 URL で返す（呼び出し側の
//!   間違いを減らす）
//! - パースは pure function。HTTP は別モジュール
//!
//! 参考: RFC 8216 §4.3, §4.4

use url::Url;

use crate::error::ApiError;

#[derive(Debug, Clone, PartialEq)]
pub struct MasterPlaylist {
    pub variants: Vec<VariantStream>,
    /// `GROUP-ID -> AlternateMedia` の辞書。今回は AUDIO のみ拾う。
    pub audio_groups: std::collections::BTreeMap<String, Vec<AlternateMedia>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantStream {
    pub uri: String,
    pub bandwidth: Option<u64>,
    pub average_bandwidth: Option<u64>,
    pub resolution: Option<(u32, u32)>,
    pub codecs: Option<String>,
    pub audio_group: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlternateMedia {
    pub group_id: String,
    pub name: Option<String>,
    pub language: Option<String>,
    pub default: bool,
    pub uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaPlaylist {
    pub init_uri: Option<String>,
    pub init_byte_range: Option<ByteRange>,
    pub segments: Vec<Segment>,
    pub target_duration_sec: Option<u32>,
    pub end_list: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub uri: String,
    pub duration_sec: f64,
    pub byte_range: Option<ByteRange>,
    /// segment 適用時点で有効な暗号鍵情報。`METHOD=NONE` のときは `None`。
    pub key: Option<KeyInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyInfo {
    /// 通常 "AES-128"。"NONE" は KEY 解除指示で、Segment.key は None になる。
    pub method: String,
    pub uri: Option<String>,
    pub iv: Option<[u8; 16]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub length: u64,
    pub offset: u64,
}

/// Master playlist をパース。
pub fn parse_master(text: &str, base: &Url) -> Result<MasterPlaylist, ApiError> {
    let lines = normalize_lines(text);
    if lines.first().map(|s| s.as_str()) != Some("#EXTM3U") {
        return Err(ApiError::ResponseShape(
            "playlist does not start with #EXTM3U".into(),
        ));
    }

    struct PendingStreamInf {
        bandwidth: Option<u64>,
        average_bandwidth: Option<u64>,
        resolution: Option<(u32, u32)>,
        codecs: Option<String>,
        audio_group: Option<String>,
    }

    let mut variants: Vec<VariantStream> = Vec::new();
    let mut audio_groups: std::collections::BTreeMap<String, Vec<AlternateMedia>> =
        Default::default();
    let mut pending_stream_inf: Option<PendingStreamInf> = None;

    for line in lines.iter().skip(1) {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            let attrs = parse_attrs(rest);
            pending_stream_inf = Some(PendingStreamInf {
                bandwidth: attrs.get("BANDWIDTH").and_then(|v| v.parse::<u64>().ok()),
                average_bandwidth: attrs
                    .get("AVERAGE-BANDWIDTH")
                    .and_then(|v| v.parse::<u64>().ok()),
                resolution: attrs.get("RESOLUTION").and_then(|v| parse_resolution(v)),
                codecs: attrs.get("CODECS").map(|s| unquote(s).to_string()),
                audio_group: attrs.get("AUDIO").map(|s| unquote(s).to_string()),
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA:") {
            let attrs = parse_attrs(rest);
            if attrs.get("TYPE").map(|s| unquote(s)) == Some("AUDIO") {
                let group_id = attrs
                    .get("GROUP-ID")
                    .map(|s| unquote(s).to_string())
                    .unwrap_or_default();
                let media = AlternateMedia {
                    group_id: group_id.clone(),
                    name: attrs.get("NAME").map(|s| unquote(s).to_string()),
                    language: attrs.get("LANGUAGE").map(|s| unquote(s).to_string()),
                    default: attrs.get("DEFAULT").map(|s| unquote(s)) == Some("YES"),
                    uri: attrs
                        .get("URI")
                        .map(|s| resolve_uri(base, unquote(s)))
                        .transpose()?,
                };
                audio_groups.entry(group_id).or_default().push(media);
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        // URI 行: 直前の STREAM-INF と紐付ける
        if let Some(p) = pending_stream_inf.take() {
            let uri = resolve_uri(base, line)?;
            variants.push(VariantStream {
                uri,
                bandwidth: p.bandwidth,
                average_bandwidth: p.average_bandwidth,
                resolution: p.resolution,
                codecs: p.codecs,
                audio_group: p.audio_group,
            });
        }
    }

    // bandwidth 降順 (None は末尾)
    variants.sort_by_key(|v| std::cmp::Reverse(v.bandwidth.unwrap_or(0)));

    Ok(MasterPlaylist {
        variants,
        audio_groups,
    })
}

/// Media playlist をパース。`base` は media playlist 自身の URL。
pub fn parse_media(text: &str, base: &Url) -> Result<MediaPlaylist, ApiError> {
    let lines = normalize_lines(text);
    if lines.first().map(|s| s.as_str()) != Some("#EXTM3U") {
        return Err(ApiError::ResponseShape(
            "playlist does not start with #EXTM3U".into(),
        ));
    }

    let mut init_uri: Option<String> = None;
    let mut init_byte_range: Option<ByteRange> = None;
    let mut target_duration_sec: Option<u32> = None;
    let mut end_list = false;
    let mut current_key: Option<KeyInfo> = None;
    let mut pending_dur: Option<f64> = None;
    let mut pending_byte_range: Option<ByteRange> = None;
    let mut last_segment_end: u64 = 0;
    let mut segments: Vec<Segment> = Vec::new();

    for line in lines.iter().skip(1) {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            let attrs = parse_attrs(rest);
            if let Some(uri) = attrs.get("URI") {
                init_uri = Some(resolve_uri(base, unquote(uri))?);
            }
            if let Some(br) = attrs.get("BYTERANGE") {
                init_byte_range = parse_byte_range(unquote(br), 0).map(|(b, _)| b);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
            current_key = parse_key(rest, base)?;
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            target_duration_sec = rest.trim().parse::<u32>().ok();
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let dur_str = rest.split(',').next().unwrap_or("0").trim();
            pending_dur = Some(dur_str.parse::<f64>().unwrap_or(0.0));
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            // RFC 8216 §4.3.2.2: "n[@o]"; offset 省略時は前 segment の末尾。
            // EXTINF の前後どちらでも書けるので独立に保持する。
            if let Some((br, new_end)) = parse_byte_range(rest.trim(), last_segment_end) {
                pending_byte_range = Some(br);
                last_segment_end = new_end;
            }
            continue;
        }
        if line == "#EXT-X-ENDLIST" {
            end_list = true;
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        // URI 行 — pending を吸収して 1 segment 確定
        if let Some(dur) = pending_dur.take() {
            let uri = resolve_uri(base, line)?;
            segments.push(Segment {
                uri,
                duration_sec: dur,
                byte_range: pending_byte_range.take(),
                key: current_key.clone(),
            });
        }
    }

    Ok(MediaPlaylist {
        init_uri,
        init_byte_range,
        segments,
        target_duration_sec,
        end_list,
    })
}

fn parse_key(rest: &str, base: &Url) -> Result<Option<KeyInfo>, ApiError> {
    let attrs = parse_attrs(rest);
    let method = attrs
        .get("METHOD")
        .map(|s| unquote(s).to_string())
        .unwrap_or_else(|| "NONE".into());
    if method.eq_ignore_ascii_case("NONE") {
        return Ok(None);
    }
    let uri = attrs
        .get("URI")
        .map(|s| resolve_uri(base, unquote(s)))
        .transpose()?;
    let iv = attrs.get("IV").and_then(|v| parse_iv(unquote(v)));
    Ok(Some(KeyInfo { method, uri, iv }))
}

fn parse_iv(s: &str) -> Option<[u8; 16]> {
    let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    if hex.is_empty() || hex.len() > 32 {
        return None;
    }
    let padded = format!("{hex:0>32}");
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        let pair = &padded[i * 2..i * 2 + 2];
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

fn parse_resolution(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn parse_byte_range(s: &str, prev_end: u64) -> Option<(ByteRange, u64)> {
    let (length_str, offset_str) = match s.split_once('@') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    };
    let length: u64 = length_str.trim().parse().ok()?;
    let offset: u64 = match offset_str {
        Some(o) => o.trim().parse().ok()?,
        None => prev_end,
    };
    let new_end = offset + length;
    Some((ByteRange { length, offset }, new_end))
}

/// `K=V,K2="V,2",K3=V3` 形式の属性リストをパース。
fn parse_attrs(s: &str) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let mut chars = s.chars().peekable();
    while chars.peek().is_some() {
        // skip spaces
        while matches!(chars.peek(), Some(c) if c.is_whitespace()) {
            chars.next();
        }
        // key
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c == ',' {
                break;
            }
            key.push(c);
            chars.next();
        }
        if chars.peek() != Some(&'=') {
            // 値なし key (RFC 上想定外、無視)
            if chars.peek() == Some(&',') {
                chars.next();
            }
            continue;
        }
        // '='
        chars.next();
        // value: 引用符ありなら閉じ引用符まで、なしならカンマまで
        let mut val = String::new();
        if chars.peek() == Some(&'"') {
            val.push('"');
            chars.next();
            for c in chars.by_ref() {
                val.push(c);
                if c == '"' {
                    break;
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == ',' {
                    break;
                }
                val.push(c);
                chars.next();
            }
        }
        if chars.peek() == Some(&',') {
            chars.next();
        }
        out.insert(key.trim().to_string(), val.trim().to_string());
    }
    out
}

fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
}

fn normalize_lines(text: &str) -> Vec<String> {
    text.split('\n')
        .map(|line| line.trim_end_matches('\r').trim().to_string())
        .collect()
}

/// 相対 URI を base URL に対して resolve。絶対 URI ならそのまま返す。
pub fn resolve_uri(base: &Url, uri: &str) -> Result<String, ApiError> {
    let resolved = base
        .join(uri)
        .map_err(|e| ApiError::ResponseShape(format!("invalid uri {uri:?}: {e}")))?;
    Ok(resolved.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://delivery.domand.nicovideo.jp/v1/abcd/master.m3u8").unwrap()
    }

    #[test]
    fn parse_master_single_variant() {
        let m = "#EXTM3U\n\
                 #EXT-X-VERSION:6\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=1500000,RESOLUTION=1280x720,CODECS=\"avc1.4d401f\"\n\
                 video-720p.m3u8\n";
        let p = parse_master(m, &base()).unwrap();
        assert_eq!(p.variants.len(), 1);
        let v = &p.variants[0];
        assert_eq!(v.bandwidth, Some(1_500_000));
        assert_eq!(v.resolution, Some((1280, 720)));
        assert_eq!(v.codecs.as_deref(), Some("avc1.4d401f"));
        assert!(v.uri.ends_with("/v1/abcd/video-720p.m3u8"));
    }

    #[test]
    fn parse_master_sorts_by_bandwidth_desc() {
        let m = "#EXTM3U\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=640x360\n\
                 low.m3u8\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1920x1080\n\
                 high.m3u8\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=1500000,RESOLUTION=1280x720\n\
                 mid.m3u8\n";
        let p = parse_master(m, &base()).unwrap();
        let bs: Vec<_> = p.variants.iter().map(|v| v.bandwidth).collect();
        assert_eq!(bs, vec![Some(3_000_000), Some(1_500_000), Some(500_000)]);
    }

    #[test]
    fn parse_master_audio_group() {
        let m = "#EXTM3U\n\
                 #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"main\",DEFAULT=YES,URI=\"audio.m3u8\"\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=1500000,AUDIO=\"aac\"\n\
                 v.m3u8\n";
        let p = parse_master(m, &base()).unwrap();
        let media = p.audio_groups.get("aac").expect("aac group");
        assert_eq!(media.len(), 1);
        assert!(media[0].default);
        assert!(media[0].uri.as_deref().unwrap().ends_with("/audio.m3u8"));
        assert_eq!(p.variants[0].audio_group.as_deref(), Some("aac"));
    }

    #[test]
    fn parse_master_rejects_missing_extm3u() {
        let m = "#EXT-X-VERSION:6\nfoo.m3u8\n";
        assert!(parse_master(m, &base()).is_err());
    }

    fn media_base() -> Url {
        Url::parse("https://delivery.domand.nicovideo.jp/v1/abcd/q/720p/playlist.m3u8").unwrap()
    }

    #[test]
    fn parse_media_init_and_segments() {
        let m = "#EXTM3U\n\
                 #EXT-X-VERSION:7\n\
                 #EXT-X-TARGETDURATION:6\n\
                 #EXT-X-MAP:URI=\"init.cmfv\"\n\
                 #EXTINF:6.0,\n\
                 seg/0.cmfv\n\
                 #EXTINF:6.0,\n\
                 seg/1.cmfv\n\
                 #EXTINF:3.5,\n\
                 seg/2.cmfv\n\
                 #EXT-X-ENDLIST\n";
        let p = parse_media(m, &media_base()).unwrap();
        assert!(p.end_list);
        assert_eq!(p.target_duration_sec, Some(6));
        assert!(p.init_uri.as_deref().unwrap().ends_with("/720p/init.cmfv"));
        assert_eq!(p.segments.len(), 3);
        assert_eq!(p.segments[0].duration_sec, 6.0);
        assert_eq!(p.segments[2].duration_sec, 3.5);
        assert!(p.segments[1].uri.ends_with("/720p/seg/1.cmfv"));
    }

    #[test]
    fn parse_media_aes128_key_propagates_to_segments() {
        let m = "#EXTM3U\n\
                 #EXT-X-MAP:URI=\"init.cmfv\"\n\
                 #EXT-X-KEY:METHOD=AES-128,URI=\"key1\",IV=0x000102030405060708090A0B0C0D0E0F\n\
                 #EXTINF:6.0,\n\
                 s0.cmfv\n\
                 #EXTINF:6.0,\n\
                 s1.cmfv\n\
                 #EXT-X-KEY:METHOD=NONE\n\
                 #EXTINF:6.0,\n\
                 s2.cmfv\n";
        let p = parse_media(m, &media_base()).unwrap();
        let k0 = p.segments[0].key.as_ref().unwrap();
        assert_eq!(k0.method, "AES-128");
        assert!(k0.uri.as_deref().unwrap().ends_with("/720p/key1"));
        assert_eq!(k0.iv.unwrap()[0..3], [0x00, 0x01, 0x02]);
        assert_eq!(k0.iv.unwrap()[15], 0x0F);
        assert!(p.segments[1].key.is_some());
        assert!(p.segments[2].key.is_none());
    }

    #[test]
    fn parse_media_iv_short_hex_is_left_padded() {
        let iv = parse_iv("0x1").unwrap();
        assert_eq!(iv[..15], [0; 15]);
        assert_eq!(iv[15], 1);
    }

    #[test]
    fn parse_media_byterange_and_init_byterange() {
        let m = "#EXTM3U\n\
                 #EXT-X-MAP:URI=\"init.cmfv\",BYTERANGE=\"816@0\"\n\
                 #EXTINF:6.0,\n\
                 #EXT-X-BYTERANGE:1024@816\n\
                 main.cmfv\n\
                 #EXTINF:6.0,\n\
                 #EXT-X-BYTERANGE:2048\n\
                 main.cmfv\n";
        let p = parse_media(m, &media_base()).unwrap();
        let init_br = p.init_byte_range.unwrap();
        assert_eq!(init_br.length, 816);
        assert_eq!(init_br.offset, 0);

        let s0 = p.segments[0].byte_range.unwrap();
        assert_eq!(s0.length, 1024);
        assert_eq!(s0.offset, 816);

        // 2 つ目は offset 省略 → 直前 segment 末尾 (816+1024=1840) から
        let s1 = p.segments[1].byte_range.unwrap();
        assert_eq!(s1.length, 2048);
        assert_eq!(s1.offset, 1840);
    }

    #[test]
    fn parse_media_handles_crlf() {
        let m = "#EXTM3U\r\n\
                 #EXT-X-MAP:URI=\"init.cmfv\"\r\n\
                 #EXTINF:6.0,\r\n\
                 a.cmfv\r\n";
        let p = parse_media(m, &media_base()).unwrap();
        assert_eq!(p.segments.len(), 1);
        assert!(p.init_uri.is_some());
    }

    #[test]
    fn parse_media_skips_unknown_directives_and_comments() {
        let m = "#EXTM3U\n\
                 # this is a comment-style line\n\
                 #EXT-X-INDEPENDENT-SEGMENTS\n\
                 #EXT-X-MAP:URI=\"i.cmfv\"\n\
                 #EXTINF:6.0,\n\
                 a.cmfv\n";
        let p = parse_media(m, &media_base()).unwrap();
        assert_eq!(p.segments.len(), 1);
    }

    #[test]
    fn resolve_uri_relative_and_absolute() {
        let b = Url::parse("https://x.com/a/b/master.m3u8").unwrap();
        assert_eq!(
            resolve_uri(&b, "seg/1.cmfv").unwrap(),
            "https://x.com/a/b/seg/1.cmfv"
        );
        assert_eq!(
            resolve_uri(&b, "/abs/seg.cmfv").unwrap(),
            "https://x.com/abs/seg.cmfv"
        );
        assert_eq!(
            resolve_uri(&b, "https://other/seg.cmfv").unwrap(),
            "https://other/seg.cmfv"
        );
    }

    #[test]
    fn parse_attrs_handles_quoted_commas() {
        let attrs = parse_attrs("BANDWIDTH=1500000,CODECS=\"avc1.4d401f,mp4a.40.2\",RES=1x1");
        assert_eq!(attrs.get("BANDWIDTH").map(String::as_str), Some("1500000"));
        assert_eq!(
            attrs.get("CODECS").map(String::as_str),
            Some("\"avc1.4d401f,mp4a.40.2\""),
        );
        assert_eq!(attrs.get("RES").map(String::as_str), Some("1x1"));
    }
}
