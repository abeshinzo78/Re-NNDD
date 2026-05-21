<script lang="ts">
  // YouTube 風のフローティングミニプレイヤー (PiP)。
  //
  // - `+layout.svelte` に常駐し、ルート遷移を跨いで再生継続。
  // - ドラッグで移動 / 角でリサイズ / リリースで近い四隅へスナップ。
  // - 元ページに居る時は自動的に非表示 (二重再生防止)。
  // - 期待挙動はホットゾーン UX: ホバーで操作 UI フェードイン、離れて
  //   2.5 秒でフェードアウト。
  //
  // 既存の `Player.svelte` を `compact` で取り回し、HLS / コメント /
  // 音声同期 / 復旧ロジックを丸ごと再利用する。

  import { onDestroy, onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import Player from './Player.svelte';
  import type { PlayerComment } from './types';
  import {
    MINI_CONSTANTS,
    miniPlayer,
    snapGeometry,
    clamp,
    clampWidth,
    type MiniGeometry,
  } from './miniPlayerStore.svelte';
  import { getBool, getNum } from '$lib/stores/settings.svelte';
  import { readSavedMuted, readSavedVolume } from './volumePersistence';
  import {
    dtoToPlayerComment,
    fetchVideoComments,
    issueHlsUrl,
    localAudioUrl,
    localVideoUrl,
    preparePlayback,
    prepareLocalPlayback,
  } from '$lib/api';
  import { advanceQueue, getQueue, hasNextInQueue, itemHref } from '$lib/stores/playbackQueue';
  import { filterComments, listNgRules, subscribeNgRules, type NgRule } from '$lib/stores/ngRules';
  import { addHistory } from '$lib/stores/history';

  type PlayerRef = {
    getVideo: () => HTMLVideoElement | null;
    seek: (t: number) => void;
    play: () => void;
    pause: () => void;
    getCurrentTime: () => number;
  };

  let playerRef = $state<PlayerRef | undefined>();
  let container = $state<HTMLDivElement | null>(null);
  // NG ルールは PiP 内のキュー進行で取得した新しい動画のコメに対して
  // フィルタを再適用するために必要 (ページ側 +page.svelte の visibleComments
  // 相当)。replaceSource() でコメを差し替えた直後、ページ側の updateComments
  // 効果は別 videoId なので発火しないため、ここで自前で持つ必要がある。
  let ngRules = $state<NgRule[]>(listNgRules());
  const ngUnsubMini = subscribeNgRules(() => (ngRules = listNgRules()));
  onDestroy(() => ngUnsubMini());
  let paused = $state(true);
  let currentTime = $state(0);
  let duration = $state(0);
  let hovering = $state(false);
  let hideHoverTimer: ReturnType<typeof setTimeout> | null = null;
  let isDragging = $state(false);
  let isResizing = $state(false);

  // 元動画ページに居る間の挙動:
  //  - 音声引き継ぎ中 (audioOwned=false): ページ側 Player がまだ実体として残り音を
  //    出している。ここで mini を見せると同じ動画が 2 個並んで気持ち悪いので隠す。
  //  - 引き継ぎ完了後 (audioOwned=true): ページ側は pip-placeholder に切り替わり、
  //    Player は破棄されている。mini を隠したままだと、ユーザは元ページに居る間
  //    流れコメを一切見られず (placeholder には canvas が無い)、別ページへ遷移
  //    して初めて mini にコメが現れる、という分かりにくい状態になる。
  //    なので引き継ぎ完了後は元ページでも mini を可視化し、placeholder と並べる。
  let pathname = $derived(page.url.pathname);
  let onSourcePage = $derived(
    miniPlayer.source != null && pathname === miniPlayer.expandHref.split('?')[0],
  );
  let hideForHandoff = $derived(onSourcePage && !miniPlayer.audioOwned);

  function saveResume() {
    const t = playerRef?.getCurrentTime() ?? currentTime;
    const id = miniPlayer.source?.videoId;
    if (id && Number.isFinite(t) && t > 0) {
      try {
        localStorage.setItem(`resume:${id}`, String(Math.floor(t)));
      } catch {
        /* ignore */
      }
    }
  }

  function expand() {
    const href = miniPlayer.expandHref;
    saveResume();
    miniPlayer.close();
    if (href && href !== pathname) {
      void goto(href);
    }
  }

  function close() {
    // 完全に閉じる (再生停止)。resume だけは保存しておく。
    saveResume();
    miniPlayer.close();
  }

  function togglePlay() {
    if (!playerRef) return;
    if (paused) playerRef.play();
    else playerRef.pause();
  }

  // PiP 内で再生中の動画が自然終了した時のオートプレイキュー進行。
  // ページ側 (/video/[id], /library/[id]) の `handleEnded` と同じ意味論を
  // mini 内で完結させる。PiP は元ページのライフサイクルから独立しているので
  // PiP 中に「次へ」が止まらないように mini が自前で処理する必要がある。
  //
  // ページ側と違って goto しないのは、PiP は元ページが placeholder に化けて
  // いる前提で動いており、別 URL へ goto すると新ページが「別動画 PiP 中」
  // プレースホルダを表示してしまい UX が壊れるため。代わりに store の
  // `replaceSource` で mini の中身だけを差し替える (YouTube ライク)。
  //
  // ページ側 `load()` と同じく、コメントと再生位置 (resume) もここで補充する。
  // 単純に `localAudioUrl()` を呼ぶと `local_audio_url` コマンドは 4 文字
  // atom を URL に詰めるだけで存在チェックをしないので、audio.mp4 が無い
  // muxed 動画でも URL が返り 404 連発になる。`prepareLocalPlayback` の
  // `localAudioPath` を見て分離音声がある時だけ URL を取る。
  function getResumeFor(id: string): number {
    if (!getBool('playback.resume_enabled')) return 0;
    try {
      return Number(localStorage.getItem(`resume:${id}`)) || 0;
    } catch {
      return 0;
    }
  }

  // 各 await の後、ユーザがキューを止めたり PiP を閉じたりしていないか
  // 再確認する。古い fetch が遅延到着して PiP に意図しない動画を流し込む
  // のを防ぐ (codex review)。`true` を返したら abort してよい。
  function staleSnapshot(currentId: string, expectedNxtId: string, idx: number): boolean {
    if (!miniPlayer.active) return true;
    if (miniPlayer.source?.videoId !== currentId) return true;
    const q2 = getQueue();
    if (!q2) return true;
    if (q2.items[idx + 1]?.videoId !== expectedNxtId) return true;
    return false;
  }

  // キュー次 item の loop 値を計算。常時ループ設定があり、かつ後続 item が
  // 無い (= キュー末尾) 時のみループを許可する。ページ側 computeDefaultLoop
  // と同じ規則。
  function nextItemLoop(nxtId: string): boolean {
    if (!getBool('playback.always_loop')) return false;
    return !hasNextInQueue(nxtId);
  }

  async function handleEnded() {
    if (!getBool('playback.autoplay_queue')) return;
    const q = getQueue();
    if (!q) return;
    const currentId = miniPlayer.source?.videoId;
    if (!currentId) return;
    const idx = q.items.findIndex((it) => it.videoId === currentId);
    if (idx < 0) return;
    const nxt = q.items[idx + 1];
    if (!nxt) return;
    try {
      const expandHref = itemHref(nxt);
      const title = nxt.title ?? nxt.videoId;
      const resumePosition = getResumeFor(nxt.videoId);
      if (nxt.source === 'online') {
        // `preparePlayback` 1 IPC で hlsUrl + nvComment を取得し、続けて
        // `fetchVideoComments` を発火する。先方の `/video/[id]` load() と同じ流れ。
        const result = await preparePlayback(nxt.videoId);
        if (staleSnapshot(currentId, nxt.videoId, idx)) return;
        let comments: PlayerComment[] = [];
        if (result.nvComment) {
          try {
            comments = await fetchVideoComments(result.nvComment);
            if (staleSnapshot(currentId, nxt.videoId, idx)) return;
          } catch (e) {
            console.warn('[MiniPlayer] comment fetch on queue advance failed', e);
          }
        }
        // ページ側 visibleComments と同じく NG ルールを適用してから渡す。
        const filtered = filterComments(ngRules, comments);
        miniPlayer.replaceSource({
          source: {
            kind: 'online',
            videoId: nxt.videoId,
            hlsUrl: result.hlsUrl,
            refreshHlsUrl: () => issueHlsUrl(nxt.videoId),
          },
          title,
          expandHref,
          comments: filtered,
          resumePosition,
          loop: nextItemLoop(nxt.videoId),
        });
        // ページ side `load()` と同じく history へ記録 (PiP 内完結再生でも
        // /history に出るように)。
        addHistory({
          videoId: result.video.id,
          title: result.video.title,
          thumbnailUrl: result.video.thumbnailUrl,
          uploaderName: result.owner?.nickname,
          duration: result.video.duration,
          viewCount: result.video.viewCount,
        });
      } else {
        // ローカルのコメントとメタは prepareLocalPlayback 一発で取れる。
        const result = await prepareLocalPlayback(nxt.videoId);
        if (staleSnapshot(currentId, nxt.videoId, idx)) return;
        if (!result) {
          console.warn('[MiniPlayer] queue advance: local playback unavailable for', nxt.videoId);
          return;
        }
        const localSrc = await localVideoUrl(nxt.videoId);
        if (staleSnapshot(currentId, nxt.videoId, idx)) return;
        const localAudioSrc = result.localAudioPath ? await localAudioUrl(nxt.videoId) : undefined;
        if (staleSnapshot(currentId, nxt.videoId, idx)) return;
        const comments = result.comments.map(dtoToPlayerComment);
        const filtered = filterComments(ngRules, comments);
        miniPlayer.replaceSource({
          source: {
            kind: 'local',
            videoId: nxt.videoId,
            localSrc,
            localAudioSrc,
          },
          title,
          expandHref,
          comments: filtered,
          resumePosition,
          loop: nextItemLoop(nxt.videoId),
        });
        addHistory({
          videoId: result.videoId,
          title: result.title,
          thumbnailUrl: result.thumbnailUrl ?? undefined,
          uploaderName: result.uploaderName ?? undefined,
          duration: result.durationSec,
          viewCount: result.viewCount ?? undefined,
          source: 'local',
        });
      }
      // 成功してから index を進める。先に進めてしまうと fetch 失敗時に
      // PiP は終わった動画のまま、永続キューだけが先に進む desync が起きる。
      advanceQueue();
    } catch (e) {
      console.warn('[MiniPlayer] queue advance failed', e);
    }
  }

  // PiP 内で NG ルールが変化したら現在表示中の comments も再フィルタする。
  // (キュー進行中の動画は既に NG 適用済みだが、追加変更を反映するため)
  $effect(() => {
    if (!miniPlayer.active) return;
    if (!miniPlayer.replacedFromQueue) return;
    void ngRules; // subscribe
    const reFiltered = filterComments(ngRules, miniPlayer.comments);
    if (reFiltered.length !== miniPlayer.comments.length) {
      const id = miniPlayer.source?.videoId;
      if (id) miniPlayer.updateComments(id, reFiltered);
    }
  });

  function onTimeFromPlayer(t: number) {
    currentTime = t;
    miniPlayer.setCurrentTime(t);
    // PiP 中もページ側と同じく resume を更新しておく
    const id = miniPlayer.source?.videoId;
    if (id && t > 0) {
      try {
        localStorage.setItem(`resume:${id}`, String(Math.floor(t)));
      } catch {
        /* ignore */
      }
    }
  }

  // PiP 起動時の「音声引き継ぎ」フック。
  // ページ側 Player は鳴り続けたまま、mini は無音 (volume=0) でロードしている。
  // 内部 Player が playing になった瞬間にこれが呼ばれる。
  //
  // 1. ページが進んだぶんのズレを埋めるため handoffTime までシーク
  //    (ロード時間ぶんの「音声巻き戻し」を防ぐ)
  // 2. ミニ側の音量を設定値に戻す
  // 3. miniPlayer.acquireAudio() でページ側にプレースホルダ切替を指示
  //
  // 音量の戻しは acquireAudio より「前」に行う。ページ側の Player が破棄され
  // 音が消える瞬間に mini が既に鳴っている状態を作るため。
  function handleReadyForAudio() {
    if (!playerRef) return;
    const targetTime = miniPlayer.handoffTime;
    const v = playerRef.getVideo();
    if (targetTime > 0) {
      const here = playerRef.getCurrentTime();
      // 前方にズレている (ページが先行している) 時だけシーク。
      // 後方は通常起きないが、稀に mini が先行してしまった場合は触らない。
      if (targetTime > here + 0.3) {
        playerRef.seek(targetTime);
      }
    }
    if (v) {
      // 引き継ぎ後の音量はソース側 Player の最新値 (= ユーザが直近に
      // 選んだ値) を引き継ぐ。これが無いと PiP に切り替えた瞬間に
      // 設定の `default_volume` (既定 1.0) へジャンプしてしまう。
      // ソース側 Player は `onVolumeChange` でこの値を localStorage に
      // 書き続けているため、ここで読むだけで OK。
      const saved = readSavedVolume();
      if (saved != null) {
        v.volume = saved;
      } else {
        const vol = getNum('playback.default_volume');
        v.volume = Number.isFinite(vol) ? Math.max(0, Math.min(1, vol)) : 1;
      }
      if (readSavedMuted()) v.muted = true;
    }
    // 引き継ぎ完了直前にユーザがソース側で停止していた場合、mini も停止して
    // 引き継ぐ。これでユーザの「停止したい」意図を尊重する。
    if (miniPlayer.sourcePaused) {
      try {
        playerRef.pause();
      } catch {
        /* ignore */
      }
    }
    miniPlayer.acquireAudio();
  }

  // 何らかの理由で playing が来ない時 (HLS のエラー継続など) も
  // 永遠にページ側 Player を残し続けるのは UX 上良くないので、
  // 一定時間で安全に判定する保険。
  //
  // ただし無条件に acquireAudio() してしまうと、mini が実際には再生に
  // 到達していないケース (HLS デコード失敗 / マニフェスト 403 連発など) で
  // 「ページ側 Player を破棄 → mini も鳴らない」= 完全無音になってしまう。
  // そこで mini の <video> が真に進行しているかを確認し、
  //  - 進行している (= playing イベントを取りこぼしただけ): 通常の引き継ぎへ
  //  - 進行していない: 引き継ぎを諦め、mini を閉じてページ側に音声を残す
  // ようにする。
  let handoffFallbackTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    if (miniPlayer.active && miniPlayer.wasPlaying && !miniPlayer.audioOwned) {
      if (handoffFallbackTimer) clearTimeout(handoffFallbackTimer);
      handoffFallbackTimer = setTimeout(() => {
        if (miniPlayer.audioOwned) return;
        const v = playerRef?.getVideo();
        const reallyPlaying =
          !!v && !v.paused && !v.ended && v.readyState >= 2 && v.currentTime > 0;
        if (reallyPlaying) {
          // playing イベントを取りこぼした稀ケース。通常パスへ寄せる。
          handleReadyForAudio();
        } else {
          // mini が起動できていない。ページ側の音声を維持するため mini を閉じる。
          // (ユーザは再度 PiP ボタンを押せば再試行できる)
          miniPlayer.close();
        }
      }, 8000);
      return () => {
        if (handoffFallbackTimer) clearTimeout(handoffFallbackTimer);
        handoffFallbackTimer = null;
      };
    }
  });

  // <video> の paused/duration を 100ms ポーリング (Player.svelte の内部
  // state を直接 bind できないため)。負荷は無視できる。
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  $effect(() => {
    if (!miniPlayer.active) return;
    pollTimer = setInterval(() => {
      const v = playerRef?.getVideo();
      if (!v) return;
      paused = v.paused;
      duration = Number.isFinite(v.duration) ? v.duration : 0;
    }, 200);
    return () => {
      if (pollTimer) clearInterval(pollTimer);
      pollTimer = null;
    };
  });

  // ============ Hover / 操作 UI 表示制御 ============
  function showOverlay() {
    hovering = true;
    if (hideHoverTimer) {
      clearTimeout(hideHoverTimer);
      hideHoverTimer = null;
    }
  }
  function scheduleHide() {
    if (hideHoverTimer) clearTimeout(hideHoverTimer);
    hideHoverTimer = setTimeout(() => {
      hovering = false;
      hideHoverTimer = null;
    }, 2500);
  }
  function onMouseLeave() {
    if (isDragging || isResizing) return;
    scheduleHide();
  }

  // ============ ドラッグ ============
  let dragStart: { x: number; y: number; gx: number; gy: number; pid: number } | null = null;

  function onDragPointerDown(e: PointerEvent) {
    const target = e.target as HTMLElement;
    if (target.closest('.no-drag')) return;
    // 左クリックのみ
    if (e.button !== 0) return;
    e.preventDefault();
    isDragging = true;
    dragStart = {
      x: e.clientX,
      y: e.clientY,
      gx: miniPlayer.geometry.x,
      gy: miniPlayer.geometry.y,
      pid: e.pointerId,
    };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onDragPointerMove(e: PointerEvent) {
    if (!isDragging || !dragStart || e.pointerId !== dragStart.pid) return;
    const height = miniPlayer.geometry.width / MINI_CONSTANTS.ASPECT_RATIO;
    const maxX = Math.max(MINI_CONSTANTS.MARGIN, window.innerWidth - miniPlayer.geometry.width);
    const maxY = Math.max(MINI_CONSTANTS.MARGIN, window.innerHeight - height);
    const nx = clamp(dragStart.gx + (e.clientX - dragStart.x), 0, maxX);
    const ny = clamp(dragStart.gy + (e.clientY - dragStart.y), 0, maxY);
    miniPlayer.setGeometry({ width: miniPlayer.geometry.width, x: nx, y: ny });
  }

  function onDragPointerUp(e: PointerEvent) {
    if (!isDragging || !dragStart || e.pointerId !== dragStart.pid) return;
    isDragging = false;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
    dragStart = null;
    // 近い四隅へスナップ
    const snapped = snapGeometry(miniPlayer.geometry, window.innerWidth, window.innerHeight);
    miniPlayer.setGeometry(snapped);
    scheduleHide();
  }

  // ============ リサイズ (左上角) ============
  let resizeStart: {
    x: number;
    y: number;
    w: number;
    gx: number;
    gy: number;
    pid: number;
  } | null = null;

  function onResizePointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    isResizing = true;
    resizeStart = {
      x: e.clientX,
      y: e.clientY,
      w: miniPlayer.geometry.width,
      gx: miniPlayer.geometry.x,
      gy: miniPlayer.geometry.y,
      pid: e.pointerId,
    };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onResizePointerMove(e: PointerEvent) {
    if (!isResizing || !resizeStart || e.pointerId !== resizeStart.pid) return;
    // 左上角を掴んでいる前提: ドラッグ方向の負号で幅増。
    const dx = resizeStart.x - e.clientX;
    const dy = resizeStart.y - e.clientY;
    // x,y の動きの大きい方で幅を決める。aspect-ratio 16:9 を維持。
    const deltaW = Math.max(dx, dy * MINI_CONSTANTS.ASPECT_RATIO);
    const newW = clampWidth(resizeStart.w + deltaW);
    const wDiff = newW - resizeStart.w;
    const hDiff = wDiff / MINI_CONSTANTS.ASPECT_RATIO;
    const nx = clamp(resizeStart.gx - wDiff, 0, Math.max(0, window.innerWidth - newW));
    const ny = clamp(
      resizeStart.gy - hDiff,
      0,
      Math.max(0, window.innerHeight - newW / MINI_CONSTANTS.ASPECT_RATIO),
    );
    miniPlayer.setGeometry({ width: newW, x: nx, y: ny });
  }

  function onResizePointerUp(e: PointerEvent) {
    if (!isResizing || !resizeStart || e.pointerId !== resizeStart.pid) return;
    isResizing = false;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
    resizeStart = null;
    const snapped = snapGeometry(miniPlayer.geometry, window.innerWidth, window.innerHeight);
    miniPlayer.setGeometry(snapped);
  }

  // ============ ウィンドウリサイズに追従 ============
  function onWindowResize() {
    const g = miniPlayer.geometry;
    const w = clampWidth(
      Math.min(
        g.width,
        Math.max(MINI_CONSTANTS.MIN_WIDTH, window.innerWidth - MINI_CONSTANTS.MARGIN * 2),
      ),
    );
    const h = w / MINI_CONSTANTS.ASPECT_RATIO;
    const next: MiniGeometry = {
      width: w,
      x: clamp(g.x, 0, Math.max(0, window.innerWidth - w)),
      y: clamp(g.y, 0, Math.max(0, window.innerHeight - h)),
    };
    miniPlayer.setGeometry(next);
  }

  // ============ キーボード ============
  function onKeyDown(e: KeyboardEvent) {
    // mini が非表示の時はキー入力を奪わない。
    //  - 音声引き継ぎ中 (元ページ上で hideForHandoff=true): ページ側 Player と
    //    ControlBar が現役なのでそちらにショートカットを譲る。
    //  - 引き継ぎ完了後で mini が可視 (元ページ含む): ページ側 Player は破棄され
    //    ControlBar も無いので、ユーザの操作対象は mini 一本。mini が拾う。
    if (!miniPlayer.active || hideForHandoff) return;
    // テキスト入力中は無視
    const tgt = e.target as HTMLElement | null;
    if (tgt) {
      const tag = tgt.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || tgt.isContentEditable)
        return;
    }
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      // 同じ window に登録されたページ Player のショートカット (例: F の
      // 全画面トグル) も同時発火するのを防ぐ。MiniPlayer は layout 経由で
      // 先に listen し始めるので、ここで stopImmediatePropagation すれば
      // 後段のページ Player リスナは呼ばれない。
      e.stopImmediatePropagation();
      close();
    } else if (e.key === ' ') {
      e.preventDefault();
      e.stopImmediatePropagation();
      togglePlay();
    } else if (e.key === 'p' || e.key === 'P') {
      e.preventDefault();
      e.stopImmediatePropagation();
      expand();
    }
  }

  onMount(() => {
    miniPlayer.hydrate();
    window.addEventListener('resize', onWindowResize);
    // ページ Player は target.addEventListener('keydown', ...) で window に
    // 後から登録する。MiniPlayer は layout マウント時点で先に登録するため、
    // capture: true 指定により確実にこちらが先に呼ばれるようにする。
    // (stopImmediatePropagation で後段を打ち切れる)
    window.addEventListener('keydown', onKeyDown, { capture: true });
  });

  onDestroy(() => {
    if (typeof window !== 'undefined') {
      window.removeEventListener('resize', onWindowResize);
      window.removeEventListener('keydown', onKeyDown, { capture: true });
    }
    if (hideHoverTimer) clearTimeout(hideHoverTimer);
    if (pollTimer) clearInterval(pollTimer);
  });

  // テンプレート用 derived
  let height = $derived(miniPlayer.geometry.width / MINI_CONSTANTS.ASPECT_RATIO);
  let progressPct = $derived(duration > 0 ? Math.min(100, (currentTime / duration) * 100) : 0);
  let onlineSrc = $derived(miniPlayer.source?.kind === 'online' ? miniPlayer.source : null);
  let localSrcObj = $derived(miniPlayer.source?.kind === 'local' ? miniPlayer.source : null);
