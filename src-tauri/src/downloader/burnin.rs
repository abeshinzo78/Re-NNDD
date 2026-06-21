//! コメント焼き込みの ffmpeg ストリーミングセッション。
//!
//! フロント (WebView) が `@xpadev-net/niconicomments` で描いた PNG フレームを
//! 1 枚ずつ stdin (`image2pipe`) へ流し込み、元動画へオーバーレイした MP4 を
//! 書き出す。これは niconicomments-convert の `converter.ts` /
//! `ffmpeg-stream/stream.ts` と同じ構成で、**描画は niconicomments 本体に
//! 完全委譲し、合成だけ ffmpeg が行う**。旧来の独自 ASS 生成は廃止した。
//!
//! ffmpeg のフィルタグラフは niconicomments-convert の `defaultOptions` を踏襲する:
//! 元動画を 16:9 に pad → 出力解像度へ scale し、コメント PNG を bt601→bt709 で
//! alphamerge して overlay する。これにより本物の niconico / convert と同じ
//! 座標・色で焼き込まれる。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::Mutex as AsyncMutex;

use crate::downloader::tools;
use crate::error::ApiError;

/// `burnin_feed` のバイナリフレーム種別。
pub const FLAG_FRAME: u8 = 0;
pub const FLAG_EMPTY: u8 = 1;
pub const FLAG_SET_EMPTY: u8 = 2;

/// 1 焼き込みセッション。ffmpeg child と、そこへ書き込む stdin を保持する。
pub struct BurnInSession {
    stdin: Option<ChildStdin>,
    child: Child,
    stderr: Arc<AsyncMutex<String>>,
    /// 透明フレームの PNG バイト列 (1 度だけ転送して使い回す)。
    empty_png: Option<Vec<u8>>,
    pub output_path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub total_frames: u64,
    pub fed: u64,
}

impl BurnInSession {
    /// 透明フレームの PNG を登録する。
    pub fn set_empty(&mut self, png: Vec<u8>) {
        self.empty_png = Some(png);
    }

