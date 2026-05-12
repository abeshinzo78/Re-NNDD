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
  import {
    MINI_CONSTANTS,
    miniPlayer,
    snapGeometry,
    clamp,
    clampWidth,
    type MiniGeometry,
  } from './miniPlayer.svelte';

  type PlayerRef = {
    getVideo: () => HTMLVideoElement | null;
    seek: (t: number) => void;
    play: () => void;
    pause: () => void;
    getCurrentTime: () => number;
  };

  let playerRef = $state<PlayerRef | undefined>();
  let container = $state<HTMLDivElement | null>(null);
  let paused = $state(true);
  let currentTime = $state(0);
  let duration = $state(0);
  let hovering = $state(false);
  let hideHoverTimer: ReturnType<typeof setTimeout> | null = null;
  let isDragging = $state(false);
  let isResizing = $state(false);

  // 元動画ページに居る間は、ページ側のプレースホルダが視覚を引き受け、
  // ミニ側は visibility:hidden で見えなくする (再生は継続)。
  // ユーザが「ここで再生に戻す」ボタンや mini の閉じる/展開を押した時のみ
  // miniPlayer.close() を呼ぶ。
  let pathname = $derived(page.url.pathname);
  let onSourcePage = $derived(
    miniPlayer.source != null && pathname === miniPlayer.expandHref.split('?')[0],
  );

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
    if (!miniPlayer.active || onSourcePage) return;
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
            resumePosition={miniPlayer.resumePosition}
            loop={miniPlayer.loop}
            compact={true}
          />
        {:else if localSrcObj}
          <Player
            bind:this={playerRef}
            hlsUrl=""
            localSrc={localSrcObj.localSrc}
            localAudioSrc={localSrcObj.localAudioSrc}
            comments={miniPlayer.comments}
            onTime={onTimeFromPlayer}
            resumePosition={miniPlayer.resumePosition}
            loop={miniPlayer.loop}
            compact={true}
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
    background: #000;
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
    background: #ff3355;
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