</script>

{#if miniPlayer.active && miniPlayer.source}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="mini"
    class:hidden={hideForHandoff}
    class:dragging={isDragging}
    class:resizing={isResizing}
    bind:this={container}
    style:left="{miniPlayer.geometry.x}px"
    style:top="{miniPlayer.geometry.y}px"
    style:width="{miniPlayer.geometry.width}px"
    style:height="{height}px"
    onmouseenter={showOverlay}
    onmousemove={showOverlay}
    onmouseleave={onMouseLeave}
    aria-label="ミニプレイヤー"
  >
    <!-- 動画レイヤ。Player.svelte を compact モードで埋め込む。
         source.videoId が変わったら強制 remount する。Player 内部の
         `resumeApplied` フラグは一度 true になると次の動画でも残ってしまい、
         新しい resumePosition が無視されるため。 -->
    <div class="video-wrap">
      {#key miniPlayer.source.videoId}
        {#if onlineSrc}
          <Player
            bind:this={playerRef}
            hlsUrl={onlineSrc.hlsUrl}
            comments={miniPlayer.comments}
            refreshHlsUrl={onlineSrc.refreshHlsUrl}
            onTime={onTimeFromPlayer}
            onEnded={handleEnded}
            resumePosition={miniPlayer.resumePosition}
            loop={miniPlayer.loop}
            compact={true}
            initialMuted={miniPlayer.wasPlaying}
            onReadyForAudio={handleReadyForAudio}
            forceAutoplay={miniPlayer.replacedFromQueue}
          />
        {:else if localSrcObj}
          <Player
            bind:this={playerRef}
            hlsUrl=""
            localSrc={localSrcObj.localSrc}
            localAudioSrc={localSrcObj.localAudioSrc}
            comments={miniPlayer.comments}
            onTime={onTimeFromPlayer}
            onEnded={handleEnded}
            resumePosition={miniPlayer.resumePosition}
            loop={miniPlayer.loop}
            compact={true}
            initialMuted={miniPlayer.wasPlaying}
            onReadyForAudio={handleReadyForAudio}
            forceAutoplay={miniPlayer.replacedFromQueue}
          />
        {/if}
      {/key}
    </div>

    <!-- ドラッグ + クリック→展開 のキャッチオール -->
    <button
      type="button"
      class="drag-surface"
      onpointerdown={onDragPointerDown}
      onpointermove={onDragPointerMove}
      onpointerup={onDragPointerUp}
      onpointercancel={onDragPointerUp}
      ondblclick={expand}
      aria-label="ドラッグで移動 / ダブルクリックで展開"
    ></button>

    <!-- ホバー時の操作オーバーレイ -->
    <div class="overlay" class:show={hovering || paused || isDragging}>
      <!-- top bar: title + close/expand -->
      <div class="top">
        <div class="title" title={miniPlayer.title}>{miniPlayer.title}</div>
        <div class="top-actions no-drag">
          <button
            type="button"
            class="icon-btn"
            title="展開 (P)"
            aria-label="元のページに展開"
            onclick={expand}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
              <path
                d="M14 3h7v7h-2V6.41l-7.29 7.3-1.42-1.42 7.3-7.29H14V3zM10 21H3v-7h2v3.59l7.29-7.3 1.42 1.42-7.3 7.29H10V21z"
                fill="currentColor"
              />
            </svg>
          </button>
          <button
            type="button"
            class="icon-btn"
            title="閉じる (Esc)"
            aria-label="ミニプレイヤーを閉じる"
            onclick={close}
          >
            <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
              <path
                d="M18.3 5.71 12 12.01l-6.3-6.3-1.4 1.41 6.29 6.3-6.29 6.3 1.4 1.41 6.3-6.3 6.3 6.3 1.41-1.41-6.3-6.3 6.3-6.3z"
                fill="currentColor"
              />
            </svg>
          </button>
        </div>
      </div>

      <!-- center: play/pause -->
      <div class="center">
        <button
          type="button"
          class="play-btn no-drag"
          onclick={togglePlay}
          title={paused ? '再生 (Space)' : '一時停止 (Space)'}
          aria-label={paused ? '再生' : '一時停止'}
        >
          {#if paused}
            <svg viewBox="0 0 24 24" width="28" height="28" aria-hidden="true">
              <path d="M8 5v14l11-7L8 5z" fill="currentColor" />
            </svg>
          {:else}
            <svg viewBox="0 0 24 24" width="28" height="28" aria-hidden="true">
              <path d="M6 5h4v14H6V5zm8 0h4v14h-4V5z" fill="currentColor" />
            </svg>
          {/if}
        </button>
      </div>
    </div>

    <!-- 進捗バー (常時表示) -->
    <div class="progress" aria-hidden="true">
      <div class="progress-fill" style:width="{progressPct}%"></div>
    </div>

    <!-- 左上リサイズハンドル -->
    <button
      type="button"
      class="resize-handle no-drag"
      aria-label="リサイズ"
      title="ドラッグでサイズ変更"
      onpointerdown={onResizePointerDown}
      onpointermove={onResizePointerMove}
      onpointerup={onResizePointerUp}
      onpointercancel={onResizePointerUp}
    >
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
        <path
          d="M2 14L14 2M2 10L10 2M2 6L6 2"
          stroke="currentColor"
          stroke-width="1.5"
          fill="none"
        />
      </svg>
    </button>
  </div>
{/if}

<style>
  .mini {
    position: fixed;
    z-index: 9999;
    background: var(--theme-bg);
    border-radius: 12px;
    overflow: hidden;
    box-shadow:
      0 10px 32px rgba(0, 0, 0, 0.55),
      0 2px 8px rgba(0, 0, 0, 0.4),
      0 0 0 1px rgba(255, 255, 255, 0.08);
    transition:
      left 0.18s cubic-bezier(0.22, 1, 0.36, 1),
      top 0.18s cubic-bezier(0.22, 1, 0.36, 1),
      box-shadow 0.2s,
      transform 0.22s cubic-bezier(0.22, 1, 0.36, 1);
    animation: mini-in 0.22s cubic-bezier(0.22, 1, 0.36, 1);
    will-change: left, top, width, height;
    user-select: none;
  }
  .mini.hidden {
    visibility: hidden;
    pointer-events: none;
  }
  .mini.dragging,
  .mini.resizing {
    transition: none;
    box-shadow:
      0 16px 48px rgba(0, 0, 0, 0.7),
      0 4px 12px rgba(0, 0, 0, 0.5),
      0 0 0 1px rgba(255, 255, 255, 0.16);
  }
  .mini.dragging {
    cursor: grabbing;
  }
  @keyframes mini-in {
    from {
      transform: scale(0.92);
      opacity: 0;
    }
    to {
      transform: scale(1);
      opacity: 1;
    }
  }
  .video-wrap {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .video-wrap :global(.player) {
    width: 100%;
    height: 100%;
    border-radius: 0;
  }
  .video-wrap :global(video) {
    width: 100%;
    height: 100%;
    aspect-ratio: auto !important;
    object-fit: contain;
  }
  /* drag-surface: 透明な全面 button。クリックを拾いつつドラッグの起点。 */
  .drag-surface {
    position: absolute;
    inset: 0;
    background: transparent;
    border: none;
    padding: 0;
    cursor: grab;
    z-index: 2;
  }
  .drag-surface:active {
    cursor: grabbing;
  }

  .overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding: 8px 10px 18px;
    opacity: 0;
    transition: opacity 0.18s ease;
    pointer-events: none;
    z-index: 3;
    background: linear-gradient(
      180deg,
      rgba(0, 0, 0, 0.55) 0%,
      rgba(0, 0, 0, 0) 35%,
      rgba(0, 0, 0, 0) 65%,
      rgba(0, 0, 0, 0.55) 100%
    );
  }
  .overlay.show {
    opacity: 1;
  }
  .overlay > * {
    pointer-events: auto;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    color: #fff;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.6);
  }
  .title {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    font-weight: 600;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 2px 4px;
  }
  .top-actions {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }
  .icon-btn {
    background: rgba(0, 0, 0, 0.5);
    color: #fff;
    border: none;
    border-radius: 6px;
    padding: 4px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s;
  }
  .icon-btn:hover {
    background: rgba(255, 255, 255, 0.22);
  }
  .center {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: 1;
  }
  .play-btn {
    background: rgba(0, 0, 0, 0.55);
    color: #fff;
    border: none;
    width: 52px;
    height: 52px;
    border-radius: 999px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition:
      transform 0.15s,
      background 0.15s;
  }
  .play-btn:hover {
    background: rgba(255, 255, 255, 0.25);
    transform: scale(1.08);
  }
  .progress {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 3px;
    background: rgba(255, 255, 255, 0.12);
    z-index: 4;
    pointer-events: none;
  }
  .progress-fill {
    height: 100%;
    background: var(--theme-accent);
    transition: width 0.18s linear;
  }
  .resize-handle {
    position: absolute;
    top: 0;
    left: 0;
    width: 18px;
    height: 18px;
    background: transparent;
    border: none;
    cursor: nwse-resize;
    color: rgba(255, 255, 255, 0.7);
    padding: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    opacity: 0;
    transition: opacity 0.18s;
    z-index: 5;
  }
  .mini:hover .resize-handle,
  .mini.dragging .resize-handle,
  .mini.resizing .resize-handle {
    opacity: 1;
  }
</style>
