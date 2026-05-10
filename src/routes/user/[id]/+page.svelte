<script lang="ts">
  import { page } from '$app/state';
  import { fetchUserVideos, type UserVideoItem } from '$lib/api';
  import { formatDate, formatDuration, formatNumber, videoUrl } from '$lib/format';

  let userId = $derived(page.params.id ?? '');
  let kind = $derived<'user' | 'channel'>(
    page.url.searchParams.get('kind') === 'channel' ? 'channel' : 'user',
  );
  let nickname = $derived(page.url.searchParams.get('name') ?? '');
  let iconUrl = $derived(page.url.searchParams.get('icon') || null);

  let items = $state<UserVideoItem[]>([]);
  let totalCount = $state(0);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let currentPage = $state(1);
  let loadingMore = $state(false);
  let debugRaw = $state<string | null>(null);

  const PAGE_SIZE = 30;

  type SortKey = 'registeredAt' | 'viewCount' | 'mylistCount';
  type SortOrder = 'desc' | 'asc';
  let sortKey = $state<SortKey>('registeredAt');
  let sortOrder = $state<SortOrder>('desc');

  async function loadVideos(reset = false) {
    if (!userId) return;
    if (reset) {
      loading = true;
      error = null;
      items = [];
      currentPage = 1;
    } else {
      loadingMore = true;
    }

    try {
      const resp = await fetchUserVideos(
        kind,
        userId,
        reset ? 1 : currentPage,
        PAGE_SIZE,
        sortKey,
        sortOrder,
      );
      if (reset) {
        items = resp.items;
      } else {
        items = [...items, ...resp.items];
      }
      totalCount = resp.totalCount;
      debugRaw = resp.debugRaw ?? null;
      currentPage = (reset ? 1 : currentPage) + 1;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
      loadingMore = false;
    }
  }

  $effect(() => {
    // Track reactive deps so the effect re-runs when any of these change.
    void [userId, kind, sortKey, sortOrder];
    loadVideos(true);
  });

  let externalHref = $derived(
    kind === 'channel'
      ? `https://ch.nicovideo.jp/ch${userId}`
      : `https://www.nicovideo.jp/user/${userId}`,
  );

  function changeSort(field: SortKey) {
    if (sortKey === field) {
      sortOrder = sortOrder === 'desc' ? 'asc' : 'desc';
    } else {
      sortKey = field;
      sortOrder = 'desc';
    }
  }

  function videoHref(id: string): string {
    let qs = `from=user&uid=${encodeURIComponent(userId)}&kind=${encodeURIComponent(kind)}`;
    if (nickname) qs += `&name=${encodeURIComponent(nickname)}`;
    if (iconUrl) qs += `&icon=${encodeURIComponent(iconUrl)}`;
    return `/video/${id}?${qs}`;
  }
</script>

