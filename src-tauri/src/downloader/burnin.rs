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
use tokio::sync::{Mutex as AsyncMutex, Notify};

use crate::downloader::tools;
use crate::error::ApiError;

/// `burnin_feed` のバイナリフレーム種別。
pub const FLAG_FRAME: u8 = 0;
pub const FLAG_EMPTY: u8 = 1;
pub const FLAG_SET_EMPTY: u8 = 2;

/// `burnin_feed` の結果。フロントはこれを見て送出を続けるか止めるか決める。
///
/// - `Accepted`: フレームを stdin へ書けた。続けてよい。
/// - `SinkClosed`: ffmpeg が必要なフレームを読み終えて stdin を閉じた。これ以上
///   送っても破棄されるだけなので **正常に** 送出を止める合図 (異常ではない)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedOutcome {
    Accepted,
    SinkClosed,
}

impl FeedOutcome {
    /// フロントへ返す bool。true = まだ受け付ける / false = 閉じたので止める。
    pub fn accepted(self) -> bool {
        matches!(self, FeedOutcome::Accepted)
    }
}

/// ffmpeg がもう stdin を読まなくなった (= パイプが閉じた) ことを示す IO エラーか。
///
/// 元動画は尺の小数秒で終わるのに対し、フロントは `ceil(尺)*fps` フレームを送る
/// ため、末尾に「動画より後ろの余剰フレーム」が必ず出る。ffmpeg は必要な分を
/// 読み終えると正常終了して stdin を閉じるので、その後の書き込みは broken pipe に
/// なる。これは異常ではなく想定内なので、成否は finish() の終了コードで判定する。
///
/// Windows の `ERROR_BROKEN_PIPE` (109) / `ERROR_NO_DATA` (232) はどちらも Rust が
/// `ErrorKind::BrokenPipe` へマップするので、プラットフォーム差なく拾える。
fn is_sink_closed(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::{BrokenPipe, ConnectionAborted, ConnectionReset, WriteZero};
    matches!(
        e.kind(),
        BrokenPipe | WriteZero | ConnectionReset | ConnectionAborted
    )
}

/// 1 焼き込みセッション。ffmpeg child と、そこへ書き込む stdin を保持する。
pub struct BurnInSession {
    stdin: Option<ChildStdin>,
    child: Child,
    stderr: Arc<AsyncMutex<String>>,
    /// 透明フレームの PNG バイト列 (1 度だけ転送して使い回す)。
    empty_png: Option<Vec<u8>>,
    /// ffmpeg が stdin を閉じた後は以降の書き込みを no-op にするフラグ。
    input_closed: bool,
    /// finish() が成功して正しい出力を生成し終えたか。セッションロックを保持した
    /// まま立てる。これで finish 成功とほぼ同時に来た遅延キャンセルが、生成済みの
    /// 正しい出力を誤って削除するのを防ぐ。
    completed: bool,
    /// キャンセル通知。finish() が ffmpeg を待っている間に burnin_cancel から
    /// 起こして強制終了させるために使う (セッションロックを介さず通知できる)。
    cancel: Arc<Notify>,
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
    ///
    /// ffmpeg が既に stdin を閉じていた (= 必要フレームを読み終えた) 場合は
    /// `SinkClosed` を返す。これは異常ではなく、フロントへ「送出を止めてよい」と
    /// 伝える正常系の合図。真の IO 異常のときだけ `Err` を返す。
    pub async fn write_frame(&mut self, png: &[u8]) -> Result<FeedOutcome, ApiError> {
        if self.input_closed {
            return Ok(FeedOutcome::SinkClosed);
        }
        let Some(stdin) = self.stdin.as_mut() else {
            self.input_closed = true;
            return Ok(FeedOutcome::SinkClosed);
        };
        match stdin.write_all(png).await {
            Ok(()) => {
                self.fed += 1;
                Ok(FeedOutcome::Accepted)
            }
            Err(e) if is_sink_closed(&e) => {
                // ffmpeg が必要分を読み終えて正常終了 → stdin が閉じた。末尾の
                // 余剰フレームによる broken pipe は想定内。エラーにせず止める。
                self.input_closed = true;
                self.stdin.take();
                Ok(FeedOutcome::SinkClosed)
            }
            Err(e) => Err(ApiError::Downloader(format!("write frame to ffmpeg: {e}"))),
        }
    }

    /// 透明フレームを 1 枚書き込む。
    pub async fn write_empty(&mut self) -> Result<FeedOutcome, ApiError> {
        if self.input_closed {
            return Ok(FeedOutcome::SinkClosed);
        }
        let png = self
            .empty_png
            .clone()
            .ok_or_else(|| ApiError::Downloader("empty frame not initialized".into()))?;
        self.write_frame(&png).await
    }

