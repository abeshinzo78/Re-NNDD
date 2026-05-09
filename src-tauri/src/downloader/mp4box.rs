//! ISO BMFF / MP4 box の最小限の読み書き。
//!
//! `mp4` クレートは fMP4（moof/mdat を扱う）に向かないので、必要な部分だけ
//! 自前で持つ。CMAF の mux に必要な操作:
//!
//! - top-level の box 列挙（ftyp / moov / moof / mdat / sidx / styp）
//! - container box の中身を再帰的に列挙
//! - 任意の box payload を `[size: u32 BE][type: 4ASCII][payload]` で書き出す
//! - tkhd / trex / tfhd の `track_ID` 書き換え
//! - mvhd の `next_track_ID` 書き換え
//!
//! Box format (RFC: ISO/IEC 14496-12):
//! ```text
//! [size: 4 BE][type: 4 ASCII]
//!   if size == 1: [largesize: 8 BE]   ← 64bit size
//!   if size == 0: extends to EOF      ← 我々は使わない (CMAF mdat は size 必須)
//! [payload: size - 8 (or - 16 if largesize)]
//! ```

use crate::error::ApiError;

#[derive(Debug, Clone, Copy)]
pub struct BoxRef<'a> {
    pub box_type: [u8; 4],
    pub payload: &'a [u8],
}

impl<'a> BoxRef<'a> {
    pub fn type_str(&self) -> String {
        String::from_utf8_lossy(&self.box_type).into_owned()
    }
}

/// 任意の `bytes` を頭から舐めて、top-level box を列挙する。
/// CMAF では size==0 (EOF まで) の box は出てこない前提。
pub fn iter_boxes(bytes: &[u8]) -> Result<Vec<BoxRef<'_>>, ApiError> {
    let mut out = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (header_len, total_len, box_type) = read_box_header(&bytes[offset..])?;
        let payload_start = offset + header_len;
        let end = offset + total_len;
        if end > bytes.len() {
            return Err(ApiError::ResponseShape(format!(
                "mp4 box {:?} extends past EOF (offset={offset}, total_len={total_len}, file_len={})",
                String::from_utf8_lossy(&box_type),
                bytes.len()
            )));
        }
        out.push(BoxRef {
            box_type,
            payload: &bytes[payload_start..end],
        });
        offset = end;
    }
    Ok(out)
}

/// container box の payload を再帰的に列挙したい時用。
/// 中身は "0 個以上の box の連結" 構造なので [`iter_boxes`] と同じパース。
pub fn iter_children(payload: &[u8]) -> Result<Vec<BoxRef<'_>>, ApiError> {
    iter_boxes(payload)
}

/// Box header をパースして `(header_len, total_len, type)` を返す。
/// header_len は payload までのバイト数 (8 または 16)。
/// total_len は size フィールドの値（ヘッダ込みのバイト総数）。
pub fn read_box_header(bytes: &[u8]) -> Result<(usize, usize, [u8; 4]), ApiError> {
    if bytes.len() < 8 {
        return Err(ApiError::ResponseShape("mp4 box header < 8 bytes".into()));
    }
    let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let mut box_type = [0u8; 4];
    box_type.copy_from_slice(&bytes[4..8]);
    if size == 1 {
        if bytes.len() < 16 {
            return Err(ApiError::ResponseShape(
                "mp4 largesize box header < 16 bytes".into(),
            ));
        }
        let large = u64::from_be_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        ]);
        if large < 16 {
            return Err(ApiError::ResponseShape(format!(
                "mp4 largesize {large} < 16"
            )));
        }
        Ok((16, large as usize, box_type))
    } else if size == 0 {
        Err(ApiError::ResponseShape(
            "mp4 box with size=0 (EOF-extending) is not supported".into(),
        ))
    } else {
        if size < 8 {
            return Err(ApiError::ResponseShape(format!("mp4 box size {size} < 8")));
        }
        Ok((8, size as usize, box_type))
    }
}

