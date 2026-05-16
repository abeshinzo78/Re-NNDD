//! yt-dlp 経由でのダウンロード。
//!
//! 自前 HLS+AES+ffmpeg より yt-dlp に丸投げした方が堅い:
//! - niconico の仕様変更に追従しているのは向こう
//! - 出力は普通の H.264 + AAC mp4 で WebKit が素直に食える
//! - 失敗パターンも yt-dlp 側で握り潰してくれている

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, watch};

use crate::downloader::tools;
use crate::error::ApiError;

#[derive(Debug)]
pub struct YtdlpResult {
    pub video_path: PathBuf,
    pub info_path: PathBuf,
    pub info_json: serde_json::Value,
    pub thumbnail_path: Option<PathBuf>,
    pub description_path: Option<PathBuf>,
}

pub async fn is_available(app: Option<&tauri::AppHandle>) -> bool {
    let r = tools::ytdlp(app);
    if matches!(r.source, tools::BinarySource::NotFound) {
        return false;
    }
    tools::tokio_command(&r.command)
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `url` を `output_dir` に DL する。`extra_cookie` は `Cookie:` ヘッダ値。
/// `on_progress` には 0.0..=1.0 が送られる。
/// `app` を渡すとバンドルされた yt-dlp を優先で使う (None なら exe 周辺 + PATH のみ)。
pub async fn download<F>(
    app: Option<&tauri::AppHandle>,
    url: &str,
    output_dir: &Path,
    extra_cookie: Option<String>,
    mut on_progress: F,
) -> Result<YtdlpResult, ApiError>
where
    F: FnMut(f64) + Send,
{
    download_inner(app, url, output_dir, extra_cookie, &mut on_progress, None).await
}

pub async fn download_with_cancel<F>(
    app: Option<&tauri::AppHandle>,
    url: &str,
    output_dir: &Path,
    extra_cookie: Option<String>,
    mut on_progress: F,
    cancel: watch::Receiver<bool>,
) -> Result<YtdlpResult, ApiError>
where
    F: FnMut(f64) + Send,
{
    download_inner(
        app,
        url,
        output_dir,
        extra_cookie,
        &mut on_progress,
        Some(cancel),
    )
    .await
}

async fn download_inner<F>(
    app: Option<&tauri::AppHandle>,
    url: &str,
    output_dir: &Path,
    extra_cookie: Option<String>,
    on_progress: &mut F,
    mut cancel: Option<watch::Receiver<bool>>,
) -> Result<YtdlpResult, ApiError>
where
    F: FnMut(f64) + Send,
{
    let yt = tools::ytdlp(app);
    if matches!(yt.source, tools::BinarySource::NotFound) {
        return Err(ApiError::Downloader(
            "yt-dlp が見つかりません。\
             `bash scripts/fetch-binaries.sh` でバンドルするか、\
             `pipx install yt-dlp` / `apt install yt-dlp` でインストールしてください。"
                .into(),
        ));
    }

    tokio::fs::create_dir_all(output_dir).await?;

    // Cookie は deprecated な --add-header 経由ではなく、Netscape 形式の
    // ファイルに書いて --cookies で渡す。--add-header だと yt-dlp が
    // ドメイン scope なし扱いで送ってしまい niconico API が 400 を返すケース
    // があった (公式エラーログ参照)。
    let cookies_file = if let Some(ref cookie) = extra_cookie {
        let path = output_dir.join(".cookies.txt");
        let netscape = build_netscape_cookies(cookie);
        if let Err(e) = tokio::fs::write(&path, netscape).await {
            tracing::warn!(error = %e, "failed to write cookies file; falling back to no cookies");
            None
        } else {
            Some(path)
        }
    } else {
        None
    };

    let mut cmd = tools::tokio_command(&yt.command);
    // Unix: yt-dlp が裏で立ち上げる ffmpeg (mp4 マージ用) も道連れに
    // 殺せるように、yt-dlp 自身を新しいプロセスグループのリーダにする。
    // こうしておけば kill_process_tree() で `-pgid` 宛に SIGKILL を投げる
    // だけで子孫まで一括停止できる。
    // `tokio::process::Command::process_group` は tokio 1.27 以降の API。
    #[cfg(unix)]
    cmd.process_group(0);
    cmd.arg("--no-warnings")
        .arg("--newline")
        .arg("--no-colors")
        .arg("--no-mtime")
        .arg("--no-playlist")
        .arg("-P")
        .arg(output_dir)
        .arg("-o")
        .arg("video.%(ext)s")
        .arg("-f")
        .arg("bv*+ba/b")
        .arg("--merge-output-format")
        .arg("mp4")
        .arg("--write-info-json")
        .arg("--write-thumbnail")
        .arg("--write-description")
        .arg("--convert-thumbnails")
        .arg("jpg")
        .arg("--postprocessor-args")
        .arg("ffmpeg:-movflags +faststart")
        .arg("--progress-template")
        .arg("nndd-progress:%(progress._percent_str)s");

    // バンドル ffmpeg を yt-dlp に明示。PATH の ffmpeg は無視させる。
    let ff = tools::ffmpeg(app);
    if !matches!(ff.source, tools::BinarySource::NotFound) {
        cmd.arg("--ffmpeg-location").arg(&ff.command);
    }

    if let Some(ref p) = cookies_file {
        cmd.arg("--cookies").arg(p);
    }

    cmd.arg(url).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| ApiError::Downloader(format!("failed to spawn yt-dlp: {e}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ApiError::Downloader("yt-dlp stdout missing".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ApiError::Downloader("yt-dlp stderr missing".into()))?;

    // stdout / stderr どちらにも progress が出る可能性がある (yt-dlp の
    // バージョンや quiet フラグ次第)。両方読んで progress を拾う。
    // stderr は終了時にエラー診断にも使うのでバッファリング。
    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<(StreamKind, String)>();
    let stdout_tx = line_tx.clone();
    let stderr_tx = line_tx.clone();
    drop(line_tx); // child reader が終わったら channel を閉じるために手元の tx を捨てる

    let stdout_handle = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = stdout_tx.send((StreamKind::Stdout, line));
        }
    });
    let stderr_handle = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = stderr_tx.send((StreamKind::Stderr, line));
        }
    });

    let mut stderr_buf = String::new();
    let status = loop {
        tokio::select! {
            line = line_rx.recv() => {
                if let Some((kind, line)) = line {
                    if let Some(pct) = parse_progress_line(&line) {
                        on_progress(pct);
                    } else if !line.is_empty() {
                        tracing::debug!(target: "ytdlp", stream = ?kind, "{line}");
                    }
                    if matches!(kind, StreamKind::Stderr) && !line.is_empty() {
                        stderr_buf.push_str(&line);
                        stderr_buf.push('\n');
                    }
                }
            }
            status = child.wait() => {
                break status.map_err(|e| ApiError::Downloader(format!("yt-dlp wait failed: {e}")))?;
            }
            changed = async {
                match cancel.as_mut() {
                    Some(rx) => rx.changed().await,
                    None => std::future::pending().await,
                }
            } => {
                if changed.is_ok() && cancel.as_ref().is_some_and(|rx| *rx.borrow()) {
                    // child.kill() は yt-dlp 本体しか殺さないので、
                    // mp4 マージ中の ffmpeg が孤児として生き残り、出力
                    // ファイルをロックして「キャンセルしたのにファイルが
                    // 消せない / 次回 DL が失敗する」事故になる。
                    // 子孫まで道連れにするために kill_process_tree を使う。
                    if let Some(pid) = child.id() {
                        kill_process_tree(pid);
                    }
                    let _ = child.kill().await;
                    let _ = stdout_handle.await;
                    let _ = stderr_handle.await;
                    cleanup_cookies(cookies_file.as_ref()).await;
                    return Err(ApiError::DownloadCanceled);
                }
            }
        }
    };
    let _ = stdout_handle.await;
    let _ = stderr_handle.await;

    // 一時 cookie ファイルは用済み。残しても害は少ないが念のため削除。
    cleanup_cookies(cookies_file.as_ref()).await;

    if !status.success() {
        return Err(ApiError::Downloader(format!(
            "yt-dlp failed (exit {}):\n{}",
            status.code().unwrap_or(-1),
            stderr_buf
                .lines()
                .filter(|l| !l.is_empty())
                .take(40)
                .collect::<Vec<_>>()
                .join("\n")
        )));
    }

    let video_path = output_dir.join("video.mp4");
    if !video_path.exists() {
        return Err(ApiError::Downloader(format!(
            "yt-dlp が video.mp4 を作らなかった (出力: {})",
            output_dir.display()
        )));
    }
    let info_path = output_dir.join("video.info.json");
    let info_json: serde_json::Value = match tokio::fs::read(&info_path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        Err(_) => serde_json::Value::Null,
    };
    let thumbnail_path = {
        let p = output_dir.join("video.jpg");
        if p.exists() {
            Some(p)
        } else {
            None
        }
    };
    let description_path = {
        let p = output_dir.join("video.description");
        if p.exists() {
            Some(p)
        } else {
            None
        }
    };

    Ok(YtdlpResult {
        video_path,
        info_path,
        info_json,
        thumbnail_path,
        description_path,
    })
}

