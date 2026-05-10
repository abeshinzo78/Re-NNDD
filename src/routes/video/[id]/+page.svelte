<script lang="ts">
  import { onDestroy } from 'svelte';
  import { page } from '$app/state';
  import Player from '$lib/player/Player.svelte';
  import CommentList from '$lib/player/CommentList.svelte';
  import { fetchVideoComments, issueHlsUrl, preparePlayback, type SearchHit } from '$lib/api';
  import { quickDownload } from '$lib/quickDownload';
  import { formatDate, formatDuration, formatNumber, videoUrl } from '$lib/format';
  import type { PlaybackPayload, PlayerComment } from '$lib/player/types';
  import { fetchRelatedVideos } from '$lib/relatedVideos';
  import { loadSearchState } from '$lib/stores/searchState';
  import SearchHitCard from '$lib/SearchHitCard.svelte';
  import MylistAddButton from '$lib/MylistAddButton.svelte';
  import { filterComments, listNgRules, subscribeNgRules, type NgRule } from '$lib/stores/ngRules';
  import { addHistory } from '$lib/stores/history';
  import { getBool, loadSettings } from '$lib/stores/settings.svelte';

  // この route は **オンライン視聴専用**。ローカル再生は /library/[id] で行う。
  // 別ルートに分けることで、ネット接続が要らないときに偶発的に niconico を
  // 叩いてしまう事故を防ぐ。
  let payload = $state<PlaybackPayload | null>(null);
  let pending = $state(true);
  let error = $state<string | null>(null);
  let currentTime = $state(0);
  let comments = $state<PlayerComment[]>([]);
  let commentsLoading = $state(false);

  let related = $state<SearchHit[]>([]);
  let relatedLoading = $state(false);
  let relatedVisibleCount = $state(0);
  let relatedError = $state<string | null>(null);

  let ngRules = $state<NgRule[]>(listNgRules());
  const ngUnsub = subscribeNgRules(() => (ngRules = listNgRules()));
  onDestroy(() => ngUnsub());

  let visibleComments = $derived(filterComments(ngRules, comments));
  let ngFilteredCount = $derived(comments.length - visibleComments.length);

  function tagSearchHref(tag: string): string {
    return `/search?q=${encodeURIComponent(tag)}&targets=tagsExact`;
  }

  let backHref = $state('/search');
  let backLabel = $state('← 検索に戻る');

  type PlayerRef = { seek: (t: number) => void; getVideo: () => HTMLVideoElement | null };
  let playerRef = $state<PlayerRef | undefined>();

  let videoId = $derived(page.params.id ?? '');
  let loadingFor: string | null = null;
  let loop = $state(false);

  let panelWidth = $state(320);
  let dragging = $state(false);
  let dragStartX = 0;
  let dragStartWidth = 0;

  // Choose the back-link target based on referrer info (from query param).
  $effect(() => {
    const from = page.url.searchParams.get('from');
    if (from === 'history') {
      backHref = '/history';
      backLabel = '← 履歴に戻る';
    } else if (from === 'user') {
      const uid = page.url.searchParams.get('uid');
      const kind = page.url.searchParams.get('kind') ?? 'user';
      const name = page.url.searchParams.get('name') ?? '';
      const icon = page.url.searchParams.get('icon') ?? '';
      if (uid) {
        const params = new URLSearchParams({ kind });
        if (name) params.set('name', name);
        if (icon) params.set('icon', icon);
        backHref = `/user/${uid}?${params}`;
        backLabel = `← ${name || uid} の投稿動画に戻る`;
      } else {
        backHref = '/search';
        backLabel = '← 検索に戻る';
      }
    } else {
      const prev = loadSearchState();
      if (prev?.lastQuery) {
        backHref = '/search';
        backLabel = `← 「${prev.lastQuery}」の検索結果に戻る`;
      } else {
        backHref = '/search';
        backLabel = '← 検索に戻る';
      }
    }
  });

  async function load(id: string) {
    if (!id) return;
    loadingFor = id;
    pending = true;
    error = null;
    payload = null;
    comments = [];
    commentsLoading = false;
    related = [];
    relatedError = null;
    await loadSettings();
    loop = getBool('playback.always_loop');

    try {
      // オンライン専用ルート — ローカル DL 済みでも常にストリーミングする。
      // ネット不要で見たい場合は /library/[id] を使う。
      const result = await preparePlayback(id);
      if (loadingFor !== id) return;
      payload = result;
      pending = false;

      // Record to history
      addHistory({
        videoId: result.video.id,
        title: result.video.title,
        thumbnailUrl: result.video.thumbnailUrl,
        uploaderName: result.owner?.nickname,
        duration: result.video.duration,
        viewCount: result.video.viewCount,
      });

      // Load comments + related in parallel — video already playable.
      if (result.nvComment) {
        commentsLoading = true;
        void fetchVideoComments(result.nvComment)
          .then((c) => {
            if (loadingFor !== id) return;
            comments = c;
          })
          .catch((e) => {
            console.warn('comment fetch failed', e);
          })
          .finally(() => {
            if (loadingFor === id) commentsLoading = false;
          });
      }

      // Defer related-video fetch so it doesn't compete with the
      // player's initial buffering / segment downloads.
      relatedLoading = true;
      setTimeout(() => {
        void fetchRelatedVideos(id, result.video.title, result.video.tags)
          .then((hits) => {
            if (loadingFor !== id) return;
            related = hits;
            // Reveal cards progressively so thumbnail decode doesn't block video
            relatedVisibleCount = 0;
            const reveal = () => {
              relatedVisibleCount += 3;
              if (relatedVisibleCount < hits.length) {
                if (window.requestIdleCallback) {
                  window.requestIdleCallback(reveal, { timeout: 200 });
                } else {
                  setTimeout(reveal, 200);
                }
              }
            };
            reveal();
          })
          .catch((e) => {
            if (loadingFor !== id) return;
            relatedError = String(e);
          })
          .finally(() => {
            if (loadingFor === id) {
              relatedLoading = false;
            }
          });
      }, 3000);
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
    if (payload && time > 0) {
      saveResumePosition(payload.video.id, time);
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

  // DL ボタン用 state
  let dlPending = $state(false);
  let dlMsg = $state<{ kind: 'ok' | 'error'; text: string } | null>(null);
  let dlMsgTimer: ReturnType<typeof setTimeout> | null = null;
  function showDlMsg(kind: 'ok' | 'error', text: string) {
    dlMsg = { kind, text };
    if (dlMsgTimer) clearTimeout(dlMsgTimer);
    dlMsgTimer = setTimeout(() => {
      dlMsg = null;
      dlMsgTimer = null;
    }, 4000);
  }
  async function onDownload(id: string) {
    dlPending = true;
    try {
      const r = await quickDownload(id);
      showDlMsg(r.ok ? 'ok' : 'error', r.message);
    } finally {
      dlPending = false;
    }
  }
</script>

<svelte:window onmousemove={onMove} onmouseup={stopDrag} />

<section class="page">
  <div class="head">
    <a class="back" href={backHref}>{backLabel}</a>
    <h2>{payload?.video.title ?? videoId}</h2>
    {#if payload}
      <div class="head-actions">
        <button
          type="button"
          class="dl-btn"
          disabled={dlPending}
          onclick={() => onDownload(payload!.video.id)}
          title="この動画をライブラリに DL"
        >
          {dlPending ? '⏳ DL 起動中…' : '⬇ ライブラリに DL'}
        </button>
        <MylistAddButton
          video={{
            videoId: payload.video.id,
            title: payload.video.title,
            thumbnailUrl: payload.video.thumbnailUrl,
            lengthSeconds: payload.video.duration,
            viewCounter: payload.video.viewCount,
            uploaderName: payload.owner?.nickname,
          }}
        />
      </div>
    {/if}
  </div>
  {#if dlMsg}
    <div class="dl-msg {dlMsg.kind}">{dlMsg.text}</div>
  {/if}

  {#if pending}
    <div class="muted">読み込み中…</div>
  {:else if error}
    <div class="error">エラー: {error}</div>
    <p class="muted">
      ログインが必要な動画の場合は <a href="/settings">設定</a> で <code>user_session</code> Cookie
      を入れてください。 DL 済みなら <a href={`/library/${videoId}`}>ローカル再生</a> に切り替えてください。
    </p>
  {:else if payload}
    {@const p = payload}
    <div class="player-row" class:dragging>
      <div class="player-col">
        <Player
          bind:this={playerRef}
          hlsUrl={p.hlsUrl}
          comments={visibleComments}
          refreshHlsUrl={() => issueHlsUrl(p.videoId)}
          onTime={handleTimeUpdate}
          resumePosition={getResumePosition(p.videoId)}
          {loop}
        />
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
        {#if commentsLoading}
          <div class="comment-loading">コメント取得中…</div>
        {/if}
        <CommentList comments={visibleComments} {currentTime} onSeek={handleSeek} />
      </div>
    </div>

    <div class="below">
      <div class="meta">
        <div class="row">
          <span>{payload.video.id}</span>
          <span class="dot">·</span>
          <span>{formatDuration(payload.video.duration)}</span>
          {#if payload.video.registeredAt}
            <span class="dot">·</span><span>{formatDate(payload.video.registeredAt)}</span>
          {/if}
          {#if payload.pickedQuality.label}
            <span class="dot">·</span>
            <span class="quality">{payload.pickedQuality.label}</span>
          {/if}
          <span class="dot">·</span>
          <span>コメ {commentsLoading ? '…' : formatNumber(comments.length)}</span>
          <a
            class="external"
            href={videoUrl(payload.video.id)}
            target="_blank"
            rel="noreferrer noopener">ニコニコで開く ↗</a
          >
        </div>
        {#if payload.owner}
          <div class="row owner">
            {#if payload.owner.iconUrl}
              <img src={payload.owner.iconUrl} alt="" loading="lazy" />
            {/if}
            {#if payload.owner.id}
              <a
                href={`/user/${payload.owner.id}?kind=${payload.owner.kind}${payload.owner.nickname ? `&name=${encodeURIComponent(payload.owner.nickname)}` : ''}${payload.owner.iconUrl ? `&icon=${encodeURIComponent(payload.owner.iconUrl)}` : ''}`}
                class="owner-link"
              >
                <span>{payload.owner.nickname ?? '不明'}</span>
              </a>
            {:else}
              <span>{payload.owner.nickname ?? '不明'}</span>
            {/if}
            <span class="muted">({payload.owner.kind})</span>
          </div>
        {/if}
        {#if payload.video.tags && payload.video.tags.length > 0}
          <div class="tags" aria-label="タグ">
            {#each payload.video.tags as tag (tag.name)}
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
        {#if payload.video.description}
          <details>
            <summary>説明文</summary>
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            <p class="desc">{@html payload.video.description}</p>
          </details>
        {/if}
      </div>

      <aside class="related">
        <h3>関連動画</h3>
        {#if relatedLoading}
          <div class="muted small">関連動画を取得中…</div>
        {:else if relatedError}
          <div class="muted small">取得失敗: {relatedError}</div>
        {:else if related.length === 0}
          <div class="muted small">関連動画は見つかりませんでした。</div>
        {:else}
          <ul class="related-list">
            {#each related.slice(0, relatedVisibleCount) as hit (hit.contentId)}
              <SearchHitCard {hit} compact />
            {/each}
          </ul>
        {/if}
      </aside>
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
  .head-actions {
    flex-shrink: 0;
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .dl-btn {
    background: #1a3a26;
    color: #b3f5b3;
    border: 1px solid #2a5a3a;
    padding: 6px 14px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
  }
  .dl-btn:hover:not(:disabled) {
    background: #2a5a3a;
    color: #fff;
  }
  .dl-btn:disabled {
    opacity: 0.6;
    cursor: wait;
  }
  .dl-msg {
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 12px;
    margin-bottom: 8px;
  }
  .dl-msg.ok {
    background: #102d20;
    border: 1px solid #1e6b48;
    color: #bbf7d0;
  }
  .dl-msg.error {
    background: #2a1212;
    border: 1px solid #5a2222;
    color: #f5b3b3;
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
  .muted {
    color: #9a9a9a;
  }
  .small {
    font-size: 12px;
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
  .comment-loading {
    position: absolute;
    top: 42px;
    left: 12px;
    z-index: 1;
    color: #9a9a9a;
    font-size: 12px;
  }
  .below {
    display: grid;
    grid-template-columns: 1fr 360px;
    gap: 16px;
    margin-top: 12px;
    contain: layout style;
  }
  @media (max-width: 1100px) {
    .below {
      grid-template-columns: 1fr;
    }
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
  .quality {
    background: #1f2a44;
    color: #93c5fd;
    border-radius: 999px;
    padding: 0 8px;
    font-size: 11px;
  }
  .external {
    margin-left: auto;
    color: #6ea8fe;
    text-decoration: none;
  }
  .external:hover {
    text-decoration: underline;
  }
  .owner img {
    width: 24px;
    height: 24px;
    border-radius: 999px;
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
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 8px;
    min-width: 0;
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
    border: 1px solid transparent;
  }
  .tag:hover {
    background: #2a2a2a;
    color: #fff;
    border-color: #3a3a3a;
  }
  .tag.locked {
    background: #1e2a3a;
    color: #b3c5ff;
  }
  .tag.locked:hover {
    background: #243246;
    color: #fff;
  }
  .lock {
    font-size: 9px;
    opacity: 0.7;
  }
  .related h3 {
    margin: 0 0 8px;
    font-size: 14px;
    color: #cfcfcf;
  }
  .related-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  /* 
   * 全画面表示時に裏側でレイアウト計算（特に関連動画の遅延読み込み画像など）
   * が走って動画がガクつくのを防ぐため、プレーヤー以外を非表示にする
   */
  :global(body:has(:fullscreen)) .head,
  :global(body:has(:fullscreen)) .divider,
  :global(body:has(:fullscreen)) .comment-panel,
  :global(body:has(:fullscreen)) .below {
    display: none !important;
  }
</style>
