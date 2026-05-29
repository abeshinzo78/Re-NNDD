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
use tokio::io::{AsyncBufReadExt, BufReader};

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
        tracing::warn!(
            seek_sec,
            "ffmpeg local frame extraction returned empty stdout"
        );
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

/// 動画の解像度と長さ。`probe_video` が返す。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProbeInfo {
    pub width: u32,
    pub height: u32,
    pub duration_sec: f64,
}

/// `ffmpeg -i` の stderr から解像度と長さを拾う。ffprobe をバンドルしていない
/// 構成でも ffmpeg 単体で完結させるためのヘルパ。`#[cfg(test)]` から呼べるよう
/// パース部分は純関数 `parse_probe` に切り出してある。
pub async fn probe_video(app: Option<&tauri::AppHandle>, video: &Path) -> Option<ProbeInfo> {
    let ff = tools::ffmpeg(app);
    if matches!(ff.source, tools::BinarySource::NotFound) {
        return None;
    }
    // `-i` だけ渡すと ffmpeg は「出力が無い」と非ゼロ終了するが、stream 情報は
    // stderr に出力済みなので終了コードは無視してよい。
    let mut cmd = tools::tokio_command(&ff.command);
    cmd.arg("-hide_banner").arg("-i").arg(video);
    let result = cmd.output().await.ok()?;
    let stderr = String::from_utf8_lossy(&result.stderr);
    parse_probe(&stderr)
}

/// `ffmpeg -i` の stderr テキストから `(width, height, duration_sec)` を取り出す。
fn parse_probe(stderr: &str) -> Option<ProbeInfo> {
    let mut dims: Option<(u32, u32)> = None;
    let mut duration: Option<f64> = None;

    for line in stderr.lines() {
        let t = line.trim();
        if duration.is_none() {
            if let Some(idx) = t.find("Duration:") {
                let rest = &t[idx + "Duration:".len()..];
                let token = rest.trim_start().split(',').next().unwrap_or("").trim();
                duration = parse_timecode(token);
            }
        }
        if dims.is_none() && t.contains("Video:") {
            dims = find_dimensions(t);
        }
    }

    let (width, height) = dims?;
    Some(ProbeInfo {
        width,
        height,
        duration_sec: duration.unwrap_or(0.0),
    })
}

/// `HH:MM:SS.cs` 形式を秒に。`N/A` 等は None。
fn parse_timecode(s: &str) -> Option<f64> {
    let mut parts = s.split(':');
    let h: f64 = parts.next()?.trim().parse().ok()?;
    let m: f64 = parts.next()?.trim().parse().ok()?;
    let sec: f64 = parts.next()?.trim().parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// 行の中から `WIDTHxHEIGHT` (例 `1280x720`) を探す。SAR 等の余計な `x` 区切り
/// に引っかからないよう、両側が数字で 2 桁以上の最初の組を採用する。
fn find_dimensions(line: &str) -> Option<(u32, u32)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'x' && i > 0 {
            // 左側の数字列。
            let mut l = i;
            while l > 0 && bytes[l - 1].is_ascii_digit() {
                l -= 1;
            }
            // 右側の数字列。
            let mut r = i + 1;
            while r < bytes.len() && bytes[r].is_ascii_digit() {
                r += 1;
            }
            if l < i && r > i + 1 {
                let w: u32 = line[l..i].parse().ok()?;
                let h: u32 = line[i + 1..r].parse().ok()?;
                // 16x9 のような明らかなアスペクト比表記を除外。
                if w >= 16 && h >= 16 {
                    return Some((w, h));
                }
            }
        }
        i += 1;
    }
    None
}

