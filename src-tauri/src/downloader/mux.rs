//! 映像 fMP4 + 音声 fMP4 → 単一 fMP4 への合成。
//!
//! niconico Domand の CMAF は次の構成:
//! - 映像 / 音声で別 m3u8 → 別 init segment + 別 media segments
//! - 各 init segment は `ftyp` + `moov` (1 track のみ)
//! - 各 media segment は `[styp]? + moof + mdat`
//! - tfhd は `default-base-is-moof` フラグを立てている前提（CMAF spec MUST）。
//!   なので moof+mdat ペアをファイル内のどこに置いても trun.data_offset は
//!   moof 起点のままで正しいまま使える。
//!
//! 合成手順:
//! 1. 映像の `ftyp` をそのまま採用
//! 2. 新しい `moov` を構築:
//!    - 映像の `mvhd` を採用（`next_track_ID = 3` に書き換え）
//!    - `trak` × 2: 映像 (track_id=1) + 音声 (track_id=2 へリナンバ)
//!    - `mvex.trex` × 2: 同様に track_id を 1, 2 に揃える
//! 3. 出力に書き出す:
//!    - 新 `ftyp`
//!    - 新 `moov`
//!    - 映像ファイル内の `moof+mdat` (`styp` 等もそのまま)
//!    - 音声ファイル内の `moof+mdat` (`tfhd.track_id = 2` に書き換え)
//!
//! 結果は 1 ファイル fragmented MP4 で、HTML5 `<video>` / VLC / mpv で再生可能。

use std::path::Path;

use crate::error::ApiError;

use super::mp4box::{
    find_child, iter_boxes, iter_children, write_box, write_container_payload, BoxRef,
};

const VIDEO_TRACK_ID: u32 = 1;
const AUDIO_TRACK_ID: u32 = 2;

/// メイン entry point: 映像 / 音声 fMP4 を読んで `output_path` に合成出力。
pub fn mux_video_and_audio(
    video_path: &Path,
    audio_path: &Path,
    output_path: &Path,
) -> Result<u64, ApiError> {
    let video_bytes = std::fs::read(video_path)?;
    let audio_bytes = std::fs::read(audio_path)?;
    let combined = mux_bytes(&video_bytes, &audio_bytes)?;
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output_path, &combined)?;
    Ok(combined.len() as u64)
}

/// テスト/in-memory 用: バイト列入力 → バイト列出力。
pub fn mux_bytes(video_bytes: &[u8], audio_bytes: &[u8]) -> Result<Vec<u8>, ApiError> {
    let video_top = iter_boxes(video_bytes)?;
    let audio_top = iter_boxes(audio_bytes)?;

    let video_ftyp = find_top(&video_top, b"ftyp").ok_or_else(|| {
        ApiError::ResponseShape("video file missing ftyp".into())
    })?;
    let video_moov = find_top(&video_top, b"moov").ok_or_else(|| {
        ApiError::ResponseShape("video file missing moov".into())
    })?;
    let audio_moov = find_top(&audio_top, b"moov").ok_or_else(|| {
        ApiError::ResponseShape("audio file missing moov".into())
    })?;

    let combined_moov = build_combined_moov(video_moov.payload, audio_moov.payload)?;

    let mut out: Vec<u8> = Vec::with_capacity(video_bytes.len() + audio_bytes.len());
    write_box(&mut out, b"ftyp", video_ftyp.payload);
    write_box(&mut out, b"moov", &combined_moov);

    // 映像側の moof+mdat / styp 等はそのままコピー
    for b in &video_top {
        match &b.box_type {
            b"ftyp" | b"moov" | b"sidx" => continue,
            b"moof" => {
                // 念のため track_id を 1 へ揃える（既に 1 のはずだが、別 ID で
                // 出てくる Domand バリエーションへの保険）
                let rewritten = rewrite_track_id_in_moof(b.payload, VIDEO_TRACK_ID)?;
                write_box(&mut out, b"moof", &rewritten);
            }
            _ => write_box(&mut out, &b.box_type, b.payload),
        }
    }
    // 音声側: moof の tfhd.track_id を 2 に書き換えてコピー
    for b in &audio_top {
        match &b.box_type {
            b"ftyp" | b"moov" | b"sidx" => continue,
            b"moof" => {
                let rewritten = rewrite_track_id_in_moof(b.payload, AUDIO_TRACK_ID)?;
                write_box(&mut out, b"moof", &rewritten);
            }
            _ => write_box(&mut out, &b.box_type, b.payload),
        }
    }

    Ok(out)
}