/// 1 つの box を `out` に書き出す。常に 32bit size を使う (payload < 4GiB の前提)。
pub fn write_box(out: &mut Vec<u8>, box_type: &[u8; 4], payload: &[u8]) {
    let total = (payload.len() + 8) as u32;
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(box_type);
    out.extend_from_slice(payload);
}

/// container box 用: 子 box 群を `(type, payload)` 列で受け取り、
/// それらを連結した payload を返す。
pub fn write_container_payload(children: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let total: usize = children.iter().map(|(_, p)| p.len() + 8).sum();
    let mut out = Vec::with_capacity(total);
    for (t, p) in children {
        write_box(&mut out, t, p);
    }
    out
}

/// container box 内から指定 type の最初の子を取り出す。
pub fn find_child<'a>(payload: &'a [u8], box_type: &[u8; 4]) -> Option<BoxRef<'a>> {
    iter_children(payload)
        .ok()?
        .into_iter()
        .find(|b| &b.box_type == box_type)
}

/// container box 内の指定 type の子を全部返す。
pub fn find_children<'a>(payload: &'a [u8], box_type: &[u8; 4]) -> Vec<BoxRef<'a>> {
    iter_children(payload)
        .unwrap_or_default()
        .into_iter()
        .filter(|b| &b.box_type == box_type)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn build_box(box_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write_box(&mut out, box_type, payload);
        out
    }

    #[test]
    fn write_then_iter_round_trip() {
        let a = build_box(b"ftyp", b"isom");
        let b = build_box(b"mdat", &[0u8; 100]);
        let mut combined = Vec::new();
        combined.extend_from_slice(&a);
        combined.extend_from_slice(&b);

        let boxes = iter_boxes(&combined).unwrap();
        assert_eq!(boxes.len(), 2);
        assert_eq!(&boxes[0].box_type, b"ftyp");
        assert_eq!(boxes[0].payload, b"isom");
        assert_eq!(&boxes[1].box_type, b"mdat");
        assert_eq!(boxes[1].payload.len(), 100);
    }

    #[test]
    fn container_children_round_trip() {
        let inner_a = b"AAAA";
        let inner_b = b"BBBBBB";
        let payload = write_container_payload(&[(b"trak", inner_a), (b"mvex", inner_b)]);
        let outer = build_box(b"moov", &payload);

        let top = iter_boxes(&outer).unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(&top[0].box_type, b"moov");

        let children = iter_children(top[0].payload).unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(&children[0].box_type, b"trak");
        assert_eq!(children[0].payload, inner_a);
        assert_eq!(&children[1].box_type, b"mvex");
        assert_eq!(children[1].payload, inner_b);

        let trak = find_child(top[0].payload, b"trak").unwrap();
        assert_eq!(trak.payload, inner_a);
        assert!(find_child(top[0].payload, b"NONE").is_none());
    }

    #[test]
    fn rejects_short_header() {
        let bytes = vec![0, 0, 0, 8];
        assert!(read_box_header(&bytes).is_err());
    }

    #[test]
    fn rejects_size_zero() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(b"mdat");
        assert!(read_box_header(&bytes).is_err());
    }

    #[test]
    fn handles_largesize_box() {
        // size=1 + type + 8byte largesize + payload
        let payload = b"abcd";
        let total = 16u64 + payload.len() as u64;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&total.to_be_bytes());
        bytes.extend_from_slice(payload);

        let (header_len, total_len, t) = read_box_header(&bytes).unwrap();
        assert_eq!(header_len, 16);
        assert_eq!(total_len, total as usize);
        assert_eq!(&t, b"mdat");
    }

    #[test]
    fn detects_box_extending_past_eof() {
        // declared total = 100, but only 16 bytes provided
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&100u32.to_be_bytes());
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&[0u8; 8]);
        assert!(iter_boxes(&bytes).is_err());
    }
}
