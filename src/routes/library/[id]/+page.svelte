<script lang="ts">
  import { onDestroy } from 'svelte';
  import { beforeNavigate } from '$app/navigation';
  import { page } from '$app/state';
  import Player from '$lib/player/Player.svelte';
  import CommentList from '$lib/player/CommentList.svelte';
  import {
    deleteLibraryVideo,
    localAudioUrl,
    localVideoUrl,
    prepareLocalPlayback,
    remuxLocalVideo,
    type LocalPlaybackPayload,
  } from '$lib/api';
  import { formatDate, formatDuration, formatNumber, videoUrl } from '$lib/format';
  import type { PlayerComment } from '$lib/player/types';
  import { filterComments, listNgRules, subscribeNgRules, type NgRule } from '$lib/stores/ngRules';
  import { addHistory } from '$lib/stores/history';
  import { getBool, loadSettings } from '$lib/stores/settings.svelte';
  import { sanitizeDescriptionHtml } from '$lib/sanitize';
  import { miniPlayer } from '$lib/player/miniPlayerStore.svelte';

  let local = $state<LocalPlaybackPayload | null>(null);
  let localSrc = $state<string | null>(null);
  let localAudioSrc = $state<string | null>(null);
  let pending = $state(true);
  let error = $state<string | null>(null);
  let currentTime = $state(0);
  let comments = $state<PlayerComment[]>([]);

  let ngRules = $state<NgRule[]>(listNgRules());
  const ngUnsub = subscribeNgRules(() => (ngRules = listNgRules()));

  let visibleComments = $derived(filterComments(ngRules, comments));
  let ngFilteredCount = $derived(comments.length - visibleComments.length);

  // 動画は内蔵 HTTP サーバ (http://127.0.0.1:port/v/{id}/...) 経由で配信する。
  // Blob URL は WebKitGTK + GStreamer の組合せだと後方 seek でガビガビになる。

  type PlayerRef = { seek: (t: number) => void; getVideo: () => HTMLVideoElement | null };
  let playerRef = $state<PlayerRef | undefined>();
  let videoId = $derived(page.params.id ?? '');
  let loadingFor: string | null = null;
  let loop = $state(false);

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

    try {
      // 設定と再生情報を並列取得
      const [, result] = await Promise.all([loadSettings(), prepareLocalPlayback(id)]);
      loop = getBool('playback.always_loop');
      if (loadingFor !== id) return;
      if (!result) {
        error = `${id} はライブラリに無い、または video.mp4 が見つかりません。`;
        pending = false;
        return;
      }
      local = result;
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
      comments = result.comments.map((c) => ({
        id: c.id,
        no: c.no,
        vposMs: c.vposMs,
        content: c.content,
        mail: c.mail,
        commands: c.commands,
        userId: c.userId ?? undefined,
        postedAt: c.postedAt ?? undefined,
        fork: c.fork,
        isOwner: c.isOwner,
        nicoruCount: c.nicoruCount ?? undefined,
        score: c.score ?? undefined,
      }));
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

  function handleSeek(t: number) {
    playerRef?.seek(t);
  }

  function getResumePosition(id: string): number {
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
    if (!local || !localSrc || pipActiveForThis) return false;
    const vid = playerRef?.getVideo();
    const t = vid?.currentTime ?? currentTime ?? 0;
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
      resumePosition: t,
      expandHref: snapHref,
      loop,
    });
    return true;
  }

  function togglePip() {
    if (pipActiveForThis) {
      miniPlayer.close();
      return;
    }
    openPipForCurrentVideo();
  }

  let pipActiveForThis = $derived(
    miniPlayer.active && miniPlayer.source?.videoId === (local?.videoId ?? ''),
  );
  $effect(() => {
    if (pipActiveForThis && local) {
      miniPlayer.updateComments(local.videoId, visibleComments);
    }
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

  onDestroy(() => {
    ngUnsub();
  });
</script>

<svelte:window onmousemove={onMove} onmouseup={stopDrag} />

<section class="page">
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
        {:else}
          <Player
            bind:this={playerRef}
            hlsUrl=""
            localSrc={ls}
            localAudioSrc={las ?? undefined}
            comments={visibleComments}
            onTime={handleTimeUpdate}
            resumePosition={getResumePosition(lp.videoId)}
            {loop}
            onLoopChange={(v) => (loop = v)}
            onTogglePip={togglePip}
            pipActive={false}
          />
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
        <CommentList comments={visibleComments} {currentTime} onSeek={handleSeek} />
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
    </div>
  {/if}
</section>

<style>
  .page {
    max-width: 1600px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 12px;
    flex-wrap: wrap;
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
    color: #6ea8fe;
    text-decoration: none;
    font-size: 13px;
    flex-shrink: 0;
  }
  .back:hover {
    text-decoration: underline;
  }
  .local-badge {
    background: #1a3a26;
    color: #b3f5b3;
    border: 1px solid #2a5a3a;
    padding: 2px 10px;
    border-radius: 999px;
    font-size: 11px;
    flex-shrink: 0;
  }
  .ghost-btn {
    background: transparent;
    border: 1px solid #3a5a6a;
    color: #93c5fd;
    padding: 2px 10px;
    border-radius: 999px;
    font-size: 11px;
    cursor: pointer;
  }
  .ghost-btn:hover:not(:disabled) {
    background: #1f2a44;
  }
  .ghost-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .danger-btn {
    background: transparent;
    border: 1px solid #5a2222;
    color: #f5b3b3;
    padding: 2px 10px;
    border-radius: 999px;
    font-size: 11px;
    cursor: pointer;
  }
  .danger-btn:hover {
    background: #2a1212;
  }
  .muted {
    color: #9a9a9a;
  }
  .error {
    background: #2a1212;
    border: 1px solid #5a2222;
    color: #f5b3b3;
    padding: 10px 12px;
    border-radius: 6px;
    font-size: 13px;
    white-space: pre-wrap;
  }
  .info {
    background: #1a2a44;
    color: #93c5fd;
    border: 1px solid #2a3f5a;
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 12px;
    margin-bottom: 8px;
  }
  .local-banner {
    display: flex;
    align-items: center;
    gap: 12px;
    background: linear-gradient(90deg, #0f2a18 0%, #1a3a26 100%);
    border: 1px solid #2a5a3a;
    border-left: 4px solid #4ade80;
    color: #b3f5b3;
    padding: 10px 16px;
    border-radius: 6px;
    margin-bottom: 10px;
  }
  .local-marker {
    background: #4ade80;
    color: #052010;
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
    color: #d1fae5;
  }
  .local-banner-sub {
    font-size: 11px;
    color: #86efac;
  }
  .local-banner-online {
    color: #93c5fd;
    text-decoration: none;
    font-size: 12px;
    padding: 4px 10px;
    border: 1px solid #2a3f5a;
    border-radius: 999px;
    flex-shrink: 0;
  }
  .local-banner-online:hover {
    background: rgba(45, 65, 100, 0.4);
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
    background: #1a1a1a;
    border-left: 1px solid #2a2a2a;
    border-right: 1px solid #2a2a2a;
    flex-shrink: 0;
    transition: background 0.1s;
  }
  .divider:hover {
    background: #333;
  }
  .dragging .divider {
    background: #2563eb;
  }
  .comment-panel {
    flex-shrink: 0;
    overflow: hidden;
    position: relative;
  }
  .below {
    display: grid;
    grid-template-columns: 1fr;
    gap: 16px;
    margin-top: 12px;
  }
  .meta {
    color: #cfcfcf;
    font-size: 13px;
    min-width: 0;
    overflow: hidden;
  }
  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }
  .dot {
    color: #555;
  }
  .external {
    margin-left: auto;
    color: #6ea8fe;
    text-decoration: none;
  }
  .external:hover {
    text-decoration: underline;
  }
  .owner-link {
    color: #eaeaea;
    text-decoration: none;
  }
  .owner-link:hover {
    text-decoration: underline;
  }
  details {
    margin-top: 12px;
    color: #cfcfcf;
  }
  details > summary {
    cursor: pointer;
    color: #b0b0b0;
    margin-bottom: 6px;
  }
  .desc {
    white-space: pre-wrap;
    line-height: 1.6;
    background: #161616;
    border: 1px solid #1f1f1f;
    padding: 10px 12px;
    border-radius: 6px;
    overflow: hidden;
    min-width: 0;
    word-break: break-word;
  }
  .ng-banner {
    background: #2a1f1a;
    color: #f5b3b3;
    border: 1px solid #5a2222;
    padding: 4px 10px;
    border-radius: 6px;
    font-size: 12px;
    margin-top: 6px;
    display: inline-block;
  }
  .pip-placeholder {
    background: #000;
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
    color: #fff;
  }
  .pip-icon {
    color: #fff;
    opacity: 0.85;
  }
  .pip-text {
    font-size: 14px;
    font-weight: 600;
    text-shadow: 0 1px 4px rgba(0, 0, 0, 0.6);
  }
  .pip-resume {
    margin-top: 4px;
    background: #2563eb;
    color: #fff;
    border: none;
    padding: 8px 16px;
    border-radius: 8px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 600;
  }
  .pip-resume:hover {
    background: #3b78f0;
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
    background: #1f1f1f;
    color: #c0c0c0;
    padding: 3px 10px;
    border-radius: 999px;
    font-size: 12px;
    text-decoration: none;
  }
  .tag:hover {
    background: #2a2a2a;
    color: #fff;
  }
  .tag.locked {
    background: #1e2a3a;
    color: #b3c5ff;
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
</style>