fn find_top<'a>(boxes: &'a [BoxRef<'a>], typ: &[u8; 4]) -> Option<BoxRef<'a>> {
    boxes.iter().find(|b| &b.box_type == typ).copied()
}

fn build_combined_moov(
    video_moov: &[u8],
    audio_moov: &[u8],
) -> Result<Vec<u8>, ApiError> {
    let mvhd = find_child(video_moov, b"mvhd")
        .ok_or_else(|| ApiError::ResponseShape("video moov missing mvhd".into()))?
        .payload
        .to_vec();
    let updated_mvhd = update_mvhd_next_track_id(&mvhd, AUDIO_TRACK_ID + 1)?;

    let video_trak = find_child(video_moov, b"trak").ok_or_else(|| {
        ApiError::ResponseShape("video moov missing trak".into())
    })?;
    let video_trak_renum = rewrite_track_id_in_trak(video_trak.payload, VIDEO_TRACK_ID)?;

    let audio_trak = find_child(audio_moov, b"trak").ok_or_else(|| {
        ApiError::ResponseShape("audio moov missing trak".into())
    })?;
    let audio_trak_renum = rewrite_track_id_in_trak(audio_trak.payload, AUDIO_TRACK_ID)?;

    // mvex 統合
    let mvex_payload = {
        let video_mvex = find_child(video_moov, b"mvex");
        let audio_mvex = find_child(audio_moov, b"mvex");
        let mut children: Vec<([u8; 4], Vec<u8>)> = Vec::new();
        // 1 つ目の mvex の mehd / その他の non-trex 子はそのまま採用
        if let Some(vmvex) = video_mvex {
            for child in iter_children(vmvex.payload)? {
                if &child.box_type == b"trex" {
                    let renum = rewrite_track_id_in_trex(child.payload, VIDEO_TRACK_ID)?;
                    children.push((*b"trex", renum));
                } else {
                    children.push((child.box_type, child.payload.to_vec()));
                }
            }
        }
        if let Some(amvex) = audio_mvex {
            for child in iter_children(amvex.payload)? {
                if &child.box_type == b"trex" {
                    let renum = rewrite_track_id_in_trex(child.payload, AUDIO_TRACK_ID)?;
                    children.push((*b"trex", renum));
                }
                // 音声側の mehd は重複なので捨てる
            }
        }
        let refs: Vec<(&[u8; 4], &[u8])> =
            children.iter().map(|(t, p)| (t, p.as_slice())).collect();
        write_container_payload(&refs)
    };

    let moov_children: Vec<(&[u8; 4], &[u8])> = vec![
        (b"mvhd", &updated_mvhd),
        (b"trak", &video_trak_renum),
        (b"trak", &audio_trak_renum),
        (b"mvex", &mvex_payload),
    ];
    Ok(write_container_payload(&moov_children))
}

/// trak payload 内の tkhd を見つけて `track_ID` を書き換える。
fn rewrite_track_id_in_trak(trak_payload: &[u8], new_id: u32) -> Result<Vec<u8>, ApiError> {
    let mut new_children: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for child in iter_children(trak_payload)? {
        if &child.box_type == b"tkhd" {
            new_children.push((*b"tkhd", rewrite_tkhd_track_id(child.payload, new_id)?));
        } else {
            new_children.push((child.box_type, child.payload.to_vec()));
        }
    }
    let refs: Vec<(&[u8; 4], &[u8])> =
        new_children.iter().map(|(t, p)| (t, p.as_slice())).collect();
    Ok(write_container_payload(&refs))
}

fn rewrite_tkhd_track_id(tkhd_payload: &[u8], new_id: u32) -> Result<Vec<u8>, ApiError> {
    if tkhd_payload.len() < 24 {
        return Err(ApiError::ResponseShape(format!(
            "tkhd payload too short ({} bytes)",
            tkhd_payload.len()
        )));
    }
    let version = tkhd_payload[0];
    // payload 構造:
    //   [version: 1][flags: 3]
    //   v0: [creation 4][modification 4][track_ID 4]...
    //   v1: [creation 8][modification 8][track_ID 4]...
    let track_id_offset = match version {
        0 => 4 + 4 + 4,
        1 => 4 + 8 + 8,
        v => {
            return Err(ApiError::ResponseShape(format!("unsupported tkhd version {v}")));
        }
    };
    if tkhd_payload.len() < track_id_offset + 4 {
        return Err(ApiError::ResponseShape(
            "tkhd payload too short for track_ID field".into(),
        ));
    }
    let mut out = tkhd_payload.to_vec();
    out[track_id_offset..track_id_offset + 4].copy_from_slice(&new_id.to_be_bytes());
    Ok(out)
}

fn rewrite_track_id_in_trex(trex_payload: &[u8], new_id: u32) -> Result<Vec<u8>, ApiError> {
    if trex_payload.len() < 8 {
        return Err(ApiError::ResponseShape(
            "trex payload too short".into(),
        ));
    }
    // [version: 1][flags: 3][track_ID: 4]...
    let mut out = trex_payload.to_vec();
    out[4..8].copy_from_slice(&new_id.to_be_bytes());
    Ok(out)
}

fn update_mvhd_next_track_id(mvhd_payload: &[u8], next_id: u32) -> Result<Vec<u8>, ApiError> {
    if mvhd_payload.len() < 4 {
        return Err(ApiError::ResponseShape(
            "mvhd payload too short".into(),
        ));
    }
    // next_track_ID は payload の最後の 4 byte。
    let mut out = mvhd_payload.to_vec();
    let n = out.len();
    out[n - 4..].copy_from_slice(&next_id.to_be_bytes());
    Ok(out)
}

/// moof payload 内の各 traf -> tfhd の `track_ID` を書き換える。
fn rewrite_track_id_in_moof(moof_payload: &[u8], new_id: u32) -> Result<Vec<u8>, ApiError> {
    let mut new_children: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for child in iter_children(moof_payload)? {
        if &child.box_type == b"traf" {
            new_children.push((*b"traf", rewrite_track_id_in_traf(child.payload, new_id)?));
        } else {
            new_children.push((child.box_type, child.payload.to_vec()));
        }
    }
    let refs: Vec<(&[u8; 4], &[u8])> =
        new_children.iter().map(|(t, p)| (t, p.as_slice())).collect();
    Ok(write_container_payload(&refs))
}

fn rewrite_track_id_in_traf(traf_payload: &[u8], new_id: u32) -> Result<Vec<u8>, ApiError> {
    let mut new_children: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for child in iter_children(traf_payload)? {
        if &child.box_type == b"tfhd" {
            if child.payload.len() < 8 {
                return Err(ApiError::ResponseShape("tfhd payload too short".into()));
            }
            // [version: 1][flags: 3][track_ID: 4]...
            let mut tfhd = child.payload.to_vec();
            tfhd[4..8].copy_from_slice(&new_id.to_be_bytes());
            new_children.push((*b"tfhd", tfhd));
        } else {
            new_children.push((child.box_type, child.payload.to_vec()));
        }
    }
    let refs: Vec<(&[u8; 4], &[u8])> =
        new_children.iter().map(|(t, p)| (t, p.as_slice())).collect();
    Ok(write_container_payload(&refs))
}


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::downloader::mp4box::{find_child as fc, find_children as fcs, iter_boxes as ib};

    fn b(t: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write_box(&mut out, t, payload);
        out
    }

    fn container(t: &[u8; 4], children: Vec<Vec<u8>>) -> Vec<u8> {
        let mut payload = Vec::new();
        for c in children {
            payload.extend_from_slice(&c);
        }
        b(t, &payload)
    }

    /// version=0 の最小 tkhd を作る。track_id を含む。
    fn fake_tkhd(track_id: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.push(0); // version
        payload.extend_from_slice(&[0, 0, 0]); // flags
        payload.extend_from_slice(&0u32.to_be_bytes()); // creation
        payload.extend_from_slice(&0u32.to_be_bytes()); // modification
        payload.extend_from_slice(&track_id.to_be_bytes()); // track_ID
        payload.extend_from_slice(&[0u8; 4]); // reserved
        payload.extend_from_slice(&100u32.to_be_bytes()); // duration
        // pad to typical tkhd size
        payload.extend_from_slice(&[0u8; 60]);
        b(b"tkhd", &payload)
    }

    fn fake_mvhd() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.push(0); // version
        payload.extend_from_slice(&[0, 0, 0]); // flags
        // dummy fields total 96 bytes, then next_track_ID at the very end
        payload.extend_from_slice(&[0u8; 96]);
        payload.extend_from_slice(&999u32.to_be_bytes()); // next_track_ID
        b(b"mvhd", &payload)
    }

    fn fake_trex(track_id: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.push(0); // version
        payload.extend_from_slice(&[0, 0, 0]); // flags
        payload.extend_from_slice(&track_id.to_be_bytes()); // track_ID
        payload.extend_from_slice(&1u32.to_be_bytes()); // default_sample_description_index
        payload.extend_from_slice(&0u32.to_be_bytes()); // default_sample_duration
        payload.extend_from_slice(&0u32.to_be_bytes()); // default_sample_size
        payload.extend_from_slice(&0u32.to_be_bytes()); // default_sample_flags
        b(b"trex", &payload)
    }

    fn fake_tfhd(track_id: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.push(0); // version
        payload.extend_from_slice(&[0, 0, 0]); // flags
        payload.extend_from_slice(&track_id.to_be_bytes()); // track_ID
        b(b"tfhd", &payload)
    }

    fn fake_init(track_id: u32) -> Vec<u8> {
        let ftyp = b(b"ftyp", b"isom");
        let mvhd = fake_mvhd();
        let tkhd = fake_tkhd(track_id);
        let trak_payload = {
            let mut p = Vec::new();
            p.extend_from_slice(&tkhd);
            p
        };
        let trak = b(b"trak", &trak_payload);
        let trex = fake_trex(track_id);
        let mvex = container(b"mvex", vec![trex]);
        let moov = container(b"moov", vec![mvhd, trak, mvex]);
        let mut out = Vec::new();
        out.extend_from_slice(&ftyp);
        out.extend_from_slice(&moov);
        out
    }

    fn fake_segment(track_id: u32, mdat_byte: u8, mdat_len: usize) -> Vec<u8> {
        let tfhd = fake_tfhd(track_id);
        let traf = container(b"traf", vec![tfhd]);
        let moof_payload = {
            // mfhd + traf
            let mut p = Vec::new();
            // mfhd: [v 1][flags 3][seq 4]
            let mut mfhd_payload = Vec::new();
            mfhd_payload.push(0);
            mfhd_payload.extend_from_slice(&[0, 0, 0]);
            mfhd_payload.extend_from_slice(&1u32.to_be_bytes());
            p.extend_from_slice(&b(b"mfhd", &mfhd_payload));
            p.extend_from_slice(&traf);
            p
        };
        let moof = b(b"moof", &moof_payload);
        let mdat = b(b"mdat", &vec![mdat_byte; mdat_len]);
        let mut out = Vec::new();
        out.extend_from_slice(&moof);
        out.extend_from_slice(&mdat);
        out
    }

    #[test]
    fn mux_combines_two_moovs_and_renumbers_audio() {
        let mut video = fake_init(1);
        video.extend_from_slice(&fake_segment(1, 0xAA, 100));
        video.extend_from_slice(&fake_segment(1, 0xAB, 50));

        let mut audio = fake_init(1);
        audio.extend_from_slice(&fake_segment(1, 0x55, 80));

        let combined = mux_bytes(&video, &audio).unwrap();
        let top = ib(&combined).unwrap();

        // ftyp と moov があり、その後に moof+mdat が並ぶ
        assert_eq!(&top[0].box_type, b"ftyp");
        assert_eq!(&top[1].box_type, b"moov");

        // moov に trak が 2 つ、mvex.trex が 2 つ
        let moov = top[1];
        let traks = fcs(moov.payload, b"trak");
        assert_eq!(traks.len(), 2);

        // 1 つ目の trak の tkhd track_id == 1, 2 つ目 == 2
        let video_trak_tkhd = fc(traks[0].payload, b"tkhd").unwrap();
        assert_eq!(read_tkhd_track_id(video_trak_tkhd.payload), 1);
        let audio_trak_tkhd = fc(traks[1].payload, b"tkhd").unwrap();
        assert_eq!(read_tkhd_track_id(audio_trak_tkhd.payload), 2);

        // mvex の trex 2 つ
        let mvex = fc(moov.payload, b"mvex").unwrap();
        let trexes = fcs(mvex.payload, b"trex");
        assert_eq!(trexes.len(), 2);
        assert_eq!(read_trex_track_id(trexes[0].payload), 1);
        assert_eq!(read_trex_track_id(trexes[1].payload), 2);

        // mvhd.next_track_ID が 3 に更新されている
        let mvhd = fc(moov.payload, b"mvhd").unwrap();
        let n = mvhd.payload.len();
        let next = u32::from_be_bytes(mvhd.payload[n - 4..n].try_into().unwrap());
        assert_eq!(next, 3);

        // moof+mdat: video×2 + audio×1 = 3 ペア
        let moofs: Vec<_> = top.iter().filter(|b| &b.box_type == b"moof").collect();
        let mdats: Vec<_> = top.iter().filter(|b| &b.box_type == b"mdat").collect();
        assert_eq!(moofs.len(), 3);
        assert_eq!(mdats.len(), 3);

        // 最後の moof は audio 側 → tfhd.track_id == 2
        let last_moof = moofs.last().unwrap();
        let traf = fc(last_moof.payload, b"traf").unwrap();
        let tfhd = fc(traf.payload, b"tfhd").unwrap();
        let tid = u32::from_be_bytes(tfhd.payload[4..8].try_into().unwrap());
        assert_eq!(tid, 2);

        // mdat のバイトパターンも見ておく: video 0xAA, 0xAB, audio 0x55 の順
        assert_eq!(mdats[0].payload[0], 0xAA);
        assert_eq!(mdats[1].payload[0], 0xAB);
        assert_eq!(mdats[2].payload[0], 0x55);
    }

    fn read_tkhd_track_id(payload: &[u8]) -> u32 {
        // version=0 前提
        u32::from_be_bytes(payload[12..16].try_into().unwrap())
    }
    fn read_trex_track_id(payload: &[u8]) -> u32 {
        u32::from_be_bytes(payload[4..8].try_into().unwrap())
    }

    #[test]
    fn rejects_video_without_moov() {
        let video = b(b"ftyp", b"isom");
        let audio = fake_init(1);
        assert!(mux_bytes(&video, &audio).is_err());
    }
}
