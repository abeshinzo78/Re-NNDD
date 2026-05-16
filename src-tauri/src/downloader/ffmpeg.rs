//! ffmpeg を使った copy-mux + faststart。
//!
//! niconico Domand が返す CMAF fMP4 を **再生互換性の高い MP4** に作り直す。
//! 再エンコードはしない (`-c copy`) ので CPU 軽い。`+faststart` で moov を
//! 先頭に持ってきて、`<video>` のシーク開始がすぐ始まるようにする。
//!
//! ffmpeg がインストールされていない / 失敗した場合は呼び出し側で fallback。

use std::path::Path;
use std::sync::Arc;

use tauri::Manager;

use crate::api::auth::SessionStore;
use crate::downloader::tools;
use crate::error::ApiError;

const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const NICO_REFERER: &str = "https://www.nicovideo.jp/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MuxOutcome {
    /// 1 ファイルへの統合に成功 (映像+音声 / 映像のみ問わず)
    Success,
    /// ffmpeg がインストールされていない (PATH に無い)
    FfmpegNotFound,
    /// ffmpeg はあるが処理に失敗した
    FfmpegFailed { stderr: String },
}

fn session_cookie_header(app: Option<&tauri::AppHandle>) -> Option<String> {
    let app = app?;
    let store = app.state::<Arc<SessionStore>>();
    store.inner().cookie_header()
}

fn domand_headers(cookie_header: Option<&str>) -> String {
    let mut headers = String::from(
        "Accept: */*\r\n\
         Accept-Language: ja,en-US;q=0.9,en;q=0.8\r\n\
         sec-fetch-mode: cors\r\n\
         sec-fetch-site: same-site\r\n\
         sec-fetch-dest: empty\r\n\
         sec-ch-ua: \"Chromium\";v=\"130\", \"Not?A_Brand\";v=\"99\"\r\n\
         sec-ch-ua-mobile: ?0\r\n\
         sec-ch-ua-platform: \"Linux\"\r\n",
    );
    if let Some(cookie) = cookie_header {
        headers.push_str("Cookie: ");
        headers.push_str(cookie);
        headers.push_str("\r\n");
    }
    headers
}

fn apply_domand_http_options(cmd: &mut tokio::process::Command, cookie_header: Option<&str>) {
    cmd.arg("-user_agent")
        .arg(BROWSER_UA)
        .arg("-referer")
        .arg(NICO_REFERER)
        .arg("-headers")
        .arg(domand_headers(cookie_header));
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

    // Pipe PNG to stdout so we don't depend on a writable working directory.
    // (Previous impl wrote to CWD-relative path which silently failed when
    // the CWD wasn't writable — leaving the screenshot button blank.)
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
        .arg("-f")
        .arg("image2pipe")
        .arg("-vcodec")
        .arg("png")
        .arg("-")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let result = match cmd.output().await {
        Ok(result) => result,
        Err(error) => {
            tracing::warn!(seek_sec, %error, "failed to spawn ffmpeg for local frame extraction");
            return None;
        }
    };
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        tracing::warn!(
            seek_sec,
            stderr = %stderr,
            "ffmpeg local frame extraction failed"
        );
        return None;
    }
    if result.stdout.is_empty() {
        tracing::warn!(seek_sec, "ffmpeg local frame extraction returned empty stdout");
        return None;
    }
    Some(result.stdout)
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
    let cookie_header = session_cookie_header(app);

    let mut cmd = tools::tokio_command(&ff.command);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-ss")
        .arg(format!("{seek_sec:.3}"));
    apply_domand_http_options(&mut cmd, cookie_header.as_deref());
    cmd.arg("-i")
        .arg(url)
        .arg("-vframes")
        .arg("1")
        .arg("-vcodec")
        .arg("png")
        .arg("-f")
        .arg("image2pipe")
        .arg("-")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Run with a generous timeout for network HLS.
    let result = match tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output()).await
    {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            tracing::warn!(
                %url,
                seek_sec,
                %error,
                "failed to spawn ffmpeg for remote frame extraction"
            );
            return None;
        }
        Err(_) => {
            tracing::warn!(%url, seek_sec, "ffmpeg remote frame extraction timed out");
            return None;
        }
    };

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        tracing::warn!(
            %url,
            seek_sec,
            stderr = %stderr,
            "ffmpeg remote frame extraction failed"
        );
        return None;
    }
    if result.stdout.is_empty() {
        tracing::warn!(
            %url,
            seek_sec,
            "ffmpeg remote frame extraction returned empty stdout"
        );
        return None;
    }
    Some(result.stdout)
}