    /// PNG 1 枚を ffmpeg stdin へ書き込む。
    pub async fn write_frame(&mut self, png: &[u8]) -> Result<(), ApiError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| ApiError::Downloader("burn-in session already closed".into()))?;
        stdin
            .write_all(png)
            .await
            .map_err(|e| ApiError::Downloader(format!("write frame to ffmpeg: {e}")))?;
        self.fed += 1;
        Ok(())
    }

    /// 透明フレームを 1 枚書き込む。
    pub async fn write_empty(&mut self) -> Result<(), ApiError> {
        let png = self
            .empty_png
            .clone()
            .ok_or_else(|| ApiError::Downloader("empty frame not initialized".into()))?;
        self.write_frame(&png).await
    }

    /// stdin を閉じて ffmpeg の終了を待つ。成功すれば `Ok(())`。
    pub async fn finish(&mut self) -> Result<(), ApiError> {
        // stdin を drop して EOF を送る → ffmpeg が encode を完了する。
        self.stdin.take();
        let status = self
            .child
            .wait()
            .await
            .map_err(|e| ApiError::Downloader(format!("wait ffmpeg: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            let stderr = self.stderr.lock().await.clone();
            Err(ApiError::Downloader(format!(
                "ffmpeg failed:\n{}",
                stderr.lines().take(30).collect::<Vec<_>>().join("\n")
            )))
        }
    }

    /// ffmpeg を強制終了する (キャンセル時)。
    pub async fn kill(&mut self) {
        self.stdin.take();
        let _ = self.child.kill().await;
    }
}

/// セッションレジストリ。Tauri state として `manage` する。
#[derive(Clone, Default)]
pub struct BurnInSessions {
    inner: Arc<std::sync::Mutex<HashMap<String, Arc<AsyncMutex<BurnInSession>>>>>,
}

impl BurnInSessions {
    pub fn insert(&self, id: String, session: BurnInSession) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(id, Arc::new(AsyncMutex::new(session)));
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<AsyncMutex<BurnInSession>>> {
        self.inner.lock().ok()?.get(id).cloned()
    }

    pub fn remove(&self, id: &str) -> Option<Arc<AsyncMutex<BurnInSession>>> {
        self.inner.lock().ok()?.remove(id)
    }
}

/// niconicomments-convert と同じフィルタグラフを組み立てる (純粋関数)。
///
/// - 入力 0 = 元動画 (`[0:v]`): fps 正規化 → 16:9 に pad → 出力解像度へ scale。
/// - 入力 1 = コメント PNG (`[1:v]`): bt601→bt709 変換 + alphamerge。
/// - `opacity < 1` のときはコメント画像のアルファを乗算する。
pub fn overlay_filter(width: u32, height: u32, fps: u32, opacity: f64) -> String {
    let op = opacity.clamp(0.0, 1.0);
    let base = format!(
        "[0:v]fps=fps={fps},pad=width=max(iw\\,ih*(16/9)):height=ow/(16/9):x=(ow-iw)/2:y=(oh-ih)/2,scale=w={width}:h={height}[video];\
         [1:v]format=yuva444p,colorspace=bt709:iall=bt601-6-525:fast=1[baseImage];\
         [1:v]format=rgba,alphaextract[alpha];\
         [baseImage][alpha]alphamerge[image]"
    );
    if op >= 0.999 {
        format!("{base};[video][image]overlay[output]")
    } else {
        format!(
            "{base};[image]format=rgba,colorchannelmixer=aa={op:.4}[imageop];[video][imageop]overlay[output]"
        )
    }
}

/// ffmpeg を起動してストリーミングセッションを作る。
#[allow(clippy::too_many_arguments)]
pub async fn spawn_session(
    app: Option<&tauri::AppHandle>,
    video: &Path,
    audio: Option<&Path>,
    output: &Path,
    width: u32,
    height: u32,
    fps: u32,
    opacity: f64,
    total_frames: u64,
) -> Result<BurnInSession, ApiError> {
    let ff = tools::ffmpeg(app);
    if matches!(ff.source, tools::BinarySource::NotFound) {
        return Err(ApiError::Downloader(
            "ffmpeg が見つかりません。インストールしてから再実行してください。".into(),
        ));
    }
    if output.exists() {
        let _ = tokio::fs::remove_file(output).await;
    }

    let filter = overlay_filter(width, height, fps, opacity);

    let mut cmd = tools::tokio_command(&ff.command);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-nostats")
        .arg("-y")
        // convert と同じ高品質スケーラ。
        .arg("-sws_flags")
        .arg("spline+accurate_rnd+full_chroma_int")
        // 入力 0: 元動画。
        .arg("-i")
        .arg(video)
        // 入力 1: コメント PNG 列 (stdin / image2pipe)。
        .arg("-f")
        .arg("image2pipe")
        .arg("-framerate")
        .arg(fps.to_string())
        .arg("-i")
        .arg("pipe:0");
    let audio_input = if let Some(a) = audio {
        cmd.arg("-i").arg(a);
        Some(2u32)
    } else {
        None
    };
    cmd.arg("-filter_complex")
        .arg(&filter)
        .arg("-map")
        .arg("[output]");
    match audio_input {
        Some(idx) => {
            cmd.arg("-map").arg(format!("{idx}:a:0"));
        }
        None => {
            // 元動画に音声があれば拾う。無ければ無視 (?)。
            cmd.arg("-map").arg("0:a:0?");
        }
    }
    cmd.arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("veryfast")
        .arg("-crf")
        .arg("20")
        .arg("-pix_fmt")
        .arg("yuv420p")
        // 出力を bt709 としてタグ付け (convert と同じ色域運用)。
        .arg("-color_range")
        .arg("tv")
        .arg("-colorspace")
        .arg("bt709")
        .arg("-color_primaries")
        .arg("bt709")
        .arg("-color_trc")
        .arg("bt709")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("192k")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output);

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| ApiError::Downloader(format!("failed to spawn ffmpeg: {e}")))?;

    let stdin = child.stdin.take();
    let stderr_buf = Arc::new(AsyncMutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let buf = stderr_buf.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut s = String::new();
            let _ = reader.read_to_string(&mut s).await;
            *buf.lock().await = s;
        });
    }

    Ok(BurnInSession {
        stdin,
        child,
        stderr: stderr_buf,
        empty_png: None,
        output_path: output.to_path_buf(),
        width,
        height,
        fps,
        total_frames,
        fed: 0,
    })
}