    /// stdin を閉じて ffmpeg の終了を待つ。成功すれば `Ok(())`。
    ///
    /// 待機中に `cancel` が起こされたら ffmpeg を強制終了して `Err` を返す
    /// (UI の「キャンセル」が encode/faststart 中でも効くようにするため)。
    pub async fn finish(&mut self) -> Result<(), ApiError> {
        // stdin を drop して EOF を送る → ffmpeg が encode を完了する。
        self.stdin.take();
        let cancel = self.cancel.clone();
        // child.wait() の完了 vs キャンセル通知を競わせる。select! は選ばれな
        // かった側の future を drop してから本体を実行するので、cancel 側の本体で
        // 改めて self.child を触れる (wait future の借用が解放済みのため)。
        let canceled = {
            let notified = cancel.notified();
            tokio::pin!(notified);
            tokio::select! {
                res = self.child.wait() => {
                    let status = res
                        .map_err(|e| ApiError::Downloader(format!("wait ffmpeg: {e}")))?;
                    if status.success() {
                        // ロック保持中に completed を立てる。以降にロックを取る
                        // 遅延キャンセルは discard_if_incomplete でスキップされ、
                        // 生成済みの正しい出力が消されない。
                        self.completed = true;
                        return Ok(());
                    }
                    let stderr = self.stderr.lock().await.clone();
                    return Err(ApiError::Downloader(format!(
                        "ffmpeg failed:\n{}",
                        stderr.lines().take(30).collect::<Vec<_>>().join("\n")
                    )));
                }
                _ = &mut notified => true,
            }
        };
        if canceled {
            let _ = self.child.start_kill();
            let _ = self.child.wait().await;
            return Err(ApiError::Downloader("burn-in canceled".into()));
        }
        Ok(())
    }

    /// ffmpeg を強制終了する (キャンセル時)。
    pub async fn kill(&mut self) {
        self.input_closed = true;
        self.stdin.take();
        let _ = self.child.kill().await;
    }

    /// キャンセル時の後始末。finish() が既に成功して正しい出力を生成済みなら
    /// 何もしない (遅延キャンセルが生成物を消すのを防ぐ)。未完なら ffmpeg を
    /// kill して部分出力を削除する。
    pub async fn discard_if_incomplete(&mut self) {
        if self.completed {
            return;
        }
        self.kill().await;
        let _ = tokio::fs::remove_file(&self.output_path).await;
    }
}

/// レジストリ内の 1 エントリ。`cancel` はセッションロックを取らずに参照できる
/// よう、ここに複製を持っておく (finish が長時間ロックを保持していても
/// burnin_cancel が即座に通知できる)。
struct SessionEntry {
    session: Arc<AsyncMutex<BurnInSession>>,
    cancel: Arc<Notify>,
}

/// セッションレジストリ。Tauri state として `manage` する。
#[derive(Clone, Default)]
pub struct BurnInSessions {
    inner: Arc<std::sync::Mutex<HashMap<String, SessionEntry>>>,
}

