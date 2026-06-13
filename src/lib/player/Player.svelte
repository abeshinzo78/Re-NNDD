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
  import { extractOnlineFrame, extractVideoFrame } from '$lib/api';
  import { getBool, getNum } from '$lib/stores/settings.svelte';
  import { readSavedMuted, readSavedVolume, saveMuted, saveVolume } from './volumePersistence';
  import * as pluginBus from '$lib/plugins/eventBus';
  import { pluginPlayerActions } from '$lib/plugins/registry';
  import { clearPlayerState, updatePlayerState } from '$lib/plugins/playerState.svelte';

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
    /** ショート動画 (縦長) 用モード。9:16 レイアウト。 */
    short?: boolean;
    /** PiP ボタンが押された時のフック (compact=false 時のみ表示) */
    onTogglePip?: () => void;
    /** PiP ボタンの aria-pressed 表示用 */
    pipActive?: boolean;
    /** ループ設定が変わった時の通知 (親の state を更新するため) */
    onLoopChange?: (value: boolean) => void;
    /** 動画タイトル（スクリーンショットのファイル名に使用） */
    videoTitle?: string;
    /** 動画 ID（スクリーンショットのファイル名に使用） */
    videoId?: string;
    /** 音声を出さずにロード/再生開始する。
     *  PiP 切替時、ページ側 Player が音声を出し続けているあいだに mini を
     *  無音でバックグラウンドロードするのに使う。playing イベントが発火し
     *  たら `onReadyForAudio` を呼ぶ。親 (MiniPlayer) はそこで音量を戻して
     *  `audioOwned` を立て、ページ側 Player をプレースホルダに退かせる。 */
    initialMuted?: boolean;
    /** 無音ロードが完了し音声を引き継げる状態になった通知。`initialMuted=true`
     *  の時のみ発火し、1 度だけ呼ばれる。 */
    onReadyForAudio?: () => void;
    /** 動画が自然終了した通知。`loop` 中は発火しない (内部でループ巻き戻しのみ行う)。
     *  オートプレイキューの「次の動画へ進む」フックを親が刺すのに使う。 */
    onEnded?: () => void;
    /** `playback.autoplay=false` 設定でも自動再生を強制する。連続再生キューの
     *  ような「ユーザが明示的に再生継続を指示した」コンテキストで使う。 */
    forceAutoplay?: boolean;
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
    short = false,
    onTogglePip,
    pipActive = false,
    onLoopChange,
    initialMuted = false,
    onReadyForAudio,
    onEnded: onEndedExternal,
    forceAutoplay = false,
    videoTitle = '',
    videoId = '',
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
  let screenshotMsg = $state<string | null>(null);
  let hideTimer: ReturnType<typeof setTimeout> | null = null;
  let hlsLevels = $state<Level[]>([]);
  let currentLevel = $state(-1);
  let lastTimeUpdateTs = 0;
  let userPickedLevel = -1;
  // 画質切り替えで hls.js を作り直す時、新インスタンスをユーザが選んだ画質に固定
  // するための退避値。index は manifest インスタンスごとにローカルで、URL 再発行で
  // 並びが変わりうるため、安定なセレクタ (height) を保持し MANIFEST_PARSED で index を
  // 解決し直す。null = 自動/最良。別動画へ遷移したら $effect でリセットする。
  let desiredHeight: number | null = null;
  // 画質切り替えで再アタッチした直後に復元する再生状態 (再生/一時停止) と速度。
  // 再生位置は pendingSeek 経由で復元する (再アタッチ中の新しいシークが優先)。
  // rateExplicit: 再アタッチ中にユーザが明示的に速度変更したか。
  // silentPlay: 復元再生で player:play を抑制するか (既に再生中だった「継続」は
  //   true。停止→再生の新規/明示再生は false で onPlaying 時に通知する)。
  let restoreAfterReattach: {
    play: boolean;
    rate: number;
    rateExplicit?: boolean;
    silentPlay?: boolean;
  } | null = null;
  // 復元再生 (restore の play) 由来の onPlaying を 1 度だけ抑制するフラグ。
  // 内部的な再開なので player:play プラグインイベントを出さないために使う。
  let suppressPlayEventOnce = false;
  // metadata 未ロード時の seek 要求を退避し、ロード後に適用する。画質切り替えの
  // 再アタッチでは「復元する再生位置」もこれで運ぶ (最新の seek が勝つ)。
  let pendingSeek: number | null = null;
  // 直近に実際にロードした HLS URL (403 で再発行され差し替わった最新版を含む)。
  // 画質切り替えの再アタッチでは prop の初期 URL ではなくこれを使い、期限切れ
  // URL を読み直して 403 → 再発行のラウンドトリップを踏むのを避ける。
  let activeHlsUrl = '';
  // 画質切り替えの再アタッチ進行中フラグ。MediaSource 付け替えで <video> が
  // 一時的に pause→再生再開するが、これはユーザ操作ではないので player:pause /
  // player:play プラグインイベントを抑制する。CommentLayer の freeze に渡すため
  // $state にする (非リアクティブだと子へ変化が伝わらない)。
  let reattaching = $state(false);
  // 一度でも実際に再生が始まったか。再生開始前の画質切替は復元せず通常の
  // ロード/オートプレイ経路に任せる (autoplay 設定が無視されるのを防ぐ)。
  let playbackStarted = false;
  // 初回ロードの既定 (音量/ミュート/速度) を適用済みか。初回 loadedmetadata 前の
  // 早期画質切替で既定適用がスキップされ full-volume 等で再生されるのを防ぐ判定。
  let initialized = false;
  // ユーザ/プラグインが明示的に一時停止し、まだ明示再生していないか。初回フレーム
  // 前の明示停止が autoplay 意図で上書きされるのを防ぐために使う。
  let userPaused = false;
  // 復元シーク (再アタッチ後に保存位置へ戻すシーク) の進行中フラグ。この間も
  // コメント overlay を凍結し、restore seek で canvas が一瞬空になるのを防ぐ。
  // CommentLayer の freeze に渡すため $state。
  let restoreSeeking = $state(false);
  let restoreSeekTimer: ReturnType<typeof setTimeout> | null = null;
  // エラー時の遅延再接続 (attachHls) タイマー。手動再アタッチ時に取り消し、古い
  // タイマーが復元済みソースを壊さないようにする。
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  // 直前の再アタッチが terminal 失敗で畳まれたか。失敗した同一画質の再選択を
  // 許すために使う (MANIFEST_PARSED 成功 / 別動画でクリア)。
  let reattachFailed = false;

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

  function canvasHasRenderedVideoPixels(canvas: HTMLCanvasElement): boolean {
    const probe = document.createElement('canvas');
    probe.width = 8;
    probe.height = 8;
    const probeCtx = probe.getContext('2d', { willReadFrequently: true });
    if (!probeCtx) return false;
    probeCtx.drawImage(canvas, 0, 0, probe.width, probe.height);
    const { data } = probeCtx.getImageData(0, 0, probe.width, probe.height);
    for (let i = 3; i < data.length; i += 4) {
      if (data[i] !== 0) return true;
    }
    return false;
  }

  async function takeScreenshot() {
    const container = stage;
    const captureComments = commentsEnabled;
    const t = currentLogicalTime();
    if (!container) return;
    screenshotMsg = 'スクリーンショット準備中…';
    const rect = container.getBoundingClientRect();
    let w = Math.round(rect.width);
    let h = Math.round(rect.height);
    if (w === 0 || h === 0) return;

    // 1) Try Rust-side ffmpeg extraction (local file → remote HLS).
    //    Higher quality (exact frame at exact timestamp) when available.
    let frame: ImageBitmap | null = null;
    let b64: string | null = null;
    if (videoId) {
      try {
        b64 = await extractVideoFrame(videoId, t);
      } catch {
        /* ignore */
      }
    }
    if (!b64 && hlsUrl) {
      try {
        b64 = await extractOnlineFrame(hlsUrl, t);
      } catch {
        /* ignore */
      }
    }
    if (b64) {
      try {
        const resp = await fetch(`data:image/png;base64,${b64}`);
        frame = await createImageBitmap(await resp.blob());
      } catch {
        /* ignore */
      }
    }

    // 2) Fall back to drawing the <video> element directly when ffmpeg
    //    extraction is unavailable (no ffmpeg / no local file / HLS fetch
    //    failed). HLS.js feeds the element via MSE so the canvas is not
    //    CORS-tainted. Use the intrinsic video size for higher fidelity
    //    than the player rect would give us.
    let videoFallbackDrawn = false;
    const canUseVideoFallback =
      !!video && video.readyState >= 2 && video.videoWidth > 0 && video.videoHeight > 0;
    if (!frame && canUseVideoFallback && video) {
      w = video.videoWidth;
      h = video.videoHeight;
    }

    const c = document.createElement('canvas');
    c.width = w;
    c.height = h;
    const ctx = c.getContext('2d');
    if (!ctx) return;
    if (frame) {
      // Preserve aspect ratio: center the frame, fill black letterbox.
      const vr = frame.width / frame.height;
      const cr = w / h;
      let dw: number, dh: number;
      if (vr > cr) {
        dw = w;
        dh = Math.round(w / vr);
      } else {
        dh = h;
        dw = Math.round(h * vr);
      }
      ctx.drawImage(frame, (w - dw) / 2, (h - dh) / 2, dw, dh);
      frame.close();
    } else if (canUseVideoFallback && video) {
      try {
        ctx.drawImage(video, 0, 0, w, h);
        videoFallbackDrawn = canvasHasRenderedVideoPixels(c);
        if (!videoFallbackDrawn) {
          console.warn('[Player] screenshot: video drawImage produced a blank canvas');
        }
      } catch (e) {
        console.warn('[Player] screenshot: video drawImage failed', e);
      }
    }

    // 3) Composite comment canvas overlay (scaled to match the chosen size).
    const commentCanvas = container.querySelector<HTMLCanvasElement>('canvas.layer');
    if (captureComments && commentCanvas && commentCanvas.width > 0 && commentCanvas.height > 0) {
      ctx.drawImage(commentCanvas, 0, 0, w, h);
    }

    // 4) Bail out if we have absolutely nothing to draw — a blank canvas
    //    with only a comment overlay (or worse, fully empty) was the
    //    pre-fix failure mode and is never what the user wants.
    if (!frame && !videoFallbackDrawn) {
      screenshotMsg = 'スクリーンショット取得に失敗しました';
      setTimeout(() => (screenshotMsg = null), 2500);
      return;
    }

    // 5) Download.
    c.toBlob((blob) => {
      if (!blob) {
        screenshotMsg = 'スクリーンショット取得に失敗しました';
        setTimeout(() => (screenshotMsg = null), 2500);
        return;
      }
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      const mm = String(Math.floor(t / 60)).padStart(2, '0');
      const ss = String(Math.floor(t % 60)).padStart(2, '0');
      const base = videoTitle
        ? videoTitle.replace(/[/\\?%*:|"<>]/g, '_').slice(0, 80)
        : 'screenshot';
      // ':' is invalid in filenames on Windows — use '-' separator.
      a.download = `${base}[${videoId || 'no-id'}]${mm}-${ss}.png`;
      a.href = url;
      a.click();
      URL.revokeObjectURL(url);
      screenshotMsg = 'スクリーンショットを保存しました';
      setTimeout(() => (screenshotMsg = null), 2500);
    });
  }

  async function loadFreshSource(forceRefresh = false) {
    const inflight = hls;
    if (!inflight) return;
    // 既に再発行済みなら最新 URL を基点にする (初回は activeHlsUrl 空で prop が新鮮)。
    // 画質切替の再アタッチで期限切れの初期 prop を読み直して 403 になるのを防ぐ。
    let url = activeHlsUrl || hlsUrl;
    // Only call refreshHlsUrl on error recovery (403 expiry etc.),
    // NOT on initial load — the prop hlsUrl is already fresh.
    if (forceRefresh && refreshHlsUrl) {
      try {
        url = await refreshHlsUrl();
      } catch (e) {
        if (hls !== inflight) return;
        errorMessage = `HLS URL 再発行失敗: ${e}`;
        loadingMessage = null;
        cancelReattach();
        return;
      }
    }
    if (hls !== inflight) return;
    activeHlsUrl = url;
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
    // 再アタッチ開始時に保留中の一過性 video エラータイマーと、エラー再接続の遅延
    // タイマーを止める。後から発火すると復元済みソースを壊したり誤って終了扱いに
    // してしまう。
    clearPendingVideoError();
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
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
        enableWorker: true,
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
        capLevelToPlayerSize: false,
        startLevel: -1,
        abrEwmaDefaultEstimate: 10_000_000,
        // 最初のフラグメントを先行取得して初動遅延を短縮
        startFragPrefetch: true,
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
        reattachFailed = false;
        if (!hls) return;
        hlsLevels = hls.levels ?? [];

        let targetIdx = -1;
        // 画質切り替え/再アタッチ時は、選択画質 (height) を新マニフェストの index に
        // 解決し直してユーザの選択を維持する (index は manifest 毎にローカルで、URL
        // 再発行で並びが変わりうるため index 退避では別解像度に化けうる)。
        if (desiredHeight != null && hls.levels) {
          targetIdx = hls.levels.findIndex((l) => l.height === desiredHeight);
        }
        if (targetIdx < 0 && initialQualityLabel && hls.levels) {
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
          if (reconnectTimer) clearTimeout(reconnectTimer);
          reconnectTimer = setTimeout(() => {
            reconnectTimer = null;
            attachHls();
          }, 300);
          return;
        }

        errorMessage = `HLS エラー: ${detail}`;
        loadingMessage = null;
        cancelReattach();
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
    // 別動画へ切り替わった: 前動画でのユーザ画質固定/復元状態はリセットし、
    // 最良画質から始める (レベル index・URL は動画ごとに意味が異なるため)。
    desiredHeight = null;
    restoreAfterReattach = null;
    suppressPlayEventOnce = false;
    pendingSeek = null;
    restoreSeeking = false;
    if (restoreSeekTimer) {
      clearTimeout(restoreSeekTimer);
      restoreSeekTimer = null;
    }
    // シークマスク状態も畳む (復元シーク中の動画遷移で映像が隠れたまま残らないように)。
    isSeeking = false;
    if (seekUnhideTimer) {
      clearTimeout(seekUnhideTimer);
      seekUnhideTimer = null;
    }
    activeHlsUrl = '';
    reattaching = false;
    playbackStarted = false;
    userPaused = false;
    initialized = false;
    reattachFailed = false;
    // 前動画のマニフェスト由来の画質メニューを残さない (新マニフェスト parse まで
    // 古い levels で setQuality が走り、前動画の currentTime を pendingSeek に拾って
    // 新動画が前動画の位置から始まるのを防ぐ)。新 MANIFEST_PARSED で再投入される。
    hlsLevels = [];
    currentLevel = -1;
    hlsUrlPrev = url;
    attachHls();
  });

  onDestroy(() => {
    detachHls();
    if (nonFatalTimer) clearTimeout(nonFatalTimer);
    if (hideTimer) clearTimeout(hideTimer);
    if (stallNudgeTimer) clearTimeout(stallNudgeTimer);
    if (seekUnhideTimer) clearTimeout(seekUnhideTimer);
    if (restoreSeekTimer) clearTimeout(restoreSeekTimer);
    if (reconnectTimer) clearTimeout(reconnectTimer);
    clearPendingVideoError();
  });

  // 確定的な終了 (terminal) ロード失敗で再アタッチ窓を畳む。これをしないと
  // reattaching / restoreAfterReattach が残り、時刻・再生状態の更新が永久に抑制
  // され、以後の play/pause/rate 操作が「適用されないスナップショット」へ吸い込
  // まれてしまう。一過性エラー (3 秒猶予の video error 等) では呼ばない —
  // HLS がまだ復帰しうるため、誤って再生意図を捨てない。
  function cancelReattach() {
    // 再生中だった切替 (継続再生 = silentPlay) を終了失敗で畳む場合、抑制した
    // detach pause の代わりに player:pause を 1 回出す (subscriber が playing の
    // まま残らないように)。明示一時停止済み (silentPlay=false 化) なら既に通知済み。
    const emitPause = restoreAfterReattach?.silentPlay === true;
    const pausePos = currentLogicalTime();
    reattaching = false;
    restoreAfterReattach = null;
    suppressPlayEventOnce = false;
    // 失敗した切替の再試行で同じ画質を選び直せるようにする (currentLevel は eager に
    // 進めているため、これが無いと同一選択が setQuality 冒頭で弾かれる)。
    reattachFailed = true;
    // 復元シーク関連の UI 状態も畳む。これを怠るとコメント凍結 (restoreSeeking) や
    // シークマスク (isSeeking) が 5 秒 fallback まで残り、別画質での復帰時に映像が
    // 隠れたままになる。
    restoreSeeking = false;
    if (restoreSeekTimer) {
      clearTimeout(restoreSeekTimer);
      restoreSeekTimer = null;
    }
    isSeeking = false;
    if (seekUnhideTimer) {
      clearTimeout(seekUnhideTimer);
      seekUnhideTimer = null;
    }
    // 再アタッチ中に onPlayState で抑制した実際の pause 状態を反映する。これを
    // しないと、失敗して停止した player が controls / plugin / isPaused 上で
    // 再生中のまま見えてしまう。
    if (video) paused = video.paused;
    if (emitPause) pluginBus.emit('player:pause', { videoId, currentTime: pausePos });
  }

  // 画質切り替えの再アタッチ中の「明示的な」再生/一時停止 (ユーザ/プラグイン操作)
  // を復元スナップショットへ反映する。付け替え途中の <video> を直接操作しても
  // 失われるため。内部的な再アタッチ pause/play と違いこれは実ユーザ操作なので、
  // reactive な paused 状態を同期し、プラグインイベントも通常どおり通知する。
  function setReattachPlayIntent(playing: boolean) {
    if (!restoreAfterReattach) return;
    const wasPlay = restoreAfterReattach.play;
    restoreAfterReattach.play = playing;
    // 明示操作なので復元再生は「継続(無音)」ではない — 再生開始時に player:play を出す。
    restoreAfterReattach.silentPlay = false;
    paused = !playing;
    // 一時停止は実状態 (停止) と一致するので通知してよいが、状態遷移時のみ (連続
    // フレームステップ等で player:pause を多重 emit しない)。再生は docs の契約
    // (player:play = フレーム出力開始) を守るため、ここでは出さず onPlaying に委ねる。
    if (!playing && wasPlay) {
      pluginBus.emit('player:pause', { videoId, currentTime: currentLogicalTime() });
    }
  }
  function togglePlay() {
    if (!video) return;
    if (reattaching && restoreAfterReattach) {
      const next = !restoreAfterReattach.play;
      userPaused = !next;
      setReattachPlayIntent(next);
      return;
    }
    if (video.paused) {
      userPaused = false;
      void video.play().catch(() => undefined);
    } else {
      userPaused = true;
      video.pause();
    }
  }
  /** クランプ用に有効な duration を返す。`video.duration` が NaN/0 のうち
   *  (metadata 未ロード時) は呼び出し側で「巻き戻り」が起きないよう Infinity を返す。 */
  function effectiveDuration(): number {
    const vd = video?.duration ?? NaN;
    if (Number.isFinite(vd) && vd > 0) return vd;
    if (duration > 0) return duration;
    return Infinity;
  }

  function applyPendingSeek() {
    if (!video || pendingSeek == null) return;
    const t = pendingSeek;
    pendingSeek = null;
    seekTo(t);
  }

  // 再アタッチ中は <video>.currentTime が一時的に 0 になる。位置を参照する各所
  // (相対シーク / フレームステップ / AB 点 / スクショ / getCurrentTime) はこの
  // 論理位置を使う。保留中の seek / 未適用 resume (pendingSeek) を優先する。
  function currentLogicalTime(): number {
    // 再アタッチ中 (および終了失敗で復帰待ちの間) は currentTime state を論理位置の
    // 真値として使う。過渡的な 0 巻き戻り / 失敗要素の 0 は onTimeUpdate 抑制や退避値
    // (pendingSeek) で吸収され、未適用の pendingSeek があればそれ、無ければ
    // currentTime が論理位置になる。
    if (reattaching || reattachFailed) return pendingSeek ?? currentTime;
    return video?.currentTime ?? currentTime;
  }
  // 論理位置を currentTime state / onTime / player:time へ反映する。再アタッチ中の
  // 明示シーク (onTimeUpdate 抑制下) を消費側 (PiP handoff / 親 / プラグイン) に
  // 伝えるために使う。
  function publishLogicalTime(t: number) {
    currentTime = t;
    onTime?.(t);
    pluginBus.emit('player:time', { videoId, currentTime: t });
  }
  // 再アタッチ中に適用/予約したシーク位置を publish し、先頭(0)シークで seeked が
  // 出ないケースのコメント凍結も解除する。
  function onReattachSeek(pos: number) {
    publishLogicalTime(pos);
    if (pos === 0) {
      restoreSeeking = false;
      if (restoreSeekTimer) {
        clearTimeout(restoreSeekTimer);
        restoreSeekTimer = null;
      }
    }
  }
  function seekDelta(delta: number) {
    if (!video) return;
    seekTo(currentLogicalTime() + delta);
  }
  function seekTo(t: number) {
    if (!video) return;
    if (!Number.isFinite(t)) return;
    // metadata 未ロードだと currentTime 代入が無視 / 失敗する WebKit 挙動が
    // あるので、readyState>=1 (HAVE_METADATA) を待ってから適用する。
    if (video.readyState < 1) {
      // 既知の論理 duration があれば範囲内にクランプしてから退避/publish する
      // (範囲外値が resume として永続化され、次の Player に near-end 判定で弾かれて
      // 先頭へ戻るのを防ぐ)。
      let queued = Math.max(0, t);
      if (duration > 0) queued = Math.min(queued, duration - 0.05);
      pendingSeek = queued;
      // 再アタッチ中の明示シークは論理位置として publish する (PiP handoffTime /
      // onTime / player:time が古い位置のまま固定されないように)。先頭(0)凍結解除込み。
      if (reattaching) onReattachSeek(pendingSeek);
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
    // 再アタッチ中に直接適用した (readyState>=1) シークも論理位置として publish する。
    // 適用済みの 0 シークを detach 過渡リセットと取り違えないようにするため。
    if (reattaching) onReattachSeek(target);
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
    // ユーザ起源の変化のみ永続化する。`onVolumeChange` 経由で全代入を
    // 保存してしまうと、loadedmetadata の初期セットや PiP 引き継ぎの
    // ような内部操作で「ユーザが選んだ値」が上書きされてしまう。
    saveVolume(video.volume);
    saveMuted(video.muted);
  }
  function toggleMute() {
    if (!video) return;
    video.muted = !video.muted;
    saveVolume(video.volume);
    saveMuted(video.muted);
  }
  function setRate(r: number) {
    if (!video) return;
    if (reattaching && restoreAfterReattach) {
      restoreAfterReattach.rate = r;
      restoreAfterReattach.rateExplicit = true;
      playbackRate = r;
      return;
    }
    video.playbackRate = r;
  }
  function toggleComments() {
    commentsEnabled = !commentsEnabled;
  }
  function setQuality(levelIndex: number) {
    if (!hls || !video) return;
    // 再アタッチ中は新 hls の levels が未パースで空になる。UI が表示している
    // hlsLevels (前回パース結果を保持) で検証し、連打を取りこぼさない。
    const levels = hlsLevels;
    if (levelIndex < 0 || levelIndex >= levels.length) return;
    // 失敗した切替の再試行では同一画質の選択も通す (currentLevel は eager に進めて
    // いるため、失敗時に同じ画質を選び直せるようにする)。
    if (levelIndex === currentLevel && !reattachFailed) return;
    // 同じ解像度の別トラック (音声コーデック違い等) を選んだだけなら、
    // 映像コーデックは変わらないので再バッファせず内部状態だけ更新する。
    const curHeight = currentLevel >= 0 ? levels[currentLevel]?.height : undefined;
    const nextHeight = levels[levelIndex]?.height;
    // 終了失敗の再試行中は同解像度ショートカットも飛ばし、実際に再アタッチする。
    if (curHeight != null && curHeight === nextHeight && !reattachFailed) {
      userPickedLevel = levelIndex;
      currentLevel = levelIndex;
      return;
    }
    // WebKitGTK の MSE は、再生中のバッファを保持/フラッシュしたままの
    // mid-stream な解像度・コーデック切替 (hls.currentLevel / nextLevel) で
    // decoder が永久停止することがある。niconico Domand は画質ごとに H.264
    // profile も解像度も異なる (例: avc1.4d401e/320x240 ⇄ avc1.42c01e/480x360)
    // ため、これが「画質切り替えで止まる」の原因になっていた。
    // hls.js を作り直して選択レベル固定で attach し直し、SourceBuffer の
    // コーデック再構成を完全に回避する。再生位置と再生状態は復元する
    // (同じ <video> 要素を使うので音量/速度/ミュートは保持される)。
    userPickedLevel = levelIndex;
    desiredHeight = levels[levelIndex]?.height ?? null;
    currentLevel = levelIndex;
    // 既に再アタッチ進行中なら、最初の切替で退避したスナップショットと位置
    // (pendingSeek) を保持し、選択画質 (desiredHeight) だけ差し替える。これで画質を
    // 連打しても元の再生位置・再生意図を失わない。
    if (!reattaching) {
      reattaching = true;
      // 位置: 確立済み再生 (initialized) のみ現在位置を退避する (終了済みは先頭)。
      // 初回ロード前は resume を通常の onDurationChange (near-end 判定込み) に委ね、
      // ユーザシークがあれば既に pendingSeek が勝つので、ここでは触らない。
      // 位置を保持すべきか: 確立済み (initialized)、または既に位置が進んでいる
      // (初回 durationchange で resume 適用済み / ユーザシーク済みで currentTime>0)
      // なら現在位置を退避する。真の未開始 (位置 0) は resume を通常の
      // onDurationChange (near-end 判定込み) に委ねる。終了済みは先頭。
      // 直前の失敗切替 (cancelReattach) で退避した位置 (pendingSeek) が残っていれば
      // それを優先する (失敗要素は 0 にリセット済みで、上書きすると先頭再生になる)。
      const pos = pendingSeek ?? (video.ended ? 0 : video.currentTime);
      if (initialized || pos > 0) {
        pendingSeek = pos;
        // 復元シーク (0→保存位置) の完了までコメント overlay を凍結する。ここで
        // 立てるのが肝心 — pendingSeek は durationchange で loadedmetadata より先に
        // 消費されうるため、loadedmetadata 時点での推測では取りこぼす。onSeeked で
        // 解除し、念のためタイマー fallback も張る (位置 0 は seek 不要なので除外)。
        if (pendingSeek > 0) {
          restoreSeeking = true;
          if (restoreSeekTimer) clearTimeout(restoreSeekTimer);
          restoreSeekTimer = setTimeout(() => {
            restoreSeeking = false;
            restoreSeekTimer = null;
          }, 5000);
        }
      }
      // 再アタッチ後に再生すべきか: 再生中、または「未開始だが autoplay 予定で
      // ユーザが明示停止していない」なら true。再生意図と速度を退避し
      // loadedmetadata で復元する。これで再生開始前の切替でも autoplay 設定が
      // 尊重され、明示停止 (userPaused) も尊重され、再アタッチ中の明示操作も拾える。
      const autoplayIntent = getBool('playback.autoplay') || forceAutoplay;
      const shouldPlay = !video.paused || (!playbackStarted && autoplayIntent && !userPaused);
      // silentPlay: 既に「実際に再生開始済み」だった切替のみ継続扱いで player:play
      // を抑制する。再生要求済みだが初回 playing 前 (playbackStarted=false) は
      // まだ player:play を出していないので、新規再生として onPlaying で通知する。
      restoreAfterReattach = {
        play: shouldPlay,
        rate: video.playbackRate,
        silentPlay: playbackStarted && !video.paused,
      };
      // control bar の表示 (再生/一時停止ボタン) と復元意図を一致させる。これを
      // しないと pre-play autoplay 予定の切替で Play ボタンが出るのにクリックすると
      // togglePlay が intent を反転し、再生開始ではなく停止になってしまう。
      paused = !shouldPlay;
    } else if (initialized) {
      // 連打 (再アタッチ中の再切替): play/rate スナップショットは保持しつつ位置を
      // 取り直す。最初の reattach の durationchange で pendingSeek が消費済みでも、
      // frozen な論理位置から復元し、次の置換が 0 から始まるのを防ぐ。
      const pos = currentLogicalTime();
      if (Number.isFinite(pos) && pos > 0) pendingSeek = pos;
    }
    attachHls();
  }
  function setCommentOpacity(o: number) {
    commentOpacity = o;
  }
  function setAbIn() {
    if (!video) return;
    abLoop = { ...abLoop, in: currentLogicalTime() };
  }
  function setAbOut() {
    if (!video) return;
    abLoop = { ...abLoop, out: currentLogicalTime() };
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
    // フレームステップは「一時停止してコマ送り」。明示的な一時停止意図として記録し、
    // 直後の画質切替が autoplay 意図で再生を再開してしまうのを防ぐ。
    userPaused = true;
    const step = forward ? 1 / 30 : -1 / 30;
    if (reattaching && restoreAfterReattach) {
      // 再アタッチ中はフレームステップも論理状態へ反映する (停止扱いにして位置を
      // 進める)。seekTo 経由にして publishLogicalTime / 先頭凍結解除も共有する。
      setReattachPlayIntent(false);
      seekTo(currentLogicalTime() + step);
      return;
    }
    if (!video.paused) video.pause();
    video.currentTime += step;
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
      onEndedExternal?.();
      // プラグイン: 動画自然終了 (loop 中は除く)
      pluginBus.emit('player:ended', { videoId });
    }
  }

  function onTimeUpdate() {
    if (!video) return;
    // 画質切り替えの再アタッチ中は currentTime が一時的に 0 へ戻る。これを
    // onTime / player:time / currentTime 状態へ流すと偽の巻き戻りになり、親が
    // 保存する再生位置まで 0 で汚染されうるため抑制する。
    if (reattaching) return;
    // フレームが進んでる = 再生できてる → 待機中の一過性 error は無視
    if (pendingVideoErrorTimer && video.currentTime > 0) {
      clearPendingVideoError();
    }
    const now = performance.now();
    if (now - lastTimeUpdateTs < 200) return;
    lastTimeUpdateTs = now;
    currentTime = video.currentTime;
    onTime?.(video.currentTime);
    // プラグイン: 再生時刻更新 (既存 200ms スロットルに乗る)
    pluginBus.emit('player:time', { videoId, currentTime: video.currentTime });
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
    // 再アタッチ中は detach で duration が一時的に NaN→0 になり、シークバーが無効化
    // され plugin state にも 0 が漏れる。同一動画なので有効値のときだけ更新する。
    if (Number.isFinite(video.duration) && video.duration > 0) {
      duration = video.duration;
    } else if (!reattaching) {
      duration = 0;
    }
    // Restore saved position once duration is available
    if (!resumeApplied && resumePosition > 0 && duration > 0) {
      resumeApplied = true;
      if (resumePosition < duration - 1) {
        video.currentTime = resumePosition;
        // 再アタッチ中の resume 適用も論理位置として反映する (currentLogicalTime が
        // 真値の currentTime を返せるように)。
        if (reattaching) publishLogicalTime(resumePosition);
      }
    }
    // metadata 来たので保留中の seek 要求を消化
    applyPendingSeek();
  }
  // 既定の再生速度を設定から適用する。
  function applyDefaultRate() {
    if (!video) return;
    const defaultRate = getNum('playback.default_rate');
    if (Number.isFinite(defaultRate) && defaultRate > 0) {
      video.playbackRate = defaultRate;
    }
  }
  // 保存済み (なければ設定既定) の音量・ミュートを適用し、初回既定適用済み
  // フラグ (initialized) を立てる。
  function applyAudioDefaults() {
    if (!video) return;
    const savedVol = readSavedVolume();
    if (savedVol != null) {
      video.volume = savedVol;
    } else {
      const defaultVol = getNum('playback.default_volume');
      if (Number.isFinite(defaultVol)) {
        video.volume = Math.max(0, Math.min(1, defaultVol));
      }
    }
    if (readSavedMuted()) video.muted = true;
    initialized = true;
  }
  function onLoadedMetadata() {
    // 退避/新規いずれの seek 要求も pendingSeek 経由で適用する (どちらの handler
    // が先に走っても最新値が勝つ)。
    applyPendingSeek();
    if (!video) return;
    // 画質切り替えで hls.js を作り直した直後は、再生状態と速度を復元する
    // (再生位置は上の pendingSeek で復元済み)。<video> 要素は使い回しているので
    // 音量/ミュートは保持されるが、MediaSource 付け替えで playbackRate は
    // defaultPlaybackRate に戻るため明示的に復元する。通常の音量・オートプレイ
    // 既定ロジックは適用しない。
    if (restoreAfterReattach) {
      const { play, rate, rateExplicit, silentPlay } = restoreAfterReattach;
      restoreAfterReattach = null;
      // スナップショットを消費したので連打保護フラグ (reattaching) は解除する。
      reattaching = false;
      if (!initialized) {
        // 初回 loadedmetadata 前の早期画質切替: 通常 init をスキップするので、ここで
        // 既定の音量・ミュート・速度を適用する (full-volume/unmuted/既定速度のまま
        // 再生されるのを防ぐ)。ただしユーザが再アタッチ中に明示的に速度変更していた
        // 場合はそれを優先する。
        applyAudioDefaults();
        if (rateExplicit && Number.isFinite(rate) && rate > 0) {
          video.playbackRate = rate;
        } else {
          applyDefaultRate();
        }
      } else if (Number.isFinite(rate) && rate > 0) {
        video.playbackRate = rate;
      }
      // 復元後の paused 状態を同期する (再アタッチ中の明示 pause を UI/プラグインの
      // getState に反映するため)。
      paused = !play;
      if (play) {
        // 継続再生 (silentPlay) のみ onPlaying の player:play を抑制する。新規/明示
        // 再生は契約どおり onPlaying で player:play を出す (再生失敗時は何も出さない)。
        suppressPlayEventOnce = silentPlay === true;
        const inst = hls;
        void video.play().catch(() => {
          // 別の attach に置き換わっていたら (連打 / エラー再接続)、この古い reject は
          // 新しい状態を壊さないよう無視する。
          if (hls !== inst) return;
          // 再生失敗時: 抑制フラグを残さず、paused を実状態へ同期する。
          suppressPlayEventOnce = false;
          paused = video?.paused ?? true;
        });
      }
      return;
    }
    // 設定からデフォルト値を反映
    applyDefaultRate();
    if (initialMuted) {
      // PiP 引き継ぎロード: 音量 0 のままバックグラウンド再生開始。
      // ページ側 Player の音声を切らずに mini をウォームアップしている最中。
      // 親 (MiniPlayer) が playing 検知後に音量を戻して引き継ぐ。
      video.volume = 0;
      initialized = true;
      void video.play().catch(() => undefined);
    } else {
      // ユーザが直近で選んだ音量があればそれを優先。無ければ設定の既定値。
      // これで PiP 切替や別動画への遷移、ページ再マウントで音量が
      // 既定値にリセットされてしまう挙動を防ぐ。
      applyAudioDefaults();
      const autoplay = getBool('playback.autoplay');
      // forceAutoplay は連続再生キュー進行など「ユーザが明示的に継続再生
      // を選んだ」コンテキスト用。`playback.autoplay=false` でも再生開始する。
      if (autoplay || forceAutoplay) {
        void video.play().catch(() => undefined);
      }
    }
  }
  // initialMuted=true で起動した時、playing イベントが 1 回目に発火したタイミングで
  // 親に「音声を引き継いでよい」通知を投げる。`onplay` ではなく `onplaying` を使うの
  // は、後者が「実際にフレーム送出が始まった」セマンティクスで、バッファリング中の
  // 偽陽性が起きにくいため。
  let audioHandoffSignaled = false;
  function onPlaying() {
    if (initialMuted && !audioHandoffSignaled) {
      audioHandoffSignaled = true;
      onReadyForAudio?.();
    }
    playbackStarted = true;
    // 画質切り替えの復元再生による onPlaying は内部的な再開なので、player:play は
    // 出さずにフラグだけ下ろす。
    if (suppressPlayEventOnce) {
      suppressPlayEventOnce = false;
      return;
    }
    // プラグイン: 再生開始
    if (video) {
      pluginBus.emit('player:play', { videoId, currentTime: video.currentTime });
    }
  }
  function onPlayState() {
    if (!video) return;
    // 画質切り替えの再アタッチ中の pause/play は MediaSource 付け替えに伴う
    // 内部的なもの。UI・プラグイン状態を揺らさないよう paused 状態の更新も
    // イベント通知も行わない (スナップショット消費後に正しい状態へ復元する)。
    if (reattaching) {
      syncAudioPlayState();
      return;
    }
    paused = video.paused;
    if (video.paused) {
      showControls();
      // 復元再生の保留中に一時停止されたら抑制フラグを残さない (次の本物の
      // 再生開始イベントが誤って消されるのを防ぐ)。
      suppressPlayEventOnce = false;
      // プラグイン: 一時停止 (再生開始は onPlaying 側で emit する)。
      // 自然終了 (video.ended=true) のときは HTMLMediaElement 仕様で
      // pause が ended 直前に出るが、ここで emit すると plugin が
      // "ユーザ pause" と "自然終了" を区別できなくなる
      // (Codex #15: docs/plugins.md は ended と排他と明記)。
      if (!video.ended) {
        pluginBus.emit('player:pause', { videoId, currentTime: video.currentTime });
      }
    }
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
    // 復元シークが完了したらコメント凍結を解除する。
    if (restoreSeeking) {
      restoreSeeking = false;
      if (restoreSeekTimer) {
        clearTimeout(restoreSeekTimer);
        restoreSeekTimer = null;
      }
    }
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
    // プラグイン: player.command (Rust dispatcher 経由) を受け取ってプレイヤー
    // を操作する。compact (PiP) インスタンスでは ignore (重複操作を避ける)。
    // owner は host 固有。複数インスタンス mount 時には個別 owner で分離する。
    const PLAYER_BUS_OWNER = compact ? '__host_player_compact__' : '__host_player_main__';
    const offControl = pluginBus.on(PLAYER_BUS_OWNER, 'plugin:player:control', (payload) => {
      if (compact) return; // PiP は受け付けない (ページ側 Player が hosts)
      const p = payload as { kind?: string; value?: number | null } | null;
      if (!p || typeof p.kind !== 'string') return;
      try {
        switch (p.kind) {
          case 'play':
            play();
            return;
          case 'pause':
            pause();
            return;
          case 'toggle':
            togglePlay();
            return;
          case 'seek':
            if (typeof p.value === 'number' && Number.isFinite(p.value)) {
              seekTo(p.value);
            }
            return;
          case 'setRate':
            if (typeof p.value === 'number' && Number.isFinite(p.value)) {
              setRate(Math.max(0.25, Math.min(4, p.value)));
            }
            return;
          case 'setVolume':
            if (typeof p.value === 'number' && Number.isFinite(p.value)) {
              setVolume(Math.max(0, Math.min(1, p.value)));
            }
            return;
          case 'toggleMute':
            toggleMute();
            return;
        }
      } catch (e) {
        console.error('[plugin] player.command handler failed:', e);
      }
    });
    return () => {
      document.removeEventListener('webkitfullscreenchange', onFullscreenChange);
      offControl();
      // PiP は state を持たないので main のときだけ clear。
      if (!compact) clearPlayerState();
    };
  });

  // プラグインから観測可能な状態スナップショットを同期更新する。
  // ここは Svelte の $effect でリアクティブに反映 — `videoId`/再生時刻/音量/
  // 一時停止/速度/ミュートが変わったら都度書き込む。compact では state を
  // 上書きしない (main が真値; PiP は一時的なミラー)。
  $effect(() => {
    if (compact) return;
    updatePlayerState({
      videoId: videoId ?? null,
      currentTime,
      duration,
      paused,
      volume,
      muted,
      playbackRate,
    });
  });

  // ショートカット登録は $effect に分離し、pluginPlayerActions() の変化を
  // 追跡して再バインドする (Codex review r3297535044: プラグインホストが
  // 非同期に register するので onMount スナップショットだと key が永遠に
  // 効かないケースを救済)。compact (PiP) モードでは登録しない。
  $effect(() => {
    if (compact) return;
    // 依存源: pluginPlayerActions() の戻り値 (registry 変化に reactive)
    const pluginKeys: Record<string, () => void> = {};
    for (const a of pluginPlayerActions()) {
      if (a.key) pluginKeys[a.key] = () => void a.handler();
    }
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
      pluginKeys: Object.keys(pluginKeys).length > 0 ? pluginKeys : undefined,
    };
    const unbindShortcuts = bindShortcuts(window, actions);
    return () => {
      unbindShortcuts();
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
    userPaused = false;
    if (reattaching && restoreAfterReattach) {
      setReattachPlayIntent(true);
      return;
    }
    void video.play().catch(() => undefined);
  }
  export function pause() {
    if (!video) return;
    userPaused = true;
    if (reattaching && restoreAfterReattach) {
      setReattachPlayIntent(false);
      return;
    }
    video.pause();
  }
  export function isPaused(): boolean {
    // 再アタッチ中は論理状態 (paused) を返す。通常時は play()/pause() が同期的に
    // 更新する video.paused を返す — onplay/onpause での paused 反映は遅れるため、
    // コマンド直後の PiP 引き継ぎが逆の wasPlaying を拾うのを防ぐ。
    if (reattaching) return paused;
    return video?.paused ?? paused;
  }
  export function getCurrentTime(): number {
    return currentLogicalTime();
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
  class:short
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
    crossorigin="anonymous"
    style:visibility={isSeeking ? 'hidden' : 'visible'}
    onplay={onPlayState}
    onpause={onPlayState}
    onplaying={onPlaying}
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
        cancelReattach();
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
        // 3 秒猶予後も復帰せず terminal となったので再アタッチ窓も畳む
        // (SRC_NOT_SUPPORTED 即時分岐と同様、スナップショットの死蔵を防ぐ)。
        cancelReattach();
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
    <CommentLayer
      {video}
      {comments}
      enabled={commentsEnabled}
      opacity={commentOpacity}
      freeze={reattaching || restoreSeeking}
    />
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
        onScreenshot={takeScreenshot}
        onToggleLoop={() => {
          const next = !loop;
          onLoopChange?.(next);
        }}
        onFullscreen={toggleFullscreen}
        onQuality={setQuality}
        onTogglePip={() => onTogglePip?.()}
        pluginActions={pluginPlayerActions()}
      />
    </div>
  {/if}
  {#if screenshotMsg}
    <div class="screenshot-toast">{screenshotMsg}</div>
  {/if}
</div>

<style>
  .player {
    position: relative;
    background: var(--theme-bg);
    border-radius: 8px;
    overflow: hidden;
    outline: none;
  }

  .seek-mask {
    position: absolute;
    inset: 0;
    background: var(--theme-bg);
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
    background: var(--theme-bg);
  }

  .player.short {
    max-width: calc(80vh * 9 / 16);
    width: calc(80vh * 9 / 16);
    margin: 0 auto;
  }

  @media (max-width: 480px) {
    .player.short {
      width: 100%;
      max-width: 100%;
    }
  }

  .player.short :global(video) {
    aspect-ratio: 9 / 16;
    max-height: 80vh;
    width: 100%;
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
    /* 映像の上に重ねるバナーなので、テーマに関係なく暗いオーバレイ +
       白文字で固定する (classic 時に --theme-text が暗茶になり、
       grey-78% 背景上で潰れる問題を防ぐ)。 */
    background: var(--theme-overlay-strong);
    color: var(--theme-on-overlay);
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
  :global(html[data-theme='niconico-classic']) .player {
    background: #ffffff;
    border-radius: 0;
  }
  /* 横長 (通常) 動画のみ 16:9 を維持。.player.short (縦長/ショート動画)
     は別途 9:16 ルール (.player.short :global(video)) があるので、ここで
     上書きしないよう :not(.short) に限定する。これを怠ると classic 時に
     縦動画も 16:9 で描かれ、画面の左右が大きく letterbox になる
     (codex review r3293692948)。 */
  :global(html[data-theme='niconico-classic']) .player:not(.short) :global(video) {
    /* metadata 未ロード時に <video> が高さ 0 に潰れて初期表示が空白に
       なる (旧コードの aspect-ratio: auto + height: auto バグ) のを防ぐ
       ため 16:9 を維持する。 */
    aspect-ratio: 16 / 9;
    max-height: min(calc(100vh - 320px), 80vh);
    background: #000000;
  }
  /* 縦長 (ショート) 動画は classic でも背景を黒にだけ揃える。
     aspect-ratio は .player.short の既存ルール (9/16) を尊重。 */
  :global(html[data-theme='niconico-classic']) .player.short :global(video) {
    background: #000000;
  }
  :global(html[data-theme='niconico-classic']) .controls-wrap {
    position: static;
    opacity: 1;
    pointer-events: auto;
    /* opacity transition は dark の auto-hide 用。classic は常時表示
       なので、controlsVisible が一過性に false に振れた瞬間の意図しない
       フェードを抑止する。 */
    transition: none;
  }
  :global(html[data-theme='niconico-classic']) .player.fullscreen :global(video) {
    width: 100%;
    height: 100%;
    max-height: none;
    aspect-ratio: auto;
  }
  :global(html[data-theme='niconico-classic']) .player.fullscreen .controls-wrap {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
  }

  .fatal-error {
    background: var(--theme-danger-bg);
    border: 1px solid var(--theme-danger-border);
    color: var(--theme-danger-text);
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
    border: 1px solid var(--theme-accent-border);
    color: var(--theme-accent-soft);
    padding: 8px 10px;
    border-radius: 4px;
    font-size: 12px;
    line-height: 1.6;
  }
  .screenshot-toast {
    position: absolute;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    /* 映像の上に重ねるトーストなのでテーマに関係なく暗オーバレイ +
       白系文字で視認性を担保 (classic の --theme-success-text=#355f2e
       が暗グレ背景に紛れる問題への対処)。 */
    background: var(--theme-overlay-strong);
    color: var(--theme-on-overlay);
    padding: 6px 14px;
    border-radius: 6px;
    font-size: 13px;
    pointer-events: none;
    z-index: 30;
  }
</style>
