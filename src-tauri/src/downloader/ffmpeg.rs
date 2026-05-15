//! ffmpeg を使った copy-mux + faststart。
//!
//! niconico Domand が返す CMAF fMP4 を **再生互換性の高い MP4** に作り直す。
//! 再エンコードはしない (`-c copy`) ので CPU 軽い。`+faststart` で moov を
//! 先頭に持ってきて、`<video>` のシーク開始がすぐ始まるようにする。
//!
//! ffmpeg がインストールされていない / 失敗した場合は呼び出し側で fallback。

use std::path::{Path, PathBuf};

use crate::downloader::tools;
use crate::error::ApiError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MuxOutcome {
    /// 1 ファイルへの統合に成功 (映像+音声 / 映像のみ問わず)
    Success,
    /// ffmpeg がインストールされていない (PATH に無い)
    FfmpegNotFound,
    /// ffmpeg はあるが処理に失敗した
    FfmpegFailed { stderr: String },
}

/// `ffmpeg` バイナリが PATH またはバンドルにあるか確認。
pub async fn is_ffmpeg_available(app: Option<&tauri::AppHandle>) -> bool {
    let r = tools::ffmpeg(app);
    if matches!(r.source, tools::BinarySource::NotFound) {
        return false;
    }
    match tools::tokio_command(&r.command)
        .arg("-version")
        .output()
        .await
    {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

/// 映像 + 音声の 2 ファイルを `output` に copy-mux する。
/// `audio` が None なら映像のみを faststart 付きで作り直す。
/// `app` を渡すとバンドル ffmpeg を優先で使う。
pub async fn remux(
    app: Option<&tauri::AppHandle>,
    video: &Path,
    audio: Option<&Path>,
    output: &Path,
) -> Result<MuxOutcome, ApiError> {
    let ff = tools::ffmpeg(app);
    if matches!(ff.source, tools::BinarySource::NotFound) {
        return Ok(MuxOutcome::FfmpegNotFound);
    }
    // 既存 output があると ffmpeg が対話 prompt を出すので消しておく。
    if output.exists() {
        let _ = tokio::fs::remove_file(output).await;
    }

    let mut cmd = tools::tokio_command(&ff.command);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(video);
    if let Some(a) = audio {
        cmd.arg("-i").arg(a);
    }
    cmd.arg("-c")
        .arg("copy")
        // moov を先頭に。`<video>` のロード初動が早い。
        .arg("-movflags")
        .arg("+faststart");
    if audio.is_some() {
        cmd.arg("-map").arg("0:v:0").arg("-map").arg("1:a:0");
    }
    cmd.arg(output);

    let result = cmd
        .output()
        .await
        .map_err(|e| ApiError::Downloader(format!("failed to spawn ffmpeg: {e}")))?;

    if result.status.success() {
        Ok(MuxOutcome::Success)
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
        Ok(MuxOutcome::FfmpegFailed { stderr })
    }
}

/// Extract a single frame from a video file at the given timestamp.
/// Returns PNG bytes. Returns None if ffmpeg is not available or fails.
pub async fn extract_frame(
    app: Option<&tauri::AppHandle>,
    video: &Path,
    seek_sec: f64,
) -> Option<Vec<u8>> {
    let ff = tools::ffmpeg(app);
    if matches!(ff.source, tools::BinarySource::NotFound) {
        return None;
    }

    // Construct a temporary output path.
    let tmp = PathBuf::from(format!("{}.screenshot.png", video.file_stem()?.to_str()?));

    let mut cmd = tools::tokio_command(&ff.command);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-ss")
        .arg(format!("{seek_sec:.3}"))
        .arg("-i")
        .arg(video)
        .arg("-vframes")
        .arg("1")
        .arg("-q:v")
        .arg("2")
        .arg(&tmp);

    let result = cmd.output().await.ok()?;
    if !result.status.success() {
        let _ = tokio::fs::remove_file(&tmp).await;
        return None;
    }

    let bytes = tokio::fs::read(&tmp).await.ok()?;
    let _ = tokio::fs::remove_file(&tmp).await;
    Some(bytes)
}

/// Extract a single frame from a remote URL (e.g. HLS playlist).
/// Pipes PNG data to stdout to avoid writing a temp file.
/// Timeout is 30 s to allow for network fetch + decode.
pub async fn extract_frame_from_url(
    app: Option<&tauri::AppHandle>,
    url: &str,
    seek_sec: f64,
) -> Option<Vec<u8>> {
    let ff = tools::ffmpeg(app);
    if matches!(ff.source, tools::BinarySource::NotFound) {
        return None;
    }

    let mut cmd = tools::tokio_command(&ff.command);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-ss")
        .arg(format!("{seek_sec:.3}"))
        .arg("-i")
        .arg(url)
        .arg("-vframes")
        .arg("1")
        .arg("-f")
        .arg("apng")
        .arg("-")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Run with a generous timeout for network HLS.
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output())
        .await
        .ok()?
        .ok()?;

    if !result.status.success() {
        return None;
    }
    Some(result.stdout)
}