<section class="page">
  <header class="header">
    <div class="identity">
      {#if iconUrl}
        <img class="icon" src={iconUrl} alt="" />
      {:else}
        <div class="icon placeholder">{kind === 'channel' ? 'CH' : 'U'}</div>
      {/if}
      <div>
        <h2>{nickname || (kind === 'channel' ? `ch${userId}` : `user/${userId}`)}</h2>
        <span class="muted">{kind === 'channel' ? 'チャンネル' : 'ユーザー'} · {userId}</span>
        {#if totalCount > 0}
          <span class="muted"> · {formatNumber(totalCount)} 件の動画</span>
        {/if}
      </div>
    </div>
    <a class="external" href={externalHref} target="_blank" rel="noreferrer noopener">
      ニコニコで開く ↗
    </a>
  </header>

  <div class="toolbar">
    <button
      class="sort-btn"
      class:active={sortKey === 'registeredAt'}
      onclick={() => changeSort('registeredAt')}
    >
      投稿日 {sortKey === 'registeredAt' ? (sortOrder === 'desc' ? '↓' : '↑') : ''}
    </button>
    <button
      class="sort-btn"
      class:active={sortKey === 'viewCount'}
      onclick={() => changeSort('viewCount')}
    >
      再生数 {sortKey === 'viewCount' ? (sortOrder === 'desc' ? '↓' : '↑') : ''}
    </button>
    <button
      class="sort-btn"
      class:active={sortKey === 'mylistCount'}
      onclick={() => changeSort('mylistCount')}
    >
      マイリスト {sortKey === 'mylistCount' ? (sortOrder === 'desc' ? '↓' : '↑') : ''}
    </button>
  </div>

  {#if loading}
    <div class="muted">読み込み中…</div>
  {:else if error}
    <div class="error">エラー: {error}</div>
  {:else if items.length === 0}
    <div class="muted">動画が見つかりませんでした。</div>
    {#if debugRaw}
      <details class="debug-details">
        <summary>API レスポンス (デバッグ)</summary>
        <pre class="debug-pre">{debugRaw}</pre>
      </details>
    {/if}
  {:else}
    <ul class="results">
      {#each items as item (item.contentId)}
        <li class="hit">
          {#if item.thumbnailUrl}
            <a href={videoHref(item.contentId)}>
              <img class="thumb" src={item.thumbnailUrl} alt="" loading="lazy" />
            </a>
          {:else}
            <div class="thumb placeholder"></div>
          {/if}
          <div class="info">
            <div class="title">
              <a href={videoHref(item.contentId)}>{item.title || '(無題)'}</a>
              <a
                class="ext"
                href={videoUrl(item.contentId)}
                target="_blank"
                rel="noreferrer noopener"
                title="ニコニコで開く">↗</a
              >
            </div>
            <div class="row-meta muted">
              <span>{item.contentId}</span>
              {#if item.lengthSeconds != null}<span class="dot">·</span><span
                  >{formatDuration(item.lengthSeconds)}</span
                >{/if}
              {#if item.startTime}<span class="dot">·</span><span>{formatDate(item.startTime)}</span
                >{/if}
            </div>
            <div class="row-meta">
              <span>再生 {formatNumber(item.viewCounter)}</span>
              <span class="dot">·</span>
              <span>コメ {formatNumber(item.commentCounter)}</span>
              <span class="dot">·</span>
              <span>マイリスト {formatNumber(item.mylistCounter)}</span>
            </div>
          </div>
        </li>
      {/each}
    </ul>
    {#if items.length < totalCount}
      <div class="more">
        <button class="more-btn" onclick={() => loadVideos(false)} disabled={loadingMore}>
          {loadingMore ? '読み込み中…' : 'もっと見る'}
        </button>
      </div>
    {/if}
  {/if}
</section>

<style>
  .page {
    max-width: 1200px;
  }
  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
    gap: 12px;
  }
  .identity {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .icon {
    width: 56px;
    height: 56px;
    border-radius: 999px;
    background: #1a1a1a;
    flex-shrink: 0;
  }
  .icon.placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    color: #666;
    font-weight: 600;
    font-size: 18px;
    background: #1a1a1a;
    border: 1px solid #2a2a2a;
  }
  h2 {
    margin: 0;
    font-size: 20px;
  }
  .muted {
    color: #9a9a9a;
    font-size: 13px;
  }
  .external {
    color: #6ea8fe;
    text-decoration: none;
    font-size: 13px;
    flex-shrink: 0;
  }
  .external:hover {
    text-decoration: underline;
  }
  .toolbar {
    display: flex;
    gap: 6px;
    margin-bottom: 12px;
  }
  .sort-btn {
    background: #1a1a1a;
    border: 1px solid #2a2a2a;
    color: #b0b0b0;
    padding: 4px 12px;
    border-radius: 6px;
    font-size: 12px;
    cursor: pointer;
  }
  .sort-btn:hover {
    background: #222;
    color: #eaeaea;
  }
  .sort-btn.active {
    background: #1f2a44;
    border-color: #3a5a8a;
    color: #93c5fd;
  }
  .error {
    background: #2a1212;
    border: 1px solid #5a2222;
    color: #f5b3b3;
    padding: 10px 12px;
    border-radius: 6px;
    font-size: 13px;
  }
  .results {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .hit {
    display: grid;
    grid-template-columns: 160px 1fr;
    gap: 12px;
    padding: 8px;
    background: #161616;
    border: 1px solid #1f1f1f;
    border-radius: 8px;
  }
  .thumb {
    width: 160px;
    height: 90px;
    object-fit: cover;
    background: #0a0a0a;
    border-radius: 4px;
  }
  .thumb.placeholder {
    border: 1px dashed #2a2a2a;
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .title {
    font-weight: 600;
  }
  .title a {
    color: #eaeaea;
    text-decoration: none;
  }
  .title a:hover {
    text-decoration: underline;
  }
  .title .ext {
    color: #6ea8fe;
    margin-left: 6px;
    font-weight: 400;
    text-decoration: none;
  }
  .title .ext:hover {
    text-decoration: underline;
  }
  .row-meta {
    font-size: 12px;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    align-items: center;
    color: #cfcfcf;
  }
  .dot {
    color: #555;
  }
  .more {
    text-align: center;
    margin-top: 12px;
  }
  .more-btn {
    background: #1f1f1f;
    border: 1px solid #2a2a2a;
    color: #cfcfcf;
    padding: 8px 24px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
  }
  .more-btn:hover {
    background: #2a2a2a;
    color: #fff;
  }
  .more-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .debug-details {
    margin-top: 12px;
  }
  .debug-details summary {
    color: #9a9a9a;
    font-size: 12px;
    cursor: pointer;
  }
  .debug-pre {
    background: #111;
    border: 1px solid #2a2a2a;
    padding: 8px;
    border-radius: 4px;
    font-size: 11px;
    color: #b0b0b0;
    overflow-x: auto;
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 400px;
    overflow-y: auto;
  }
</style>