/// ASS 字幕を映像へ焼き込んで `output` を作る。映像は再エンコード (libx264)、
/// 音声はコピー。`audio` が Some なら別ファイルの音声を多重化する。
///
/// `total_duration_sec` は進捗 % 計算の分母。`on_progress` には 0.0〜1.0 が渡る。
/// `ass_path` はフィルタのファイル名エスケープを避けるため、その親ディレクトリを
/// ワーキングディレクトリにしてベース名だけをフィルタへ渡す。
pub async fn burn_in_comments<F>(
    app: Option<&tauri::AppHandle>,
    video: &Path,
    audio: Option<&Path>,
    ass_path: &Path,
    output: &Path,
    total_duration_sec: f64,
    mut on_progress: F,
) -> Result<MuxOutcome, ApiError>
where
    F: FnMut(f64),
{
    let ff = tools::ffmpeg(app);
    if matches!(ff.source, tools::BinarySource::NotFound) {
        return Ok(MuxOutcome::FfmpegNotFound);
    }
    if output.exists() {
        let _ = tokio::fs::remove_file(output).await;
    }

    let ass_dir = ass_path
        .parent()
        .ok_or_else(|| ApiError::Downloader("ass path has no parent directory".to_string()))?;
    let ass_name = ass_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| ApiError::Downloader("ass path has no file name".to_string()))?;

    let mut cmd = tools::tokio_command(&ff.command);
    cmd.current_dir(ass_dir);
    cmd.arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-nostats")
        .arg("-progress")
        .arg("pipe:1")
        .arg("-y")
        .arg("-i")
        .arg(video);
    if let Some(a) = audio {
        cmd.arg("-i").arg(a);
    }
    // [0:v] に ass フィルタを適用してラベル付き出力にする。
    cmd.arg("-filter_complex")
        .arg(format!("[0:v]ass={ass_name}[v]"))
        .arg("-map")
        .arg("[v]");
    if audio.is_some() {
        cmd.arg("-map").arg("1:a:0");
    } else {
        // 元動画に音声があれば拾う。無くてもエラーにしない (`?`)。
        cmd.arg("-map").arg("0:a:0?");
    }
    cmd.arg("-c:v")
        .arg("libx264")
        .arg("-preset")
        .arg("veryfast")
        .arg("-crf")
        .arg("20")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-c:a")
        .arg("copy")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output);

    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| ApiError::Downloader(format!("failed to spawn ffmpeg: {e}")))?;

    // stderr は別タスクで全部吸い出す (失敗時の診断用)。
    let stderr_handle = child.stderr.take().map(|stderr| {
        tokio::spawn(async move {
            let mut buf = String::new();
            let mut reader = BufReader::new(stderr);
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut reader, &mut buf).await;
            buf
        })
    });

    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(sec) = parse_progress_line(&line) {
                if total_duration_sec > 0.0 {
                    on_progress((sec / total_duration_sec).clamp(0.0, 1.0));
                }
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| ApiError::Downloader(format!("failed to wait ffmpeg: {e}")))?;

    let stderr = match stderr_handle {
        Some(h) => h.await.unwrap_or_default(),
        None => String::new(),
    };

    if status.success() {
        on_progress(1.0);
        Ok(MuxOutcome::Success)
    } else {
        Ok(MuxOutcome::FfmpegFailed { stderr })
    }
}

/// `-progress pipe:1` の 1 行から経過秒を取り出す。`out_time_us`
/// (マイクロ秒) を優先し、無ければ `out_time=HH:MM:SS.us` を見る。
fn parse_progress_line(line: &str) -> Option<f64> {
    let line = line.trim();
    if let Some(v) = line.strip_prefix("out_time_us=") {
        return v.trim().parse::<f64>().ok().map(|us| us / 1_000_000.0);
    }
    if let Some(v) = line.strip_prefix("out_time_ms=") {
        // 仕様上は ms 名だが実体はマイクロ秒の ffmpeg ビルドが多い。
        return v.trim().parse::<f64>().ok().map(|us| us / 1_000_000.0);
    }
    if let Some(v) = line.strip_prefix("out_time=") {
        return parse_timecode(v.trim());
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_probe_extracts_dims_and_duration() {
        let stderr = "\
Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'video.mp4':
  Duration: 00:03:21.50, start: 0.000000, bitrate: 1234 kb/s
    Stream #0:0(und): Video: h264 (High), yuv420p(tv), 1280x720 [SAR 1:1 DAR 16:9], 1000 kb/s, 30 fps
    Stream #0:1(und): Audio: aac (LC), 48000 Hz, stereo, fltp, 128 kb/s
";
        let info = parse_probe(stderr).unwrap();
        assert_eq!(info.width, 1280);
        assert_eq!(info.height, 720);
        assert!((info.duration_sec - 201.5).abs() < 0.01);
    }

    #[test]
    fn parse_probe_handles_portrait() {
        let stderr = "    Stream #0:0: Video: h264, yuv420p, 720x1280, 30 fps";
        let info = parse_probe(stderr).unwrap();
        assert_eq!(info.width, 720);
        assert_eq!(info.height, 1280);
    }

    #[test]
    fn parse_probe_none_without_video() {
        let stderr = "  Duration: 00:01:00.00\n    Stream #0:0: Audio: aac";
        assert!(parse_probe(stderr).is_none());
    }

    #[test]
    fn timecode_parses() {
        assert!((parse_timecode("00:03:21.50").unwrap() - 201.5).abs() < 0.001);
        assert!(parse_timecode("N/A").is_none());
    }

    #[test]
    fn dimensions_ignores_aspect_ratio() {
        // DAR 16:9 の "16x9" ではなく実解像度を拾う。
        let line = "Video: h264, yuv420p, 1920x1080 [SAR 1:1 DAR 16x9], 30 fps";
        assert_eq!(find_dimensions(line), Some((1920, 1080)));
    }

    #[test]
    fn progress_line_parsing() {
        assert!((parse_progress_line("out_time_us=2500000").unwrap() - 2.5).abs() < 0.001);
        assert!((parse_progress_line("out_time=0:00:05.00").unwrap() - 5.0).abs() < 0.001);
        assert_eq!(parse_progress_line("frame=10"), None);
        assert_eq!(parse_progress_line("progress=continue"), None);
    }
}
