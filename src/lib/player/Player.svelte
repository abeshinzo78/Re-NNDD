<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import Hls from 'hls.js';
  import type { Level } from 'hls.js';
  import CommentLayer from './CommentLayer.svelte';
  import ControlBar from './ControlBar.svelte';
  import { bindShortcuts, type PlayerActions } from './shortcuts';
  import { disableSubtleCryptoOnce } from './disableSubtleCrypto';
  import { TauriHlsLoader } from './tauriHlsLoader';
  import type { PlayerComment } from './types';
  import { getBool, getNum } from '$lib/stores/settings.svelte';

  type Props = {
    /** HLS playlist URL（ストリーミング用）。`localSrc` を渡すならこちらは空文字でよい */
    hlsUrl: string;
    /** asset:// などの直接 src として使える URL。指定時は HLS 経路を完全にバイパス */
    localSrc?: string;
    /** 音声トラックを別ファイルで持っている時の URL（dual-element 同期再生）。
     *  指定時は隠し `<audio>` 要素を作って video と play/pause/seek/rate を同期する。 */
    localAudioSrc?: string;
    comments: PlayerComment[];
    refreshHlsUrl?: () => Promise<string>;
    onTime?: (time: number) => void;
    initialQualityLabel?: string;
    resumePosition?: number;
    loop?: boolean;
    /** ミニプレイヤー (PiP) 用の compact モード。ControlBar を抑制する。 */
    compact?: boolean;
    /** PiP ボタンが押された時のフック (compact=false 時のみ表示) */
    onTogglePip?: () => void;
    /** PiP ボタンの aria-pressed 表示用 */
    pipActive?: boolean;
  };

  let {
    hlsUrl,
    localSrc,
    localAudioSrc,
    comments,
    refreshHlsUrl,
    onTime,
    initialQualityLabel,
    resumePosition = 0,
    loop = false,
    compact = false,
    onTogglePip,
    pipActive = false,
  }: Props = $props();

  let stage = $state<HTMLDivElement | null>(null);
  let video = $state<HTMLVideoElement | null>(null);
  let audioEl = $state<HTMLAudioElement | null>(null);
  let hls: Hls | null = null;
  // seek 中は decode 途中フレームを見せないように <video> を visibility:hidden
  let isSeeking = $state(false);
  let seekUnhideTimer: ReturnType<typeof setTimeout> | null = null;
  // <video> の error イベントは初回 GOP デコードでよく一過性で出る。
  // 即時にバナーを出すと再生できてるのにエラーが見える。猶予 1.5s 待って
  // play イベントが来てなければ初めて表示する。
  let pendingVideoErrorTimer: ReturnType<typeof setTimeout> | null = null;
  function clearPendingVideoError() {
    if (pendingVideoErrorTimer) {
      clearTimeout(pendingVideoErrorTimer);
      pendingVideoErrorTimer = null;
    }
  }

  let paused = $state(true);
  let currentTime = $state(0);
  let duration = $state(0);
  let volume = $state(1);
  let muted = $state(false);
  let playbackRate = $state(1);
  let commentsEnabled = $state(compact || getBool('comment.default_enabled'));
  let commentOpacity = $state(getNum('comment.default_opacity'));
  let abLoop = $state<{ in: number | null; out: number | null; enabled: boolean }>({
    in: null,
    out: null,
    enabled: false,
  });
  let errorMessage = $state<string | null>(null);
  let loadingMessage = $state<string | null>(null);
  let isFullscreen = $state(false);
  let controlsVisible = $state(true);
  let hideTimer: ReturnType<typeof setTimeout> | null = null;
  let hlsLevels = $state<Level[]>([]);
  let currentLevel = $state(-1);
  let lastTimeUpdateTs = 0;
  let userPickedLevel = -1;

  const MAX_HLS_REISSUE_RETRIES = 3;
  const MAX_RECOVERY_ATTEMPTS = 3;
  // フラグメントが正常に読めたらリカバリ予算を戻す。
  // 一過性のエラーで予算を使い切って永続停止するのを防ぐ。
  const RESET_AFTER_LOADED_FRAGS = 3;
  let reissueAttempts = 0;
  let mediaRecoveryAttempts = 0;
  let networkRecoveryAttempts = 0;
  let stallRecoveryAttempts = 0;
  let consecutiveLoadedFrags = 0;
  let nonFatalTimer: ReturnType<typeof setTimeout> | null = null;
  let nonFatalCount = 0;
  let stallNudgeTimer: ReturnType<typeof setTimeout> | null = null;

  function showNonFatal(msg: string) {
    nonFatalCount++;
    loadingMessage = msg;
    if (nonFatalTimer) clearTimeout(nonFatalTimer);
    nonFatalTimer = setTimeout(() => {
      nonFatalCount = 0;
      if (loadingMessage === msg) loadingMessage = null;
      nonFatalTimer = null;
    }, 3000);
  }

  function showControls() {
    controlsVisible = true;
    if (hideTimer) clearTimeout(hideTimer);
    if (!paused) {
      hideTimer = setTimeout(() => {
        controlsVisible = false;
      }, 3000);
    }
  }

  async function loadFreshSource(forceRefresh = false) {
    const inflight = hls;
    if (!inflight) return;
    let url = hlsUrl;
    // Only call refreshHlsUrl on error recovery (403 expiry etc.),
    // NOT on initial load — the prop hlsUrl is already fresh.
    if (forceRefresh && refreshHlsUrl) {
      try {
        url = await refreshHlsUrl();
      } catch (e) {
        if (hls !== inflight) return;
        errorMessage = `HLS URL 再発行失敗: ${e}`;
        loadingMessage = null;
        return;
      }
    }
    if (hls !== inflight) return;
    inflight.loadSource(url);
  }

  function pickBestLevelIndex(levels: Level[]): number {
    if (!levels.length) return -1;
    let bestIdx = 0;
    let bestScore = -1;
    levels.forEach((lv, i) => {
      const h = lv.height ?? 0;
      const br = lv.bitrate ?? 0;
      // Height dominates (720p > 480p), bitrate breaks ties.
      const score = h * 1_000_000 + br;
      if (score > bestScore) {
        bestScore = score;
        bestIdx = i;
      }
    });
    return bestIdx;
  }

  function attachHls() {
    if (!video || !hlsUrl) return;
    detachHls();
    errorMessage = null;
    loadingMessage = 'HLS を初期化中…';
    reissueAttempts = 0;
    mediaRecoveryAttempts = 0;
    networkRecoveryAttempts = 0;
    stallRecoveryAttempts = 0;
    consecutiveLoadedFrags = 0;
    if (Hls.isSupported()) {
      disableSubtleCryptoOnce();
      hls = new Hls({
        enableWorker: false,
        debug: false,
        loader: TauriHlsLoader,
        enableSoftwareAES: true,
        lowLatencyMode: false,
        maxBufferHole: 0.5,
        maxFragLookUpTolerance: 0.5,
        maxBufferLength: 30,
        maxMaxBufferLength: 300,
        highBufferWatchdogPeriod: 3,
        nudgeMaxRetry: 8,
        backBufferLength: 30,
        // 画面サイズで画質を絞らない（Linux/HiDPI で誤縮退しがち）
        capLevelToPlayerSize: false,
        // 初期レベル: マニフェスト解析時に最高画質へロックする。
        // -1 のままだと帯域推定で低画質スタートになり、最初の数秒の
        // 印象画質が落ちる。startLevel もマニフェスト後にロックする。
        startLevel: -1,
        // 既定推定帯域を上げる: niconico の上位画質は 5-10Mbps 出るので
        // 5Mbps 推定だと初手で 720p が選ばれてからアップグレードする
        // 挙動になる。10Mbps にして初手から 1080p を取りにいく。
        abrEwmaDefaultEstimate: 10_000_000,
        manifestLoadingMaxRetry: 6,
        manifestLoadingRetryDelay: 500,
        manifestLoadingMaxRetryTimeout: 64_000,
        levelLoadingMaxRetry: 6,
        levelLoadingRetryDelay: 500,
        levelLoadingMaxRetryTimeout: 64_000,
        fragLoadingMaxRetry: 8,
        fragLoadingRetryDelay: 500,
        fragLoadingMaxRetryTimeout: 64_000,
      });
      hls.attachMedia(video);
      hls.on(Hls.Events.MEDIA_ATTACHED, () => {
        loadingMessage = 'プレイリストを取得中…';
        void loadFreshSource(false);
      });
      hls.on(Hls.Events.MANIFEST_PARSED, () => {
        loadingMessage = null;
        reissueAttempts = 0;
        mediaRecoveryAttempts = 0;
        networkRecoveryAttempts = 0;
        if (!hls) return;
        hlsLevels = hls.levels ?? [];

        let targetIdx = -1;
        if (initialQualityLabel && hls.levels) {
          targetIdx = hls.levels.findIndex(
            (l) =>
              l.height?.toString() === initialQualityLabel?.replace('p', '') ||
              l.name === initialQualityLabel,
          );
        }
        if (targetIdx < 0 && hls.levels && hls.levels.length > 0) {
          targetIdx = pickBestLevelIndex(hls.levels);
        }
        if (targetIdx >= 0) {
          // Lock quality immediately — ABR is useless with custom IPC loader
          // because it misinterprets IPC latency as low bandwidth.
          hls.currentLevel = targetIdx;
          userPickedLevel = targetIdx;
          currentLevel = targetIdx;
        }
        console.log(
          '[Player] MANIFEST_PARSED levels=',
          hls.levels?.map((l, i) => `${i}:${l.height}p/${Math.round((l.bitrate ?? 0) / 1000)}kbps`),
          'locked=',
          targetIdx,
        );
      });
      hls.on(Hls.Events.LEVEL_SWITCHED, (_e, data) => {
        // If ABR tries to switch away from user's chosen level, force it back
        if (userPickedLevel >= 0 && data.level !== userPickedLevel && hls) {
          hls.currentLevel = userPickedLevel;
        } else {
          currentLevel = data.level;
        }
      });
      // Successful fragment loads → progressively restore recovery budget.
      // 単発の 403/伸縮で全予算を消費して停止するのを防ぐ。
      hls.on(Hls.Events.FRAG_LOADED, () => {
        consecutiveLoadedFrags += 1;
        if (consecutiveLoadedFrags >= RESET_AFTER_LOADED_FRAGS) {
          if (
            reissueAttempts > 0 ||
            networkRecoveryAttempts > 0 ||
            mediaRecoveryAttempts > 0 ||
            stallRecoveryAttempts > 0
          ) {
            reissueAttempts = 0;
            networkRecoveryAttempts = 0;
            mediaRecoveryAttempts = 0;
            stallRecoveryAttempts = 0;
            consecutiveLoadedFrags = 0;
          }
        }
        if (loadingMessage) loadingMessage = null;
      });
      hls.on(Hls.Events.ERROR, (_event, data) => {
        consecutiveLoadedFrags = 0;
        const detail = [data.type, data.details, data.reason, data.response?.text]
          .filter(Boolean)
          .join(' / ');

        // バッファが空で止まったケース: 軽くナッジしてから startLoad
        if (!data.fatal && data.details === 'bufferStalledError') {
          if (stallRecoveryAttempts < MAX_RECOVERY_ATTEMPTS && hls && video) {
            stallRecoveryAttempts += 1;
            if (stallNudgeTimer) clearTimeout(stallNudgeTimer);
            stallNudgeTimer = setTimeout(() => {
              if (!hls || !video) return;
              try {
                hls.startLoad();
              } catch {
                /* */
              }
              // micro-nudge: わずかにシークして decoder を起こす
              try {
                video.currentTime = video.currentTime + 0.01;
              } catch {
                /* */
              }
            }, 200);
            showNonFatal(
              `バッファ停止 — 再開中 (${stallRecoveryAttempts}/${MAX_RECOVERY_ATTEMPTS})`,
            );
            return;
          }
        }

        if (!data.fatal && data.details === 'bufferSeekOverHole') {
          if (nonFatalCount < 2) showNonFatal(`HLS: ${detail}`);
          return;
        }

        // 非 fatal の levelLoadError / fragLoadError は hls.js の内部リトライに
        // 任せるが、ユーザに見える非 fatal バナーで気付けるようにしておく。
        if (!data.fatal) {
          if (!errorMessage) showNonFatal(`HLS: ${detail}`);
          return;
        }

        // 403 on the manifest = signed URL expired. Re-issue first.
        const responseText = typeof data.response?.text === 'string' ? data.response.text : '';
        const reasonText = typeof data.reason === 'string' ? data.reason : '';
        const looksLikeExpiry =
          (data.details === 'manifestLoadError' ||
            data.details === 'levelLoadError' ||
            data.details === 'fragLoadError') &&
          (data.response?.code === 403 ||
            responseText.includes('403') ||
            reasonText.includes('403'));
        if (looksLikeExpiry && refreshHlsUrl && reissueAttempts < MAX_HLS_REISSUE_RETRIES) {
          reissueAttempts += 1;
          loadingMessage = `URL 期限切れ — 再発行中 (${reissueAttempts}/${MAX_HLS_REISSUE_RETRIES})…`;
          void loadFreshSource(true);
          return;
        }

        switch (data.type) {
          case Hls.ErrorTypes.NETWORK_ERROR: {
            // Try a URL re-issue once before giving up — fragment 403s after
            // a long pause are the common transient case.
            if (refreshHlsUrl && reissueAttempts < MAX_HLS_REISSUE_RETRIES) {
              reissueAttempts += 1;
              loadingMessage = `通信エラー — URL を再発行中 (${reissueAttempts}/${MAX_HLS_REISSUE_RETRIES})…`;
              void loadFreshSource(true);
              return;
            }
            if (networkRecoveryAttempts < MAX_RECOVERY_ATTEMPTS && hls) {
              networkRecoveryAttempts += 1;
              loadingMessage = `通信エラー — 再試行中 (${networkRecoveryAttempts}/${MAX_RECOVERY_ATTEMPTS})…`;
              // 指数バックオフ: 0.5s, 1s, 2s
              const delay = 500 * Math.pow(2, networkRecoveryAttempts - 1);
              setTimeout(() => {
                try {
                  hls?.startLoad();
                } catch {
                  /* */
                }
              }, delay);
              return;
            }
            break;
          }
          case Hls.ErrorTypes.MEDIA_ERROR: {
            if (mediaRecoveryAttempts < MAX_RECOVERY_ATTEMPTS && hls) {
              mediaRecoveryAttempts += 1;
              loadingMessage = `デコードエラー — 復旧試行中 (${mediaRecoveryAttempts}/${MAX_RECOVERY_ATTEMPTS})…`;
              if (mediaRecoveryAttempts === 1) {
                hls.recoverMediaError();
              } else {
                hls.swapAudioCodec();
                hls.recoverMediaError();
              }
              return;
            }
            break;
          }
          default:
            break;
        }

        // ここまで来たら通常リカバリでは復帰できない。最終手段として
        // HLS インスタンスを作り直して URL も再発行する。これでも
        // ダメなら諦めてエラー表示を出す。
        if (refreshHlsUrl && reissueAttempts < MAX_HLS_REISSUE_RETRIES + 1) {
          reissueAttempts += 1;
          loadingMessage = `致命的エラー — 完全再接続中 (${reissueAttempts})…`;
          setTimeout(() => {
            attachHls();
          }, 300);
          return;
        }

        errorMessage = `HLS エラー: ${detail}`;
        loadingMessage = null;
      });
    } else if (video.canPlayType('application/vnd.apple.mpegurl')) {
      video.src = hlsUrl;
      loadingMessage = null;
    } else {
      errorMessage = 'この WebView は HLS をサポートしていません';
      loadingMessage = null;
    }
  }

  function detachHls() {
    if (hls) {
      hls.destroy();
      hls = null;
    }
  }

  // Single $effect: attach HLS when video element and hlsUrl are ready.
  // localSrc が指定されている時は HLS を完全にスキップして直接 src= に流す。
  let hlsUrlPrev = '';
  let localSrcPrev = '';
  $effect(() => {
    const v = video;
    if (!v) return;
    if (localSrc) {
      // ローカルファイル再生モード — HLS インスタンスは作らない
      detachHls();
      if (localSrc !== localSrcPrev) {
        localSrcPrev = localSrc;
        v.src = localSrc;
        loadingMessage = null;
        errorMessage = null;
        clearPendingVideoError();
      }
      return;
    }
    const url = hlsUrl;
    if (!url) return;
    if (url === hlsUrlPrev && hls) return; // already attached to this URL
    hlsUrlPrev = url;
    attachHls();
  });

  onDestroy(() => {
    detachHls();
    if (nonFatalTimer) clearTimeout(nonFatalTimer);
    if (hideTimer) clearTimeout(hideTimer);
    if (stallNudgeTimer) clearTimeout(stallNudgeTimer);
    if (seekUnhideTimer) clearTimeout(seekUnhideTimer);
    clearPendingVideoError();
  });

  function togglePlay() {
    if (!video) return;
    if (video.paused) void video.play().catch(() => undefined);
    else video.pause();
  }
  /** クランプ用に有効な duration を返す。`video.duration` が NaN/0 のうち
   *  (metadata 未ロード時) は呼び出し側で「巻き戻り」が起きないよう Infinity を返す。 */
  function effectiveDuration(): number {
    const vd = video?.duration ?? NaN;
    if (Number.isFinite(vd) && vd > 0) return vd;
    if (duration > 0) return duration;
    return Infinity;
  }

  // metadata が来てない時に seek 要求が来たら、ロード完了後に適用するために
  // 退避しておく。これが無いと先頭巻き戻り or 無反応になる。
  let pendingSeek: number | null = null;

  function applyPendingSeek() {
    if (!video || pendingSeek == null) return;
    const t = pendingSeek;
    pendingSeek = null;
    seekTo(t);
  }

  function seekDelta(delta: number) {
    if (!video) return;
    seekTo(video.currentTime + delta);
  }
  function seekTo(t: number) {
    if (!video) return;
    if (!Number.isFinite(t)) return;
    // metadata 未ロードだと currentTime 代入が無視 / 失敗する WebKit 挙動が
    // あるので、readyState>=1 (HAVE_METADATA) を待ってから適用する。
    if (video.readyState < 1) {
      pendingSeek = Math.max(0, t);
      return;
    }
    let target = Math.max(0, t);
    const d = video.duration;
    if (Number.isFinite(d) && d > 0) {
      target = Math.min(target, d - 0.05);
    }
    // 後方 seek は WebKitGTK + GStreamer + Blob URL の組合せで GOP リセットが
    // 雑になり、緑ノイズ / 前フレーム残骸 (= "ガビガビ") が出やすい。
    // fastSeek が使えるならキーフレームへ直接 snap させて decode 部分を省く。
    // 前方 seek は普通通り currentTime で精度優先。
    const isBackward = target < video.currentTime;
    const fast = (video as HTMLVideoElement & { fastSeek?: (t: number) => void }).fastSeek;
    try {
      if (isBackward && typeof fast === 'function') {
        fast.call(video, target);
      } else {
        video.currentTime = target;
      }
    } catch (e) {
      // fastSeek 失敗時は currentTime にフォールバック

      console.error('[Player] seekTo failed, falling back', e, 'target=', target);
      try {
        video.currentTime = target;
      } catch (e2) {
        console.error('[Player] currentTime fallback also failed', e2);
      }
    }
  }
  function jumpToFraction(frac: number) {
    if (!video) return;
    const d = effectiveDuration();
    if (!Number.isFinite(d) || d <= 0) return;
    seekTo(d * frac);
  }
  function setVolume(v: number) {
    if (!video) return;
    const next = Math.max(0, Math.min(1, v));
    video.volume = next;
    if (next > 0 && video.muted) video.muted = false;
  }
  function toggleMute() {
    if (!video) return;
    video.muted = !video.muted;
  }
  function setRate(r: number) {
    if (!video) return;
    video.playbackRate = r;
  }
  function toggleComments() {
    commentsEnabled = !commentsEnabled;
  }
  function setQuality(levelIndex: number) {
    if (!hls) return;
    userPickedLevel = levelIndex;
    hls.currentLevel = levelIndex;
    currentLevel = levelIndex;
  }
  function setCommentOpacity(o: number) {
    commentOpacity = o;
  }
  function setAbIn() {
    if (!video) return;
    abLoop = { ...abLoop, in: video.currentTime };
  }
  function setAbOut() {
    if (!video) return;
    abLoop = { ...abLoop, out: video.currentTime };
  }
  function toggleAbLoop() {
    if (abLoop.in == null || abLoop.out == null) return;
    abLoop = { ...abLoop, enabled: !abLoop.enabled };
  }
  function clearAb() {
    abLoop = { in: null, out: null, enabled: false };
  }
  function frameStep(forward: boolean) {
    if (!video) return;
    if (!video.paused) video.pause();
    video.currentTime += forward ? 1 / 30 : -1 / 30;
  }

  type FullscreenDocument = Document & {
    webkitFullscreenElement?: Element | null;
    webkitExitFullscreen?: () => Promise<void> | void;
  };
  type FullscreenElement = HTMLElement & {
    webkitRequestFullscreen?: () => Promise<void> | void;
  };
  function getFullscreenEl(): Element | null {
    const d = document as FullscreenDocument;
    return d.fullscreenElement ?? d.webkitFullscreenElement ?? null;
  }
  function exitFullscreen() {
    const d = document as FullscreenDocument;
    if (d.exitFullscreen) void d.exitFullscreen();
    else if (d.webkitExitFullscreen) void d.webkitExitFullscreen();
  }
  function requestFullscreen(el: HTMLElement) {
    const e = el as FullscreenElement;
    if (e.requestFullscreen) void e.requestFullscreen();
    else if (e.webkitRequestFullscreen) void e.webkitRequestFullscreen();
  }
  function toggleFullscreen() {
    if (!stage) return;
    if (getFullscreenEl()) exitFullscreen();
    else requestFullscreen(stage);
  }
  function onFullscreenChange() {
    isFullscreen = getFullscreenEl() === stage;
    showControls();
  }

  function onEnded() {
    if (!video) return;
    if (loop) {
      video.currentTime = 0;
      void video.play().catch(() => undefined);
    } else {
      paused = true;
      showControls();
    }
  }

  function onTimeUpdate() {
    if (!video) return;
    // フレームが進んでる = 再生できてる → 待機中の一過性 error は無視
    if (pendingVideoErrorTimer && video.currentTime > 0) {
      clearPendingVideoError();
    }
    const now = performance.now();
    if (now - lastTimeUpdateTs < 200) return;
    lastTimeUpdateTs = now;
    currentTime = video.currentTime;
    onTime?.(video.currentTime);
    maybeCorrectDrift();
    if (
      abLoop.enabled &&
      abLoop.in != null &&
      abLoop.out != null &&
      abLoop.out > abLoop.in &&
      video.currentTime >= abLoop.out
    ) {
      video.currentTime = abLoop.in;
    }
  }
  let resumeApplied = false;

  function onDurationChange() {
    if (!video) return;
    duration = Number.isFinite(video.duration) ? video.duration : 0;
    // Restore saved position once duration is available
    if (!resumeApplied && resumePosition > 0 && duration > 0) {
      resumeApplied = true;
      if (resumePosition < duration - 1) {
        video.currentTime = resumePosition;
      }
    }
    // metadata 来たので保留中の seek 要求を消化
    applyPendingSeek();
  }
  function onLoadedMetadata() {
    applyPendingSeek();
    if (!video) return;
    // 設定からデフォルト値を反映
    const defaultVol = getNum('playback.default_volume');
    const defaultRate = getNum('playback.default_rate');
    const autoplay = getBool('playback.autoplay');
    if (Number.isFinite(defaultVol)) {
      video.volume = Math.max(0, Math.min(1, defaultVol));
    }
    if (Number.isFinite(defaultRate) && defaultRate > 0) {
      video.playbackRate = defaultRate;
    }
    if (autoplay) {
      void video.play().catch(() => undefined);
    }
  }
  function onPlayState() {
    if (!video) return;
    paused = video.paused;
    if (video.paused) showControls();
    syncAudioPlayState();
    // 再生開始 = 一過性 error は無視
    if (!video.paused) {
      clearPendingVideoError();
      // 復旧後にエラーバナーが残っていれば消す
      if (errorMessage && video.readyState >= 2) errorMessage = null;
    }
  }
  function onVolumeChange() {
    if (!video) return;
    volume = video.volume;
    muted = video.muted;
    syncAudioVolume();
  }
  function onSeeking() {
    isSeeking = true;
    if (seekUnhideTimer) clearTimeout(seekUnhideTimer);
    syncAudioSeek();
  }
  function onSeeked() {
    syncAudioSeek();
    syncAudioPlayState();
    // decode が新フレームを描画するまで 1 frame 待ってから戻す
    // (即解除すると古いフレーム or ガベージが一瞬見える)
    if (seekUnhideTimer) clearTimeout(seekUnhideTimer);
    seekUnhideTimer = setTimeout(() => {
      isSeeking = false;
      seekUnhideTimer = null;
    }, 60);
  }
  function onRateChange() {
    if (!video) return;
    playbackRate = video.playbackRate;
    if (audioEl) audioEl.playbackRate = video.playbackRate;
  }

  // ============== Audio dual-element 同期 ==============
  // localAudioSrc が指定された時のみ動く。<audio> を play/pause/seek/rate/mute/
  // volume で video に追従させる。ドリフトしたら currentTime を強制合わせ。
  const AUDIO_DRIFT_THRESHOLD = 0.12;
  let lastDriftCorrection = 0;

  $effect(() => {
    if (!audioEl) return;
    if (localAudioSrc) {
      audioEl.src = localAudioSrc;
      audioEl.preload = 'auto';
    } else {
      audioEl.removeAttribute('src');
    }
  });

  function syncAudioPlayState() {
    if (!video || !audioEl || !localAudioSrc) return;
    if (video.paused !== audioEl.paused) {
      if (video.paused) audioEl.pause();
      else void audioEl.play().catch(() => undefined);
    }
  }

  function syncAudioSeek() {
    if (!video || !audioEl || !localAudioSrc) return;
    audioEl.currentTime = video.currentTime;
  }

  function maybeCorrectDrift() {
    if (!video || !audioEl || !localAudioSrc) return;
    const now = performance.now();
    if (now - lastDriftCorrection < 500) return;
    if (video.paused) return;
    const drift = Math.abs(video.currentTime - audioEl.currentTime);
    if (drift > AUDIO_DRIFT_THRESHOLD) {
      audioEl.currentTime = video.currentTime;
      lastDriftCorrection = now;
    }
  }

  function syncAudioVolume() {
    if (!video || !audioEl) return;
    audioEl.volume = video.volume;
    audioEl.muted = video.muted;
  }

  onMount(() => {
    document.addEventListener('webkitfullscreenchange', onFullscreenChange);
    // compact (PiP) モードでは window レベルのショートカットを登録しない。
    // ミニ側と通常ページ側の <Player> が同時に存在する状況で 1 キーが
    // 2 重発火するのを防ぐ。ミニ側専用のショートカットは MiniPlayer が
    // 別途登録する。
    let unbindShortcuts: (() => void) | null = null;
    if (!compact) {
      const actions: PlayerActions = {
        togglePlay,
        seekDelta,
        jumpToFraction,
        toggleComments,
        toggleFullscreen,
        toggleMute,
        setAbIn,
        setAbOut,
        toggleAbLoop,
        volumeDelta: (d) => setVolume((video?.volume ?? volume) + d),
        frameStep,
        togglePip: onTogglePip ? () => onTogglePip?.() : undefined,
      };
      unbindShortcuts = bindShortcuts(window, actions);
    }
    return () => {
      document.removeEventListener('webkitfullscreenchange', onFullscreenChange);
      unbindShortcuts?.();
    };
  });

  export function getVideo(): HTMLVideoElement | null {
    return video;
  }
  export function seek(t: number) {
    seekTo(t);
  }
  export function play() {
    if (!video) return;
    void video.play().catch(() => undefined);
  }
  export function pause() {
    if (!video) return;
    video.pause();
  }
  export function getCurrentTime(): number {
    return video?.currentTime ?? currentTime;
  }