/// `burnin_feed` のバイナリフレームを分解する (純粋関数)。
///
/// レイアウト: `[u8 flag][u32 LE session_len][session utf8][payload...]`
/// 戻り値: `(flag, session_id, payload)`。
pub fn parse_feed_frame(body: &[u8]) -> Option<(u8, String, &[u8])> {
    if body.len() < 5 {
        return None;
    }
    let flag = body[0];
    let sid_len = u32::from_le_bytes([body[1], body[2], body[3], body[4]]) as usize;
    let sid_start: usize = 5;
    let sid_end = sid_start.checked_add(sid_len)?;
    if body.len() < sid_end {
        return None;
    }
    let sid = std::str::from_utf8(&body[sid_start..sid_end])
        .ok()?
        .to_string();
    let payload = &body[sid_end..];
    Some((flag, sid, payload))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn filter_uses_convert_graph_and_16x9_pad() {
        let f = overlay_filter(1920, 1080, 30, 1.0);
        assert!(f.contains("[0:v]fps=fps=30"));
        assert!(f.contains("pad=width=max(iw\\,ih*(16/9))"));
        assert!(f.contains("scale=w=1920:h=1080"));
        assert!(f.contains("colorspace=bt709:iall=bt601-6-525:fast=1"));
        assert!(f.contains("alphamerge[image]"));
        assert!(f.contains("[video][image]overlay[output]"));
    }

    #[test]
    fn filter_applies_opacity_when_below_one() {
        let f = overlay_filter(1280, 720, 30, 0.5);
        assert!(f.contains("colorchannelmixer=aa=0.5000"));
        assert!(f.contains("[video][imageop]overlay[output]"));
    }

    #[test]
    fn feed_frame_roundtrips() {
        let sid = "abc123";
        let png = [0u8, 1, 2, 3, 255];
        let mut body = Vec::new();
        body.push(FLAG_FRAME);
        body.extend_from_slice(&(sid.len() as u32).to_le_bytes());
        body.extend_from_slice(sid.as_bytes());
        body.extend_from_slice(&png);
        let (flag, parsed_sid, payload) = parse_feed_frame(&body).unwrap();
        assert_eq!(flag, FLAG_FRAME);
        assert_eq!(parsed_sid, sid);
        assert_eq!(payload, png);
    }

    #[test]
    fn feed_frame_empty_has_no_payload() {
        let sid = "s";
        let mut body = Vec::new();
        body.push(FLAG_EMPTY);
        body.extend_from_slice(&(sid.len() as u32).to_le_bytes());
        body.extend_from_slice(sid.as_bytes());
        let (flag, parsed_sid, payload) = parse_feed_frame(&body).unwrap();
        assert_eq!(flag, FLAG_EMPTY);
        assert_eq!(parsed_sid, sid);
        assert!(payload.is_empty());
    }

    #[test]
    fn feed_frame_rejects_truncated() {
        assert!(parse_feed_frame(&[]).is_none());
        assert!(parse_feed_frame(&[FLAG_FRAME, 10, 0, 0, 0]).is_none()); // sid_len > body
    }
}
