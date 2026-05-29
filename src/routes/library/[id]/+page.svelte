<script lang="ts">
  import { onDestroy } from 'svelte';
  import { beforeNavigate, goto } from '$app/navigation';
  import { page } from '$app/state';
  import Player from '$lib/player/Player.svelte';
  import CommentList from '$lib/player/CommentList.svelte';
  import QueueBanner from '$lib/QueueBanner.svelte';
  import {
    deleteLibraryVideo,
    dtoToPlayerComment,
    localAudioUrl,
    localVideoUrl,
    prepareLocalPlayback,
    remuxLocalVideo,
    type LocalPlaybackPayload,
    listCommentSnapshots,
    loadSnapshotComments,
    deleteCommentSnapshot,
    updateSnapshotNote,
    refetchVideoComments,
    exportVideoWithComments,
    listenBurnInProgress,
    type CommentSnapshotRow,
  } from '$lib/api';
  import { open as shellOpen } from '@tauri-apps/plugin-shell';
  import { formatDate, formatDuration, formatNumber, videoUrl } from '$lib/format';
  import type { PlayerComment } from '$lib/player/types';
  import { filterComments, listNgRules, subscribeNgRules, type NgRule } from '$lib/stores/ngRules';
  import { addHistory } from '$lib/stores/history';
  import { getBool, getStr, loadSettings } from '$lib/stores/settings.svelte';
  import { sanitizeDescriptionHtml } from '$lib/sanitize';
  import { miniPlayer } from '$lib/player/miniPlayerStore.svelte';
  import {
    advanceQueue,
    getQueue,
    hasNextInQueue,
    itemHref,
    setQueueIndexByVideoId,
    subscribeQueue,
  } from '$lib/stores/playbackQueue';

  let local = $state<LocalPlaybackPayload | null>(null);
  let localSrc = $state<string | null>(null);
  let localAudioSrc = $state<string | null>(null);
  let pending = $state(true);
  let error = $state<string | null>(null);
  let currentTime = $state(0);
  let comments = $state<PlayerComment[]>([]);
  // コメ取得が「決着」したか (成功/失敗/取得不要のいずれかで確定)。
  // load() リセット直後の一過性 comments=[] で PiP のコメ層を潰さないよう、
  // mini への updateComments はこのフラグが true になってからのみ走らせる。
  let commentsSettled = $state(false);

  let ngRules = $state<NgRule[]>(listNgRules());
  const ngUnsub = subscribeNgRules(() => (ngRules = listNgRules()));

  let visibleComments = $derived(filterComments(ngRules, comments));
  let ngFilteredCount = $derived(comments.length - visibleComments.length);

  // コメントスナップショット管理
  let snapshots = $state<CommentSnapshotRow[]>([]);
  let activeSnapshotId = $state<number | null>(null);
  let snapshotLoading = $state(false);
  let snapshotMessage = $state<string | null>(null);
  let editingNoteId = $state<number | null>(null);
  let editingNoteText = $state('');
  let playerKey = $state(0);
  // スナップショット切替時の再生位置復元用（テンプレート内で消費するので $state 禁止）
  let _pendingResumeTime: number | null = null;

  // 動画は内蔵 HTTP サーバ (http://127.0.0.1:port/v/{id}/...) 経由で配信する。
  // Blob URL は WebKitGTK + GStreamer の組合せだと後方 seek でガビガビになる。

  type PlayerRef = { seek: (t: number) => void; getVideo: () => HTMLVideoElement | null };
  let playerRef = $state<PlayerRef | undefined>();
  let videoId = $derived(page.params.id ?? '');
  let theme = $derived(getStr('appearance.theme'));
  let isClassicTheme = $derived(theme === 'niconico-classic');
  let loadingFor: string | null = null;
  let loop = $state(false);
  // ユーザが Player の loop ボタンを明示的に操作したかを記録する。
  // true の間は後段の自動再計算 (キュー変更によるリセットなど) を抑止する。
  let loopUserSet = $state(false);

  function computeDefaultLoop(id: string): boolean {
    if (!getBool('playback.always_loop')) return false;
    const fromQueue = page.url.searchParams.get('from') === 'queue';
    return !(fromQueue && hasNextInQueue(id));
  }

  // 「ユーザが今キュー再生中である」確証がある時だけ forceAutoplay。
  // bookmark / shared URL で `?from=queue` 残骸付きでも、キューに乗って
  // いない動画では `playback.autoplay=false` 設定を尊重する (codex
  // r3283322762)。
  function shouldForceAutoplay(id: string): boolean {
    if (page.url.searchParams.get('from') !== 'queue') return false;
    const q = getQueue();
    if (!q) return false;
    return q.items[q.index]?.videoId === id;
  }

  // キュー停止/進行で loop の既定値が変わるケースに追従する (codex review)。
  const unsubQueueLoop = subscribeQueue(() => {
    if (loopUserSet) return;
    const id = local?.videoId ?? videoId;
    if (!id) return;
    loop = computeDefaultLoop(id);
  });

  let panelWidth = $state(320);
  let dragging = $state(false);
  let dragStartX = 0;
  let dragStartWidth = 0;

  let backHref = $state('/library');
  let backLabel = $state('← ライブラリに戻る');

  $effect(() => {
    const from = page.url.searchParams.get('from');
    if (from === 'history') {
      backHref = '/history';
      backLabel = '← 履歴に戻る';
    } else if (from === 'queue') {
      const q = getQueue();
      if (q) {
        if (q.context === 'mylist') {
          backHref = `/playlists?mylistId=${encodeURIComponent(q.contextId)}`;
          backLabel = `← マイリスト「${q.label}」に戻る`;
        } else if (q.context === 'smart') {
          backHref = `/playlists/smart/${q.contextId}`;
          backLabel = `← スマートプレイリスト「${q.label}」に戻る`;
        } else if (q.context === 'series') {
          backHref = `/series/${q.contextId}`;
          backLabel = `← シリーズ「${q.label}」に戻る`;
        } else {
          backHref = '/library';
          backLabel = '← ライブラリに戻る';
        }
      } else {
        backHref = '/library';
        backLabel = '← ライブラリに戻る';
      }
    } else {
      backHref = '/library';
      backLabel = '← ライブラリに戻る';
    }
  });

  function tagSearchHref(tag: string): string {
    return `/search?q=${encodeURIComponent(tag)}&targets=tagsExact`;
  }

  async function load(id: string) {
    if (!id) return;
    loadingFor = id;
    pending = true;
    error = null;
    local = null;
    localSrc = null;
    localAudioSrc = null;
    comments = [];
    commentsSettled = false;

    try {
      // 設定と再生情報を並列取得
      const [, result] = await Promise.all([loadSettings(), prepareLocalPlayback(id)]);
      loop = computeDefaultLoop(id);
      loopUserSet = false;
      if (loadingFor !== id) return;
      if (!result) {
        error = `${id} はライブラリに無い、または video.mp4 が見つかりません。`;
        pending = false;
        return;
      }
      local = result;
      // スナップショット一覧も並行取得
      await loadSnapshots(id);
      // 使用中のスナップショットIDを記録。最初は最新を選択。
      if (snapshots.length > 0) {
        activeSnapshotId = snapshots[0].id;
      }
      // 内蔵 HTTP サーバの URL を取る。Range 対応なので後方 seek が clean。
      try {
        localSrc = await localVideoUrl(id);
        if (result.localAudioPath) {
          localAudioSrc = await localAudioUrl(id);
        }
      } catch (e) {
        error = `ローカル URL 解決失敗: ${e}`;
        pending = false;
        return;
      }
      if (loadingFor !== id) return;
      comments = result.comments.map(dtoToPlayerComment);
      commentsSettled = true;
      addHistory({
        videoId: result.videoId,
        title: result.title,
        thumbnailUrl: result.thumbnailUrl ?? undefined,
        uploaderName: result.uploaderName ?? undefined,
        duration: result.durationSec,
        viewCount: result.viewCount ?? undefined,
        source: 'local',
      });
      pending = false;
    } catch (e) {
      if (loadingFor !== id) return;
      error = String(e);
      pending = false;
    }
  }

  $effect(() => {
    void load(videoId);
  });

  $effect(() => {
    if (videoId) setQueueIndexByVideoId(videoId);
  });

  function handleSeek(t: number) {
    playerRef?.seek(t);
  }

  function handleEnded() {
    if (!getBool('playback.autoplay_queue')) return;
    const q = getQueue();
    if (!q) return;
    const idx = q.items.findIndex((it) => it.videoId === (local?.videoId ?? videoId));
    if (idx < 0) return;
    const nxt = q.items[idx + 1];
    if (!nxt) return;
    advanceQueue();
    void goto(itemHref(nxt));
  }

  function getResumePosition(id: string): number {
    if (_pendingResumeTime != null) {
      const t = _pendingResumeTime;
      _pendingResumeTime = null;
      return t;
    }
    const pipPos = miniPlayer.consumeReturnPosition(id);
    if (pipPos > 0) return pipPos;
    if (!getBool('playback.resume_enabled')) return 0;
    try {
      return Number(localStorage.getItem(`resume:${id}`)) || 0;
    } catch {
      return 0;
    }
  }
  function saveResumePosition(id: string, t: number) {
    try {
      localStorage.setItem(`resume:${id}`, String(Math.floor(t)));
    } catch {
      /* */
    }
  }

  function handleTimeUpdate(time: number) {
    currentTime = time;
    if (local && time > 0) {
      saveResumePosition(local.videoId, time);
    }
    // PiP 音声引き継ぎ中はページ側 currentTime をストアに書く。
    // mini が引き継ぎ瞬間にここへシークすることで音声の巻き戻しを防ぐ。
    if (
      local &&
      miniPlayer.active &&
      !miniPlayer.audioOwned &&
      miniPlayer.source?.videoId === local.videoId
    ) {
      miniPlayer.setHandoffTime(time);
    }
  }

  function startDrag(e: MouseEvent) {
    e.preventDefault();
    dragging = true;
    dragStartX = e.clientX;
    dragStartWidth = panelWidth;
  }
  function onMove(e: MouseEvent) {
    if (!dragging) return;
    const delta = dragStartX - e.clientX;
    panelWidth = Math.max(200, Math.min(600, dragStartWidth + delta));
  }
  function stopDrag() {
    dragging = false;
  }

  let remuxing = $state(false);
  let remuxMessage = $state<string | null>(null);
  async function onRemux(id: string) {
    remuxing = true;
    remuxMessage = null;
    try {
      const msg = await remuxLocalVideo(id);
      remuxMessage = msg + ' — リロードします';
      await load(id);
    } catch (e) {
      remuxMessage = `失敗: ${e}`;
    } finally {
      remuxing = false;
    }
  }

  function openPipForCurrentVideo(): boolean {
    if (!local || !localSrc) return false;
    // 同じ動画で既に PiP 起動済み (音声引き継ぎ中も含む) なら何もしない。
    if (miniPlayer.active && miniPlayer.source?.videoId === local.videoId) return false;
    // 別動画が PiP 稼働中ならこのページの Player はマウントされていない。
    // 再生中でない動画を PiP 化することは無いので、現在の PiP をそのまま維持する。
    if (
      miniPlayer.active &&
      !!miniPlayer.source?.videoId &&
      miniPlayer.source.videoId !== local.videoId
    ) {
      return false;
    }
    const vid = playerRef?.getVideo();
    const t = vid?.currentTime ?? currentTime ?? 0;
    // 起動時点で再生中だった場合のみ mini の無音ロード→引き継ぎフローを使う。
    const wasPlaying = vid != null && !vid.paused && !vid.ended;
    // パラ遷移で local が書き換わっても影響を受けないようスナップ。
    const snapVideoId = local.videoId;
    const snapTitle = local.title;
    const snapSrc = localSrc;
    const snapAudio = localAudioSrc ?? undefined;
    const snapHref = page.url.pathname + (page.url.search ?? '');
    if (snapVideoId) {
      try {
        localStorage.setItem(`resume:${snapVideoId}`, String(Math.floor(t)));
      } catch {
        /* ignore */
      }
    }
    miniPlayer.open({
      source: {
        kind: 'local',
        videoId: snapVideoId,
        localSrc: snapSrc,
        localAudioSrc: snapAudio,
      },
      title: snapTitle,
      comments: visibleComments,
      rawComments: comments,
      resumePosition: t,
      expandHref: snapHref,
      loop,
      wasPlaying,
    });
    return true;
  }

  function togglePip() {
    // 音声引き継ぎ中も同じ動画なら「PiP 化済み」扱いで閉じる。
    if (miniPlayer.active && miniPlayer.source?.videoId === (local?.videoId ?? '')) {
      miniPlayer.close();
      return;
    }
    openPipForCurrentVideo();
  }

  // 音声引き継ぎが完了するまでプレースホルダへ切り替えない (音切れ防止)。
  let pipActiveForThis = $derived(
    miniPlayer.active &&
      miniPlayer.audioOwned &&
      miniPlayer.source?.videoId === (local?.videoId ?? ''),
  );

  // グローバル単一アクティブ Player 不変条件: PiP が別動画で稼働中なら、ここでも
  // Player を絶対にマウントしない。audioOwned に関係なく排他する (引き継ぎ中も同様)。
  // これが無いと PiP (動画 A) + ページ Player (動画 B) で二重再生になる。
  let pipActiveForOther = $derived(
    miniPlayer.active &&
      !!miniPlayer.source?.videoId &&
      miniPlayer.source.videoId !== (local?.videoId ?? ''),
  );
  let pipExpandHref = $derived(miniPlayer.expandHref || '/');
  let pipOtherTitle = $derived(miniPlayer.title || 'ミニプレイヤー');
  // PiP 中はミニ側で取得済みコメの方が新しい可能性があるので、ミニ側にも反映。
  // ただし load() 直後の comments=[] (ローディング中の一過性空配列) で
  // mini を上書きすると PiP のコメ層が destroy されてしまうので、コメ取得が
  // 決着 (commentsSettled=true) してからのみ更新する。NG ルールで全件除外
  // された結果の [] のような「正当な空」は commentsSettled 後に発生するので
  // このガードを通って mini へ伝播する。
  $effect(() => {
    if (pipActiveForThis && local && commentsSettled) {
      miniPlayer.updateComments(local.videoId, visibleComments, comments);
    }
  });

  // 音声引き継ぎ中、ソース側 Player の paused 状態をストアへ反映する。
  // 引き継ぎ完了前にユーザが停止した意図を mini へ引き継ぐため。
  $effect(() => {
    if (!local) return;
    if (!miniPlayer.active) return;
    if (miniPlayer.audioOwned) return;
    if (miniPlayer.source?.videoId !== local.videoId) return;
    const id = setInterval(() => {
      const v = playerRef?.getVideo();
      if (v) miniPlayer.setSourcePaused(v.paused || v.ended);
    }, 200);
    return () => clearInterval(id);
  });

  beforeNavigate((nav) => {
    if (!getBool('pip.auto_navigate')) return;
    const toPath = nav.to?.url.pathname;
    const fromPath = nav.from?.url.pathname;
    if (!toPath || toPath === fromPath) return;
    if (/^\/video\//.test(toPath) || /^\/library\//.test(toPath)) return;
    openPipForCurrentVideo();
  });

  async function onDelete(id: string) {
    if (!confirm('ライブラリから完全削除しますか？')) return;
    try {
      await deleteLibraryVideo(id);
      window.location.href = '/library';
    } catch (e) {
      error = `削除失敗: ${e}`;
    }
  }

  async function loadSnapshots(vid: string) {
    try {
      snapshots = await listCommentSnapshots(vid);
    } catch {
      snapshots = [];
    }
  }

  async function switchSnapshot(snapId: number) {
    if (snapshotLoading) return;
    snapshotLoading = true;
    snapshotMessage = null;
    try {
      const cs = await loadSnapshotComments(snapId);
      const newComments = cs.map(dtoToPlayerComment);
      // 現在の再生位置を保存
      const vid = playerRef?.getVideo();
      const t = vid ? vid.currentTime : currentTime;
      _pendingResumeTime = t;
      // 全 state を同期的に更新（Svelte が一括で処理 → Player を即再マウント）
      comments = newComments;
      commentsSettled = true;
      activeSnapshotId = snapId;
      playerKey++;
      // スナップショット一覧は後で更新（表示のみ、ブロッキング不要）
      void loadSnapshots(videoId);
    } catch (e) {
      snapshotMessage = `コメント読込失敗: ${e}`;
      void loadSnapshots(videoId);
    } finally {
      snapshotLoading = false;
    }
  }

  async function onRefetch(vid: string) {
    if (!confirm('niconico から最新のコメントを再取得しますか？')) return;
    snapshotLoading = true;
    snapshotMessage = null;
    try {
      const newId = await refetchVideoComments(vid);
      await loadSnapshots(vid);
      snapshotMessage = `再取得完了 (新スナップショット #${newId})`;
      activeSnapshotId = newId;
    } catch (e) {
      snapshotMessage = `再取得失敗: ${e}`;
    } finally {
      snapshotLoading = false;
    }
  }

  async function onDeleteSnapshot(snapId: number) {
    if (!confirm('このスナップショットを削除しますか？')) return;
    try {
      await deleteCommentSnapshot(snapId);
      await loadSnapshots(videoId);
      if (activeSnapshotId === snapId) {
        activeSnapshotId = null;
        // 削除後、必要なら最新に戻す
        if (snapshots.length > 0) {
          await switchSnapshot(snapshots[0].id);
        } else {
          comments = [];
          commentsSettled = true;
        }
      }
    } catch (e) {
      snapshotMessage = `削除失敗: ${e}`;
    }
  }

  function startEditNote(snap: CommentSnapshotRow) {
    editingNoteId = snap.id;
    editingNoteText = snap.note ?? '';
  }

  function cancelEditNote() {
    editingNoteId = null;
    editingNoteText = '';
  }

  async function saveNote(snapId: number) {
    try {
      await updateSnapshotNote(snapId, editingNoteText || null);
      await loadSnapshots(videoId);
      editingNoteId = null;
      editingNoteText = '';
    } catch (e) {
      snapshotMessage = `ノート保存失敗: ${e}`;
    }
  }

  // ---- コメント焼き込みエクスポート (Phase 1.9) ----
  let burnInOpen = $state(false);
  let burnInRunning = $state(false);
  let burnInPercent = $state(0);
  let burnInMessage = $state<string | null>(null);
  let burnInResultPath = $state<string | null>(null);
  let binFontScale = $state(1.0);
  let binOpacity = $state(1.0);
  let burnInUnlisten: (() => void) | null = null;

  async function onBurnIn(vid: string) {
    if (burnInRunning) return;
    if (activeSnapshotId == null) {
      burnInMessage = '焼き込むスナップショットを選択してください';
      return;
    }
    burnInRunning = true;
    burnInPercent = 0;
    burnInMessage = null;
    burnInResultPath = null;
    burnInUnlisten?.();
    burnInUnlisten = await listenBurnInProgress((p) => {
      if (p.videoId === vid) burnInPercent = p.percent;
    });
    try {
      const res = await exportVideoWithComments(vid, {
        snapshotId: activeSnapshotId,
        fontScale: binFontScale,
        opacity: binOpacity,
      });
      burnInPercent = 1;
      burnInResultPath = res.outputPath;
      burnInMessage = `完了: ${res.commentCount} 件を ${res.width}×${res.height} で焼き込みました`;
    } catch (e) {
      burnInMessage = `焼き込み失敗: ${e}`;
    } finally {
      burnInRunning = false;
      burnInUnlisten?.();
      burnInUnlisten = null;
    }
  }

  async function revealExport() {
    if (!burnInResultPath) return;
    try {
      // 出力ファイルの親フォルダを既定のファイラで開く。
      const dir = burnInResultPath.replace(/[\\/][^\\/]*$/, '');
      await shellOpen(dir);
    } catch (e) {
      burnInMessage = `フォルダを開けません: ${e}`;
    }
  }

  onDestroy(() => {
    ngUnsub();
    unsubQueueLoop();
    burnInUnlisten?.();
  });
</script>

<svelte:window onmousemove={onMove} onmouseup={stopDrag} />

<section class="page" class:classic={isClassicTheme}>
  <div class="head">
    <a class="back" href={backHref}>{backLabel}</a>
    <h2>{local?.title ?? videoId}</h2>
    {#if local}
      <span class="local-badge">ローカル再生</span>
      <button
        type="button"
        class="ghost-btn"
        title="WebKit 互換 MP4 へ ffmpeg で作り直す"
        disabled={remuxing}
        onclick={() => onRemux(local!.videoId)}>{remuxing ? 'remux 中…' : '再 mux'}</button
      >
      <button
        type="button"
        class="danger-btn"
        title="ライブラリから完全削除"
        onclick={() => onDelete(local!.videoId)}>削除</button
      >
    {/if}
  </div>

  <QueueBanner videoId={local?.videoId ?? videoId} />

  {#if remuxMessage}
    <div class="info">{remuxMessage}</div>
  {/if}

  {#if pending}
    <div class="muted">読み込み中…</div>
  {:else if error}
    <div class="error">{error}</div>
    <p class="muted">
      オンラインで見るなら <a href={`/video/${videoId}`}>/video/{videoId}</a> へ。
    </p>
  {:else if local && localSrc}
    {@const lp = local}
    {@const ls = localSrc}
    {@const las = localAudioSrc}

    <div class="local-banner">
      <span class="local-marker" aria-hidden="true">LOCAL</span>
      <div class="local-banner-text">
        <strong>ローカル再生中</strong>
        <span class="local-banner-sub">
          ネット接続不要 / コメントは DL 時点のスナップショット
          {#if las}<span class="dot">·</span>映像 + 音声 別ファイル同期再生{/if}
        </span>
      </div>
      <a class="local-banner-online" href={`/video/${lp.videoId}`} title="オンラインで開く">
        オンラインで見る ↗
      </a>
    </div>

    <div class="player-frame">
      <div class="viewer-toolbar">
        <div class="viewer-toolbar-meta">
          <span class="toolbar-id">{lp.videoId}</span>
          <span class="toolbar-sep">|</span>
          <span>{formatDuration(lp.durationSec)}</span>
          {#if lp.postedAt}
            <span class="toolbar-sep">|</span>
            <span>{formatDate(new Date(lp.postedAt * 1000).toISOString())}</span>
          {/if}
        </div>
        <div class="viewer-toolbar-actions">
          <a class="toolbar-link" href={`/video/${lp.videoId}`}>ニコニコで再生</a>
        </div>
      </div>
      <div class="player-row" class:dragging>
        <div class="player-col">
          {#if pipActiveForThis}
            <div class="pip-placeholder">
              <div class="pip-thumb">
                {#if lp.thumbnailUrl}
                  <img src={lp.thumbnailUrl} alt="" />
                {/if}
                <div class="pip-overlay">
                  <div class="pip-icon" aria-hidden="true">
                    <svg viewBox="0 0 24 24" width="44" height="44">
                      <path d="M3 5h18v14H3V5zm2 2v10h14V7H5zm7 4h6v4h-6v-4z" fill="currentColor" />
                    </svg>
                  </div>
                  <div class="pip-text">ミニプレイヤーで再生中</div>
                  <button type="button" class="pip-resume" onclick={() => miniPlayer.close()}>
                    ここで再生に戻す
                  </button>
                </div>
              </div>
            </div>
          {:else if pipActiveForOther}
            <div class="pip-placeholder">
              <div class="pip-thumb">
                {#if lp.thumbnailUrl}
                  <img src={lp.thumbnailUrl} alt="" />
                {/if}
                <div class="pip-overlay">
                  <div class="pip-icon" aria-hidden="true">
                    <svg viewBox="0 0 24 24" width="44" height="44">
                      <path d="M3 5h18v14H3V5zm2 2v10h14V7H5zm7 4h6v4h-6v-4z" fill="currentColor" />
                    </svg>
                  </div>
                  <div class="pip-text">別の動画がミニプレイヤーで再生中</div>
                  <div class="pip-other-title">{pipOtherTitle}</div>
                  <div class="pip-actions">
                    <button
                      type="button"
                      class="pip-resume"
                      onclick={() => miniPlayer.close()}
                      title="PiP を閉じてこのページの動画を再生"
                    >
                      PiP を閉じてここで再生
                    </button>
                    <a
                      class="pip-link"
                      href={pipExpandHref}
                      title="ミニプレイヤーで再生中の動画ページへ"
                    >
                      PiP の動画を開く
                    </a>
                  </div>
                </div>
              </div>
            </div>
          {:else}
            {#key playerKey}
              <Player
                bind:this={playerRef}
                hlsUrl=""
                localSrc={ls}
                localAudioSrc={las ?? undefined}
                comments={visibleComments}
                videoTitle={lp.title}
                videoId={lp.videoId}
                onTime={handleTimeUpdate}
                onEnded={handleEnded}
                resumePosition={getResumePosition(lp.videoId)}
                {loop}
                onLoopChange={(v) => {
                  loop = v;
                  loopUserSet = true;
                }}
                onTogglePip={togglePip}
                pipActive={false}
                forceAutoplay={shouldForceAutoplay(lp.videoId)}
                short={lp.isShort}
              />
            {/key}
          {/if}
          {#if ngFilteredCount > 0}
            <div class="ng-banner">NG: {ngFilteredCount} 件のコメを除外中</div>
          {/if}
        </div>
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <div
          class="divider"
          role="separator"
          aria-label="コメントパネル幅調整"
          onmousedown={startDrag}
        ></div>
        <div class="comment-panel" style:width="{panelWidth}px" style:min-width="{panelWidth}px">
          {#if isClassicTheme}
            <div class="side-header">
              <span>{formatNumber(visibleComments.length)} 件</span>
              <label class="side-toggle">
                <input type="checkbox" checked={loop} onchange={() => (loop = !loop)} />
                <span>連続再生</span>
              </label>
            </div>
          {/if}
          <CommentList comments={visibleComments} {currentTime} onSeek={handleSeek} />
        </div>
      </div>
    </div>

    <div class="below">
      <div class="meta">
        <div class="row">
          <span>{lp.videoId}</span>
          <span class="dot">·</span>
          <span>{formatDuration(lp.durationSec)}</span>
          {#if lp.postedAt}
            <span class="dot">·</span>
            <span>{formatDate(new Date(lp.postedAt * 1000).toISOString())}</span>
          {/if}
          <span class="dot">·</span>
          <span>コメ {formatNumber(comments.length)}</span>
          <a class="external" href={videoUrl(lp.videoId)} target="_blank" rel="noreferrer noopener"
            >ニコニコで開く ↗</a
          >
        </div>
        {#if lp.uploaderName}
          <div class="row owner">
            {#if lp.uploaderId}
              <a
                href={`/user/${lp.uploaderId}?kind=${lp.uploaderType ?? 'user'}&name=${encodeURIComponent(lp.uploaderName)}`}
                class="owner-link"
              >
                <span>{lp.uploaderName}</span>
              </a>
            {:else}
              <span>{lp.uploaderName}</span>
            {/if}
            {#if lp.uploaderType}<span class="muted">({lp.uploaderType})</span>{/if}
          </div>
        {/if}
        {#if lp.tags.length > 0}
          <div class="tags" aria-label="タグ">
            {#each lp.tags as tag (tag.name)}
              <a
                class="tag"
                class:locked={tag.isLocked}
                href={tagSearchHref(tag.name)}
                title="このタグで検索"
              >
                {#if tag.isLocked}<span class="lock" aria-hidden="true">🔒</span>{/if}
                {tag.name}
              </a>
            {/each}
          </div>
        {/if}
        {#if lp.description}
          <details>
            <summary>説明文</summary>
            <!-- 説明文の HTML はサニタイズ済みのものだけを `{@html}` に渡す。
                 詳細は src/lib/sanitize.ts のコメントを参照。 -->
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            <p class="desc">{@html sanitizeDescriptionHtml(lp.description)}</p>
          </details>
        {/if}
      </div>

      <!-- コメントスナップショット管理 -->
      <div class="snapshot-section">
        <h3>コメントスナップショット</h3>
        {#if snapshotMessage}
          <div class="snap-msg">{snapshotMessage}</div>
        {/if}
        {#if snapshots.length === 0}
          <div class="muted">スナップショットがありません</div>
        {:else}
          <table class="snap-table">
            <thead>
              <tr>
                <th>取得日時</th>
                <th>コメント数</th>
                <th>ノート</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {#each snapshots as snap (snap.id)}
                <tr class:active={activeSnapshotId === snap.id}>
                  <td>
                    {formatDate(new Date(snap.takenAt * 1000).toISOString())}
                    {#if snap.isInitial}<span class="snap-badge">初期</span>{/if}
                  </td>
                  <td>{snap.commentCount}</td>
                  <td class="note-cell">
                    {#if editingNoteId === snap.id}
                      <input
                        type="text"
                        class="note-input"
                        bind:value={editingNoteText}
                        placeholder="ノート…"
                        onkeydown={(e) => {
                          if (e.key === 'Enter') saveNote(snap.id);
                          if (e.key === 'Escape') cancelEditNote();
                        }}
                      />
                      <button type="button" class="snap-btn-small" onclick={() => saveNote(snap.id)}
                        >保存</button
                      >
                      <button type="button" class="snap-btn-small" onclick={cancelEditNote}
                        >取消</button
                      >
                    {:else}
                      <!-- svelte-ignore a11y_no_static_element_interactions -->
                      <span class="note-text" ondblclick={() => startEditNote(snap)}
                        >{snap.note ?? ''}</span
                      >
                    {/if}
                  </td>
                  <td class="actions-cell">
                    {#if activeSnapshotId !== snap.id}
                      <button
                        type="button"
                        class="snap-btn"
                        disabled={snapshotLoading}
                        onclick={() => switchSnapshot(snap.id)}>切替</button
                      >
                    {:else}
                      <span class="snap-active">使用中</span>
                    {/if}
                    <button
                      type="button"
                      class="snap-btn-edit"
                      title="ノート編集"
                      onclick={() => startEditNote(snap)}
                      disabled={editingNoteId !== null}>✎</button
                    >
                    {#if !snap.isInitial}
                      <button
                        type="button"
                        class="snap-btn-del"
                        title="削除"
                        onclick={() => onDeleteSnapshot(snap.id)}>✕</button
                      >
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
        <div class="snap-actions">
          <button
            type="button"
            class="snap-refetch-btn"
            disabled={snapshotLoading}
            onclick={() => onRefetch(lp.videoId)}
          >
            {snapshotLoading ? '再取得中…' : 'niconico からコメント再取得'}
          </button>
        </div>

        <!-- コメント焼き込みエクスポート (Phase 1.9) -->
        <div class="burnin-section">
          <button
            type="button"
            class="burnin-toggle"
            disabled={activeSnapshotId == null}
            aria-expanded={burnInOpen}
            onclick={() => (burnInOpen = !burnInOpen)}
          >
            🎬 コメントを焼き込んでエクスポート
          </button>
          {#if burnInOpen}
            <div class="burnin-form">
              <label class="burnin-opt">
                <span>フォント倍率</span>
                <input
                  type="range"
                  min="0.5"
                  max="2"
                  step="0.1"
                  bind:value={binFontScale}
                  disabled={burnInRunning}
                />
                <span class="burnin-val">{binFontScale.toFixed(1)}×</span>
              </label>
              <label class="burnin-opt">
                <span>不透明度</span>
                <input
                  type="range"
                  min="0.1"
                  max="1"
                  step="0.05"
                  bind:value={binOpacity}
                  disabled={burnInRunning}
                />
                <span class="burnin-val">{Math.round(binOpacity * 100)}%</span>
              </label>
              <button
                type="button"
                class="burnin-go"
                disabled={burnInRunning || activeSnapshotId == null}
                onclick={() => onBurnIn(lp.videoId)}
              >
                {burnInRunning
                  ? `焼き込み中… ${Math.round(burnInPercent * 100)}%`
                  : 'エクスポート開始'}
              </button>
              {#if burnInRunning}
                <div class="burnin-bar">
                  <div
                    class="burnin-bar-fill"
                    style:width={`${Math.round(burnInPercent * 100)}%`}
                  ></div>
                </div>
              {/if}
              <p class="burnin-hint muted">
                映像を再エンコードするため時間がかかります。出力は exports フォルダに保存されます。
              </p>
            </div>
          {/if}
          {#if burnInMessage}
            <div class="snap-msg">{burnInMessage}</div>
          {/if}
          {#if burnInResultPath}
            <div class="burnin-result">
              <code class="burnin-path">{burnInResultPath}</code>
              <button type="button" class="snap-btn-small" onclick={revealExport}>
                フォルダを開く
              </button>
            </div>
          {/if}
        </div>
      </div>
    </div>
  {/if}
</section>

<style>
  .page {
    max-width: 1600px;
  }
  .page.classic {
    max-width: 1760px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 12px;
    flex-wrap: wrap;
  }
  .page.classic .head {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.65) inset;
    padding: 12px 14px;
  }
  .head h2 {
    margin: 0;
    font-size: 18px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .back {
    color: var(--theme-accent-soft);
    text-decoration: none;
    font-size: 13px;
    flex-shrink: 0;
  }
  .back:hover {
    text-decoration: underline;
  }
  .local-badge {
    background: var(--theme-success-bg);
    color: var(--theme-success-text);
    border: 1px solid var(--theme-success-border);
    padding: 2px 10px;
    border-radius: 999px;
    font-size: 11px;
    flex-shrink: 0;
  }
  .ghost-btn {
    background: transparent;
    border: 1px solid var(--theme-accent-border);
    color: var(--theme-accent-soft);
    padding: 2px 10px;
    border-radius: 999px;
    font-size: 11px;
    cursor: pointer;
  }
  .ghost-btn:hover:not(:disabled) {
    background: var(--theme-accent-bg);
  }
  .ghost-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .danger-btn {
    background: transparent;
    border: 1px solid var(--theme-danger-border);
    color: var(--theme-danger-text);
    padding: 2px 10px;
    border-radius: 999px;
    font-size: 11px;
    cursor: pointer;
  }
  .danger-btn:hover {
    background: var(--theme-danger-bg);
  }
  /* クラシック時、ヘッダ右側のローカルバッジ・ghost/danger ボタンは
     デフォルトのままだと白背景に薄色のテキストで「ボタンが消える」状態
     になる。video 側 dl-btn と同じ凸凹陰影の角丸 3px ボタンへ揃える。 */
  .page.classic .local-badge {
    border-radius: 3px;
    background: linear-gradient(180deg, #fffdf9 0%, #e9f0e3 100%);
    border: 1px solid #bfd0b3;
    color: #355f2e;
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.85) inset;
    padding: 3px 10px;
  }
  .page.classic .ghost-btn {
    background: linear-gradient(180deg, #ffffff 0%, #ebe2d4 100%);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    border-radius: 3px;
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.85) inset;
    padding: 3px 10px;
  }
  .page.classic .ghost-btn:hover:not(:disabled) {
    background: linear-gradient(180deg, #fff8ef 0%, #e6d6c2 100%);
  }
  .page.classic .danger-btn {
    background: linear-gradient(180deg, #fffafa 0%, #f6e0e0 100%);
    border: 1px solid #d8b1b1;
    color: #7e2020;
    border-radius: 3px;
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.85) inset;
    padding: 3px 10px;
  }
  .page.classic .danger-btn:hover {
    background: linear-gradient(180deg, #fff0f0 0%, #efc8c8 100%);
  }
  .muted {
    color: var(--theme-text-muted);
  }
  .error {
    background: var(--theme-danger-bg);
    border: 1px solid var(--theme-danger-border);
    color: var(--theme-danger-text);
    padding: 10px 12px;
    border-radius: 6px;
    font-size: 13px;
    white-space: pre-wrap;
  }
  .info {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
    border: 1px solid var(--theme-accent-border);
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 12px;
    margin-bottom: 8px;
  }
  .local-banner {
    display: flex;
    align-items: center;
    gap: 12px;
    background: linear-gradient(90deg, var(--theme-success-bg-2) 0%, var(--theme-success-bg) 100%);
    border: 1px solid var(--theme-success-border);
    border-left: 4px solid var(--theme-success-strong);
    color: var(--theme-success-text);
    padding: 10px 16px;
    border-radius: 6px;
    margin-bottom: 10px;
  }
  .local-marker {
    background: var(--theme-success-strong);
    color: var(--theme-success-bg-2);
    font-weight: 700;
    font-size: 11px;
    letter-spacing: 0.05em;
    padding: 4px 8px;
    border-radius: 4px;
    flex-shrink: 0;
  }
  .local-banner-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .local-banner-text strong {
    font-size: 14px;
    color: var(--theme-success-text);
  }
  .local-banner-sub {
    font-size: 11px;
    color: var(--theme-success-text);
  }
  .local-banner-online {
    color: var(--theme-accent-soft);
    text-decoration: none;
    font-size: 12px;
    padding: 4px 10px;
    border: 1px solid var(--theme-accent-border);
    border-radius: 999px;
    flex-shrink: 0;
  }
  .local-banner-online:hover {
    background: var(--theme-accent-bg);
  }
  .page.classic .local-banner {
    border-radius: 3px;
    background: linear-gradient(180deg, #fffdf7 0%, #e9f0e3 100%);
  }
  .player-frame {
    display: flex;
    flex-direction: column;
  }
  .page.classic .player-frame {
    border: 1px solid var(--theme-border);
    background: var(--theme-surface-2);
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.75) inset;
  }
  .viewer-toolbar {
    display: none;
  }
  .page.classic .viewer-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 10px 12px;
    background: linear-gradient(180deg, #fffdf9 0%, #efe6d9 100%);
    border-bottom: 1px solid var(--theme-border);
    color: var(--theme-text-soft);
    font-size: 13px;
  }
  .viewer-toolbar-meta,
  .viewer-toolbar-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .toolbar-id {
    font-weight: 700;
  }
  .toolbar-sep {
    color: var(--theme-text-faint);
  }
  .toolbar-link {
    display: inline-flex;
    align-items: center;
    padding: 7px 12px;
    border: 1px solid var(--theme-border-strong);
    border-radius: 3px;
    text-decoration: none;
    color: var(--theme-text);
    background: linear-gradient(180deg, #ffffff 0%, #ebe2d4 100%);
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.85) inset;
  }
  .toolbar-link:hover {
    background: linear-gradient(180deg, #fff8ef 0%, #e6d6c2 100%);
  }
  .player-row {
    display: flex;
    align-items: stretch;
  }
  .player-row.dragging {
    user-select: none;
    cursor: col-resize;
  }
  .player-col {
    flex: 1 1 auto;
    min-width: 0;
    contain: layout style paint;
  }
  .divider {
    width: 5px;
    cursor: col-resize;
    background: var(--theme-surface-3);
    border-left: 1px solid var(--theme-border-strong);
    border-right: 1px solid var(--theme-border-strong);
    flex-shrink: 0;
    transition: background 0.1s;
  }
  .divider:hover {
    background: var(--theme-surface-hover);
  }
  .dragging .divider {
    background: var(--theme-accent);
  }
  .page.classic .divider {
    width: 7px;
    background: linear-gradient(180deg, #f4efe6 0%, #e3d9ca 100%);
    border-left: 1px solid var(--theme-border);
    border-right: 1px solid var(--theme-border);
  }
  .comment-panel {
    flex-shrink: 0;
    overflow: hidden;
    position: relative;
  }
  .page.classic .comment-panel {
    background: var(--theme-surface-2);
    border-left: 1px solid var(--theme-border);
  }
  .side-header {
    display: none;
  }
  .page.classic .side-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--theme-border);
    background: linear-gradient(180deg, #fffdf9 0%, #f2e8db 100%);
    color: var(--theme-text);
    font-size: 13px;
    font-weight: 700;
  }
  .side-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-weight: 400;
    font-size: 12px;
    color: var(--theme-text-soft);
  }
  .below {
    display: grid;
    grid-template-columns: 1fr;
    gap: 16px;
    margin-top: 12px;
  }
  .meta {
    color: var(--theme-text-soft);
    font-size: 13px;
    min-width: 0;
    overflow: hidden;
  }
  .page.classic .meta {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.75) inset;
    padding: 12px 14px;
  }
  .page.classic .row:first-child {
    margin-top: 0;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--theme-border);
  }
  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }
  .dot {
    color: var(--theme-text-faint);
  }
  .external {
    margin-left: auto;
    color: var(--theme-accent-soft);
    text-decoration: none;
  }
  .external:hover {
    text-decoration: underline;
  }
  .owner-link {
    color: var(--theme-text);
    text-decoration: none;
  }
  .owner-link:hover {
    text-decoration: underline;
  }
  details {
    margin-top: 12px;
    color: var(--theme-text-soft);
  }
  details > summary {
    cursor: pointer;
    color: var(--theme-text-soft);
    margin-bottom: 6px;
  }
  .desc {
    white-space: pre-wrap;
    line-height: 1.6;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    padding: 10px 12px;
    border-radius: 6px;
    overflow: hidden;
    min-width: 0;
    word-break: break-word;
  }
  .page.classic .desc {
    border-radius: 3px;
    background: #fffcf7;
  }
  .ng-banner {
    background: var(--theme-danger-bg-2);
    color: var(--theme-danger-text);
    border: 1px solid var(--theme-danger-border);
    padding: 4px 10px;
    border-radius: 6px;
    font-size: 12px;
    margin-top: 6px;
    display: inline-block;
  }
  .pip-placeholder {
    background: var(--theme-bg);
    border-radius: 8px;
    overflow: hidden;
    aspect-ratio: 16 / 9;
    width: 100%;
    position: relative;
  }
  .pip-thumb {
    position: relative;
    width: 100%;
    height: 100%;
  }
  .pip-thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    filter: brightness(0.45) blur(4px);
  }
  .pip-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--theme-on-overlay);
  }
  .pip-icon {
    color: var(--theme-on-overlay);
    opacity: 0.85;
  }
  .pip-text {
    font-size: 14px;
    font-weight: 600;
    text-shadow: 0 1px 4px rgba(0, 0, 0, 0.6);
  }
  .pip-resume {
    margin-top: 4px;
    background: var(--theme-accent);
    color: var(--theme-on-overlay);
    border: none;
    padding: 8px 16px;
    border-radius: 8px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 600;
  }
  .pip-resume:hover {
    background: var(--theme-accent-hover);
  }
  .pip-other-title {
    font-size: 12px;
    color: var(--theme-on-overlay);
    opacity: 0.85;
    max-width: 80%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: center;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.6);
  }
  .pip-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: 10px;
    margin-top: 4px;
  }
  .pip-link {
    color: var(--theme-on-overlay);
    opacity: 0.9;
    font-size: 12px;
    text-decoration: underline;
    padding: 4px 8px;
  }
  .pip-link:hover {
    opacity: 1;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 8px;
  }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: var(--theme-border);
    color: var(--theme-chip-text);
    padding: 3px 10px;
    border-radius: 999px;
    font-size: 12px;
    text-decoration: none;
  }
  .page.classic .tag {
    border-radius: 3px;
    background: linear-gradient(180deg, #f6efe4 0%, #e7dccb 100%);
    border: 1px solid var(--theme-border);
    color: var(--theme-text-soft);
  }
  .tag:hover {
    background: var(--theme-border-strong);
    color: var(--theme-text);
  }
  .tag.locked {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
  }
  .lock {
    font-size: 9px;
    opacity: 0.7;
  }
  :global(body:has(:fullscreen)) .head,
  :global(body:has(:fullscreen)) .divider,
  :global(body:has(:fullscreen)) .comment-panel,
  :global(body:has(:fullscreen)) .below {
    display: none !important;
  }

  /* コメントスナップショット管理 */
  .snapshot-section {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 6px;
    padding: 12px 14px;
    min-width: 0;
  }
  .page.classic .snapshot-section {
    border-radius: 3px;
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.75) inset;
  }
  .snapshot-section h3 {
    margin: 0 0 10px 0;
    font-size: 14px;
  }
  .snap-msg {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
    border: 1px solid var(--theme-accent-border);
    padding: 6px 10px;
    border-radius: 6px;
    font-size: 12px;
    margin-bottom: 8px;
  }
  .snap-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .snap-table th {
    text-align: left;
    color: var(--theme-text-muted);
    font-weight: 500;
    padding: 4px 8px;
    border-bottom: 1px solid var(--theme-border);
    font-size: 11px;
  }
  .snap-table td {
    padding: 6px 8px;
    border-bottom: 1px solid var(--theme-border);
    vertical-align: middle;
  }
  .snap-table tr:hover {
    background: var(--theme-surface-hover);
  }
  .snap-table tr.active {
    background: var(--theme-accent-bg);
  }
  .snap-badge {
    background: var(--theme-success-bg);
    color: var(--theme-success-text);
    border: 1px solid var(--theme-success-border);
    padding: 1px 6px;
    border-radius: 999px;
    font-size: 10px;
    margin-left: 6px;
  }
  .note-cell {
    max-width: 200px;
  }
  .note-text {
    color: var(--theme-text-soft);
    cursor: default;
    display: inline-block;
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    line-height: 1.4;
  }
  .note-text:hover {
    color: var(--theme-text);
    text-decoration: underline;
    text-decoration-style: dotted;
  }
  .note-input {
    width: 120px;
    padding: 2px 6px;
    border: 1px solid var(--theme-accent-border);
    border-radius: 4px;
    font-size: 12px;
    background: var(--theme-input-bg);
    color: var(--theme-text);
  }
  .actions-cell {
    white-space: nowrap;
    text-align: right;
  }
  .snap-btn {
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    border: none;
    padding: 3px 10px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 11px;
  }
  .snap-btn:hover:not(:disabled) {
    background: var(--theme-accent-hover);
  }
  .snap-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .snap-active {
    color: var(--theme-accent-soft);
    font-size: 11px;
    font-weight: 600;
  }
  .snap-btn-edit {
    background: transparent;
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text-soft);
    padding: 3px 6px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 11px;
    margin-left: 4px;
  }
  .snap-btn-edit:hover:not(:disabled) {
    background: var(--theme-surface-hover);
  }
  .snap-btn-edit:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .snap-btn-del {
    background: transparent;
    border: 1px solid var(--theme-danger-border);
    color: var(--theme-danger-text);
    padding: 3px 6px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 11px;
    margin-left: 4px;
  }
  .snap-btn-del:hover {
    background: var(--theme-danger-bg);
  }
  .snap-btn-small {
    background: var(--theme-surface-3);
    border: 1px solid var(--theme-border);
    color: var(--theme-text-soft);
    padding: 2px 6px;
    border-radius: 3px;
    cursor: pointer;
    font-size: 10px;
    margin-left: 3px;
  }
  .snap-btn-small:hover {
    background: var(--theme-surface-hover);
  }
  .snap-actions {
    margin-top: 10px;
  }
  .snap-refetch-btn {
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    border: none;
    padding: 6px 16px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 12px;
    font-weight: 600;
  }
  .snap-refetch-btn:hover:not(:disabled) {
    background: var(--theme-accent-hover);
  }
  .snap-refetch-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* コメント焼き込みエクスポート */
  .burnin-section {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--theme-border, rgba(128, 128, 128, 0.25));
  }
  .burnin-toggle {
    background: transparent;
    color: var(--theme-accent);
    border: 1px solid var(--theme-accent);
    padding: 6px 14px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 12px;
    font-weight: 600;
  }
  .burnin-toggle:hover:not(:disabled) {
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
  }
  .burnin-toggle:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .burnin-form {
    margin-top: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 420px;
  }
  .burnin-opt {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }
  .burnin-opt > span:first-child {
    width: 72px;
    flex-shrink: 0;
  }
  .burnin-opt input[type='range'] {
    flex: 1;
  }
  .burnin-val {
    width: 44px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .burnin-go {
    align-self: flex-start;
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    border: none;
    padding: 6px 16px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 12px;
    font-weight: 600;
  }
  .burnin-go:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  .burnin-bar {
    height: 6px;
    border-radius: 3px;
    background: rgba(128, 128, 128, 0.3);
    overflow: hidden;
  }
  .burnin-bar-fill {
    height: 100%;
    background: var(--theme-accent);
    transition: width 0.2s linear;
  }
  .burnin-hint {
    font-size: 11px;
    margin: 2px 0 0;
  }
  .burnin-result {
    margin-top: 8px;
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .burnin-path {
    font-size: 11px;
    word-break: break-all;
    background: rgba(128, 128, 128, 0.15);
    padding: 2px 6px;
    border-radius: 4px;
  }
</style>