impl BurnInSessions {
    pub fn insert(&self, id: String, session: BurnInSession) {
        let cancel = session.cancel.clone();
        if let Ok(mut map) = self.inner.lock() {
            map.insert(
                id,
                SessionEntry {
                    session: Arc::new(AsyncMutex::new(session)),
                    cancel,
                },
            );
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<AsyncMutex<BurnInSession>>> {
        Some(self.inner.lock().ok()?.get(id)?.session.clone())
    }

    /// セッションロックを介さずキャンセル通知ハンドルだけ取り出す。
    pub fn cancel_handle(&self, id: &str) -> Option<Arc<Notify>> {
        Some(self.inner.lock().ok()?.get(id)?.cancel.clone())
    }

    pub fn remove(&self, id: &str) -> Option<Arc<AsyncMutex<BurnInSession>>> {
        Some(self.inner.lock().ok()?.remove(id)?.session)
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
        format!("{base};[video][image]overlay=eof_action=pass[output]")
    } else {
        format!(
            "{base};[image]format=rgba,colorchannelmixer=aa={op:.4}[imageop];[video][imageop]overlay=eof_action=pass[output]"
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
        input_closed: false,
        completed: false,
        cancel: Arc::new(Notify::new()),
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
        // eof_action=pass: コメントフレームが先に尽きても末尾は素の映像を流す
        // (整数 duration 切り捨てによる「最後のコメントが固まる」現象を防ぐ)。
        assert!(f.contains("[video][image]overlay=eof_action=pass[output]"));
    }

    #[test]
    fn filter_applies_opacity_when_below_one() {
        let f = overlay_filter(1280, 720, 30, 0.5);
        assert!(f.contains("colorchannelmixer=aa=0.5000"));
        assert!(f.contains("[video][imageop]overlay=eof_action=pass[output]"));
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

    #[test]
    fn sink_closed_detects_broken_pipe_family() {
        use std::io::{Error, ErrorKind};
        // パイプが閉じた系 = 末尾余剰フレームによる正常な打ち切り。
        assert!(is_sink_closed(&Error::from(ErrorKind::BrokenPipe)));
        assert!(is_sink_closed(&Error::from(ErrorKind::WriteZero)));
        assert!(is_sink_closed(&Error::from(ErrorKind::ConnectionReset)));
        assert!(is_sink_closed(&Error::from(ErrorKind::ConnectionAborted)));
        // 本物の IO 異常はエラーとして扱う (打ち切りにしない)。
        assert!(!is_sink_closed(&Error::from(ErrorKind::PermissionDenied)));
        assert!(!is_sink_closed(&Error::from(ErrorKind::NotFound)));
        assert!(!is_sink_closed(&Error::from(ErrorKind::Other)));
    }

    #[test]
    fn windows_broken_pipe_errnos_map_to_sink_closed() {
        // Windows の ERROR_BROKEN_PIPE (109) / ERROR_NO_DATA (232) は Rust が
        // ErrorKind::BrokenPipe へマップする。os error 109 の正常打ち切りを保証。
        #[cfg(windows)]
        {
            use std::io::Error;
            assert!(is_sink_closed(&Error::from_raw_os_error(109)));
            assert!(is_sink_closed(&Error::from_raw_os_error(232)));
        }
        // 非 Windows でも EPIPE(32) は BrokenPipe にマップされる。
        #[cfg(unix)]
        {
            use std::io::Error;
            assert!(is_sink_closed(&Error::from_raw_os_error(32)));
        }
    }

    #[test]
    fn feed_outcome_accepted_flag() {
        assert!(FeedOutcome::Accepted.accepted());
        assert!(!FeedOutcome::SinkClosed.accepted());
    }

    /// 長時間動く子プロセスを持つセッションを直接組む (ffmpeg 不要のテスト用)。
    #[cfg(unix)]
    fn session_with_long_child() -> BurnInSession {
        let child = tokio::process::Command::new("sleep")
            .arg("30")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sleep");
        BurnInSession {
            stdin: None,
            child,
            stderr: Arc::new(AsyncMutex::new(String::new())),
            empty_png: None,
            input_closed: false,
            completed: false,
            cancel: Arc::new(Notify::new()),
            output_path: std::path::PathBuf::from("/tmp/burnin-test-output.mp4"),
            width: 2,
            height: 2,
            fps: 30,
            total_frames: 0,
            fed: 0,
        }
    }

    /// PR #15 レビュー対応: encode/faststart を待っている最中でもキャンセルが
    /// 効くこと。レジストリ経由で取り出した cancel ハンドルを叩くと finish が
    /// 速やかに Err を返して子プロセスを kill する (= UI のキャンセルが届く)。
    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_handle_aborts_finish_via_registry() {
        let sessions = BurnInSessions::default();
        sessions.insert("s1".into(), session_with_long_child());
        let session = sessions.get("s1").expect("session present");

        let started = std::time::Instant::now();
        let handle = tokio::spawn(async move {
            let mut s = session.lock().await;
            s.finish().await
        });

        // finish が child.wait() を待ち始める頃にキャンセル通知。notify_one は
        // permit を残すので、待機開始より先でも取りこぼさない。
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        sessions
            .cancel_handle("s1")
            .expect("cancel handle present")
            .notify_one();

        let res = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("finish must return promptly after cancel")
            .expect("join task");
        assert!(res.is_err(), "canceled finish should return Err");
        // sleep 30 を待たずに、キャンセルで早期に返っていること。
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "finish should be aborted, not wait for the child to exit"
        );
    }

    /// 通知が finish の待機開始より「先」に来ても取りこぼさない (notify_one の
    /// permit 保持) ことを確認する。
    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_before_finish_waits_is_not_lost() {
        let mut session = session_with_long_child();
        // finish より先に通知しておく。
        session.cancel.notify_one();
        let res = tokio::time::timeout(std::time::Duration::from_secs(5), session.finish())
            .await
            .expect("finish must observe the stored cancel permit");
        assert!(res.is_err(), "pre-sent cancel should still abort finish");
        session.kill().await; // 念のため子プロセスを回収。
    }

    /// PR #16 レビュー対応: finish 成功とほぼ同時の遅延キャンセルが、生成済みの
    /// 正しい出力を消さないこと。completed=true なら discard はファイルを残し、
    /// completed=false (= 中断/失敗) なら部分出力を消す。
    #[cfg(unix)]
    #[tokio::test]
    async fn discard_keeps_completed_output_removes_incomplete() {
        let dir = tempfile::tempdir().expect("tempdir");

        // completed=true → 出力を残す。
        let mut done = session_with_long_child();
        let kept = dir.path().join("kept.mp4");
        std::fs::write(&kept, b"ok").expect("write kept");
        done.output_path = kept.clone();
        done.completed = true;
        done.discard_if_incomplete().await;
        assert!(kept.exists(), "completed output must be kept");
        done.kill().await; // 残った子プロセスを回収。

        // completed=false → 部分出力を削除。
        let mut partial = session_with_long_child();
        let gone = dir.path().join("partial.mp4");
        std::fs::write(&gone, b"partial").expect("write partial");
        partial.output_path = gone.clone();
        partial.completed = false;
        partial.discard_if_incomplete().await;
        assert!(!gone.exists(), "incomplete output must be removed");
    }
}