</script>

<svelte:window onfullscreenchange={onFullscreenChange} />

{#if errorMessage}
  <div class="fatal-error">
    <div>{errorMessage}</div>
    {#if errorMessage.includes('decode') || errorMessage.includes('DECODE') || errorMessage.includes('SRC_NOT_SUPPORTED')}
      <div class="fatal-tip">
        💡 ストリーミング再生でデコード失敗するケースは、niconico の最新コーデック (AV1 等) を
        WebView の GStreamer が食えてないことが多いです。
        <strong>ダウンロードしてローカル再生</strong>すると yt-dlp + ffmpeg が H.264/AAC
        に変換して保存するので、ほぼ解決します。
      </div>
    {/if}
  </div>
{/if}

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="player"
  class:fullscreen={isFullscreen}
  bind:this={stage}
  tabindex="-1"
  onmousemove={showControls}
>
  {#if isSeeking}
    <div class="seek-mask" aria-hidden="true"></div>
  {/if}
  <video
    bind:this={video}
    style:visibility={isSeeking ? 'hidden' : 'visible'}
    onplay={onPlayState}
    onpause={onPlayState}
    onended={onEnded}
    ontimeupdate={onTimeUpdate}
    ondurationchange={onDurationChange}
    onloadedmetadata={onLoadedMetadata}
    onvolumechange={onVolumeChange}
    onratechange={onRateChange}
    onseeking={onSeeking}
    onseeked={onSeeked}
    onerror={() => {
      const code = video?.error?.code ?? 0;
      const codeMap: Record<number, string> = {
        1: 'MEDIA_ERR_ABORTED',
        2: 'MEDIA_ERR_NETWORK',
        3: 'MEDIA_ERR_DECODE',
        4: 'MEDIA_ERR_SRC_NOT_SUPPORTED',
      };
      const detail = video?.error?.message || codeMap[code] || `code ${code}`;

      // 初期バッファリング中の MEDIA_ERR_DECODE は WebKitGTK + GStreamer で
      // 頻発する一過性エラー。play/timeupdate が来れば自然回復する。
      // console 出力は debug レベルに下げてノイズを減らす。
      if (code === 3) {
        console.debug(
          '[Player] <video> decode error (likely transient):',
          detail,
          'src=',
          video?.currentSrc,
        );
      } else {
        console.warn('[Player] <video> error:', detail, 'src=', video?.currentSrc);
      }

      // SRC_NOT_SUPPORTED は本質的に詰みなので即表示。
      // それ以外 (decode/network 系) は 3s 様子見して、
      // その間に play / timeupdate が走ったら一過性として無視する。
      if (code === 4) {
        errorMessage = `動画再生エラー: ${detail}`;
        return;
      }
      clearPendingVideoError();
      pendingVideoErrorTimer = setTimeout(() => {
        pendingVideoErrorTimer = null;
        // currentTime が進んでいる / 再生中なら無視
        const recovered =
          !!video && (!video.paused || (video.currentTime > 0 && video.readyState >= 2));
        if (recovered) return;
        errorMessage = `動画再生エラー: ${detail}`;
      }, 3000);
    }}
    preload="auto"
  ></video>
  {#if localAudioSrc}
    <audio
      bind:this={audioEl}
      preload="auto"
      onerror={() => {
        const code = audioEl?.error?.code ?? 0;

        console.error('[Player] <audio> error: code', code, 'src=', audioEl?.currentSrc);
      }}
      style="display:none"
    ></audio>
  {/if}
  <!-- 動画ソース (localSrc / hlsUrl) が変わったら CommentLayer を remount。
       これで前動画の canvas ピクセルが残像として残るのを確実に防ぐ。 -->
  {#key localSrc || hlsUrl}
    <CommentLayer {video} {comments} enabled={commentsEnabled} opacity={commentOpacity} />
  {/key}
  {#if loadingMessage}
    <div class="loading">{loadingMessage}</div>
  {/if}
  {#if !compact}
    <div class="controls-wrap" class:visible={controlsVisible}>
      <ControlBar
        {video}
        {paused}
        {currentTime}
        {duration}
        {volume}
        {muted}
        {playbackRate}
        {commentsEnabled}
        {commentOpacity}
        {abLoop}
        {hlsLevels}
        {currentLevel}
        {loop}
        {pipActive}
        showPip={!!onTogglePip}
        onTogglePlay={togglePlay}
        onSeek={seekTo}
        onVolume={setVolume}
        onToggleMute={toggleMute}
        onRate={setRate}
        onToggleComments={toggleComments}
        onCommentOpacity={setCommentOpacity}
        onSetAbIn={setAbIn}
        onSetAbOut={setAbOut}
        onToggleAb={toggleAbLoop}
        onClearAb={clearAb}
        onToggleLoop={() => {
          loop = !loop;
        }}
        onFullscreen={toggleFullscreen}
        onQuality={setQuality}
        onTogglePip={() => onTogglePip?.()}
      />
    </div>
  {/if}
</div>

<style>
  .player {
    position: relative;
    background: #000;
    border-radius: 8px;
    overflow: hidden;
    outline: none;
  }

  .seek-mask {
    position: absolute;
    inset: 0;
    background: #000;
    z-index: 4;
    pointer-events: none;
  }

  .player.fullscreen {
    border-radius: 0;
  }

  .player :global(video) {
    display: block;
    width: 100%;
    aspect-ratio: 16 / 9;
    object-fit: contain;
    background: #000;
  }

  .player.fullscreen :global(video) {
    width: 100%;
    height: 100%;
  }

  .loading {
    position: absolute;
    bottom: 12px;
    left: 12px;
    right: 12px;
    background: rgba(20, 20, 20, 0.78);
    color: #eaeaea;
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
    pointer-events: none;
    z-index: 5;
  }

  .controls-wrap {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    z-index: 20;
    opacity: 0;
    transition: opacity 0.25s ease;
    pointer-events: none;
  }

  .controls-wrap.visible {
    opacity: 1;
    pointer-events: auto;
  }

  .fatal-error {
    background: #2a1212;
    border: 1px solid #5a2222;
    color: #f5b3b3;
    padding: 10px 12px;
    border-radius: 6px;
    font-size: 13px;
    margin-bottom: 8px;
    white-space: pre-wrap;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .fatal-tip {
    background: rgba(37, 99, 235, 0.15);
    border: 1px solid #2a3f5a;
    color: #c5d8f5;
    padding: 8px 10px;
    border-radius: 4px;
    font-size: 12px;
    line-height: 1.6;
  }
</style>