async fn cleanup_cookies(path: Option<&PathBuf>) {
    if let Some(p) = path {
        let _ = tokio::fs::remove_file(p).await;
    }
}

/// `pid` を起点としたプロセス木全体に SIGKILL 相当を送る。
///
/// - Unix: spawn 時に `process_group(0)` で yt-dlp を pgid=pid のリーダに
///   しているので、`kill -KILL -<pid>` で子孫（マージ用 ffmpeg を含む）
///   まで一括停止できる。
/// - Windows: `taskkill /F /T` が同じ役割（プロセス木の強制終了）。
///
/// ユーティリティ系コマンドは PATH に必ずある前提だが、無くても害はない
/// ので失敗は無視する。
fn kill_process_tree(pid: u32) {
    #[cfg(unix)]
    {
        let _ = tools::std_command("kill")
            .arg("-KILL")
            .arg(format!("-{pid}"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = tools::std_command("taskkill")
            .args(["/F", "/T", "/PID"])
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

#[derive(Debug, Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

/// "nndd-progress:42.5%" や "[download]  42.5% of ~120MiB ..." を 0.0..=1.0 に。
/// `"user_session=xxx; foo=bar"` 形式の Cookie ヘッダ文字列から、
/// nicovideo.jp ドメインに scope された Netscape 形式の cookies.txt を作る。
/// yt-dlp の `--cookies` がこの形式を期待する。
pub(crate) fn build_netscape_cookies(cookie_header: &str) -> String {
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    // 期限は 2038 直前 (i32 max) で十分。yt-dlp は session cookie 扱いを嫌う。
    let exp = "2147483647";
    for kv in cookie_header.split(';') {
        let kv = kv.trim();
        let Some((k, v)) = kv.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        if k.is_empty() {
            continue;
        }
        // domain  flag  path  secure  expiration  name  value (TAB 区切り)
        out.push_str(&format!(".nicovideo.jp\tTRUE\t/\tTRUE\t{exp}\t{k}\t{v}\n"));
    }
    out
}

fn parse_progress_line(line: &str) -> Option<f64> {
    if let Some(rest) = line.strip_prefix("nndd-progress:") {
        let s = rest.trim().trim_end_matches('%').trim();
        if let Ok(p) = s.parse::<f64>() {
            return Some((p / 100.0).clamp(0.0, 1.0));
        }
    }
    if line.starts_with("[download]") {
        // 例: "[download]   42.5% of ~120.00MiB at 5.20MiB/s ETA 00:12"
        if let Some(idx) = line.find('%') {
            let head = &line[..idx];
            let num_str = head.trim_start_matches("[download]").trim();
            if let Ok(p) = num_str.parse::<f64>() {
                return Some((p / 100.0).clamp(0.0, 1.0));
            }
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_custom_progress_template() {
        assert_eq!(parse_progress_line("nndd-progress:50.0%"), Some(0.5));
        assert_eq!(parse_progress_line("nndd-progress:  42.5%"), Some(0.425));
        assert_eq!(parse_progress_line("nndd-progress:100%"), Some(1.0));
    }

    #[test]
    fn parses_default_download_lines() {
        assert_eq!(
            parse_progress_line("[download]   0.0% of ~  120.00MiB at  Unknown B/s ETA Unknown"),
            Some(0.0)
        );
        let pct = parse_progress_line("[download]  42.5% of ~120.00MiB at    5.20MiB/s ETA 00:12")
            .unwrap();
        assert!((pct - 0.425).abs() < 1e-9);
    }

    #[test]
    fn ignores_non_progress_lines() {
        assert!(parse_progress_line("[info] Downloading webpage").is_none());
        assert!(parse_progress_line("").is_none());
        assert!(parse_progress_line("ERROR: something").is_none());
    }

    #[test]
    fn clamps_out_of_range() {
        assert_eq!(parse_progress_line("nndd-progress:-5%"), Some(0.0));
        assert_eq!(parse_progress_line("nndd-progress:200%"), Some(1.0));
    }

    #[test]
    fn builds_netscape_cookies_from_header() {
        let s = build_netscape_cookies("user_session=abc; nicosid=xyz");
        assert!(s.starts_with("# Netscape HTTP Cookie File\n"));
        assert!(s.contains(".nicovideo.jp\tTRUE\t/\tTRUE\t2147483647\tuser_session\tabc\n"));
        assert!(s.contains(".nicovideo.jp\tTRUE\t/\tTRUE\t2147483647\tnicosid\txyz\n"));
    }

    #[test]
    fn netscape_cookies_skips_invalid_pairs() {
        let s = build_netscape_cookies("=novalue; user_session=abc; broken");
        // user_session のみが有効な行になる
        assert_eq!(s.matches("\tuser_session\tabc\n").count(), 1);
        assert!(!s.contains("\tnovalue\t"));
        assert!(!s.contains("\tbroken\t"));
    }
}
