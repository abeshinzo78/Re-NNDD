<script lang="ts">
  import { page } from '$app/state';
  import {
    fetchUserVideos,
    fetchUserMylists,
    fetchUserSeriesList,
    fetchMylistVideos,
    fetchSeriesVideos,
    type UserVideoItem,
    type UserMylistSummary,
    type UserSeriesSummary,
  } from '$lib/api';
  import { formatDate, formatDuration, formatNumber, videoUrl } from '$lib/format';

  let userId = $derived(page.params.id ?? '');
  let kind = $derived<'user' | 'channel'>(
    page.url.searchParams.get('kind') === 'channel' ? 'channel' : 'user',
  );
  let nickname = $derived(page.url.searchParams.get('name') ?? '');
  let iconUrl = $derived(page.url.searchParams.get('icon') || null);

  type Tab = 'videos' | 'mylists' | 'series';
  let activeTab = $state<Tab>('videos');

  // ---- 投稿動画 ----
  let items = $state<UserVideoItem[]>([]);
  let totalCount = $state(0);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let currentPage = $state(1);
  let loadingMore = $state(false);

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
      currentPage = (reset ? 1 : currentPage) + 1;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
      loadingMore = false;
    }
  }

  // ---- マイリスト一覧 ----
  let mylists = $state<UserMylistSummary[]>([]);
  let mylistsLoading = $state(false);
  let mylistsError = $state<string | null>(null);

  // 展開中のマイリスト ID とその動画一覧
  let expandedMylistId = $state<string | null>(null);
  let mylistVideos = $state<UserVideoItem[]>([]);
  let mylistVideosLoading = $state(false);

  async function loadMylists() {
    if (!userId || kind === 'channel') return;
    mylistsLoading = true;
    mylistsError = null;
    try {
      const resp = await fetchUserMylists(userId);
      mylists = resp.items;
    } catch (e) {
      mylistsError = String(e);
    } finally {
      mylistsLoading = false;
    }
  }

  // ---- シリーズ一覧 ----
  let seriesList = $state<UserSeriesSummary[]>([]);
  let seriesLoading = $state(false);
  let seriesError = $state<string | null>(null);

  let expandedSeriesId = $state<string | null>(null);
  let seriesVideos = $state<UserVideoItem[]>([]);
  let seriesVideosLoading = $state(false);

  async function loadSeries() {
    if (!userId || kind === 'channel') return;
    seriesLoading = true;
    seriesError = null;
    try {
      const resp = await fetchUserSeriesList(userId);
      seriesList = resp.items;
    } catch (e) {
      seriesError = String(e);
    } finally {
      seriesLoading = false;
    }
  }

  // ---- タブ切替時のデータロード ----
  $effect(() => {
    void [userId, kind, sortKey, sortOrder];
    if (activeTab === 'videos') {
      loadVideos(true);
    }
  });

  $effect(() => {
    if (activeTab === 'mylists' && mylists.length === 0 && !mylistsLoading) {
      loadMylists();
    }
  });

  $effect(() => {
    if (activeTab === 'series' && seriesList.length === 0 && !seriesLoading) {
      loadSeries();
    }
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

  async function toggleMylist(id: string) {
    if (expandedMylistId === id) {
      expandedMylistId = null;
      mylistVideos = [];
      return;
    }
    expandedMylistId = id;
    mylistVideosLoading = true;
    try {
      const resp = await fetchMylistVideos(id, 1, 100);
      mylistVideos = resp.items;
    } catch {
      mylistVideos = [];
    } finally {
      mylistVideosLoading = false;
    }
  }

  async function toggleSeries(id: string) {
    if (expandedSeriesId === id) {
      expandedSeriesId = null;
      seriesVideos = [];
      return;
    }
    expandedSeriesId = id;
    seriesVideosLoading = true;
    try {
      const resp = await fetchSeriesVideos(id, 1, 100);
      seriesVideos = resp.items;
    } catch {
      seriesVideos = [];
    } finally {
      seriesVideosLoading = false;
    }
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

  {#if kind !== 'channel'}
    <nav class="tabs">
      <button
        class="tab"
        class:active={activeTab === 'videos'}
        onclick={() => (activeTab = 'videos')}
      >
        投稿動画
      </button>
      <button
        class="tab"
        class:active={activeTab === 'mylists'}
        onclick={() => (activeTab = 'mylists')}
      >
        マイリスト
      </button>
      <button
        class="tab"
        class:active={activeTab === 'series'}
        onclick={() => (activeTab = 'series')}
      >
        シリーズ
      </button>
    </nav>
  {/if}

  <!-- ===== 投稿動画タブ ===== -->
  {#if activeTab === 'videos'}
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
                {#if item.startTime}<span class="dot">·</span><span
                    >{formatDate(item.startTime)}</span
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
  {/if}

  <!-- ===== マイリストタブ ===== -->
  {#if activeTab === 'mylists'}
    {#if mylistsLoading}
      <div class="muted">読み込み中…</div>
    {:else if mylistsError}
      <div class="error">エラー: {mylistsError}</div>
    {:else if mylists.length === 0}
      <div class="muted">公開マイリストはありません。</div>
    {:else}
      <div class="card-list">
        {#each mylists as ml (ml.id)}
          <div
            class="list-card"
            role="button"
            tabindex="0"
            onclick={() => toggleMylist(ml.id)}
            onkeydown={(e) => e.key === 'Enter' && toggleMylist(ml.id)}
          >
            <div class="list-card-thumb">
              {#if ml.thumbnailUrl}
                <img src={ml.thumbnailUrl} alt="" loading="lazy" />
              {:else}
                <div class="list-card-thumb placeholder">
                  <svg viewBox="0 0 24 24" width="24" height="24"
                    ><path
                      d="M4 6H2v14c0 1.1.9 2 2 2h14v-2H4V6zm16-4H8c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm0 14H8V4h12v12z"
                      fill="currentColor"
                    /></svg
                  >
                </div>
              {/if}
            </div>
            <div class="list-card-body">
              <div class="list-card-title">{ml.name}</div>
              {#if ml.description}
                <div class="list-card-desc">{ml.description}</div>
              {/if}
              <div class="list-card-meta">
                {#if ml.itemsCount != null}{ml.itemsCount} 本の動画{/if}
              </div>
            </div>
            <span class="list-card-arrow" aria-hidden="true"
              >{expandedMylistId === ml.id ? '▾' : '›'}</span
            >
          </div>
          {#if expandedMylistId === ml.id}
            <div class="expanded-list">
              {#if mylistVideosLoading}
                <div class="muted">読み込み中…</div>
              {:else if mylistVideos.length === 0}
                <div class="muted">動画が見つかりませんでした。</div>
              {:else}
                <ul class="results">
                  {#each mylistVideos as item (item.contentId)}
                    <li class="hit compact">
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
                        </div>
                        <div class="row-meta muted">
                          {#if item.lengthSeconds != null}<span
                              >{formatDuration(item.lengthSeconds)}</span
                            >{/if}
                          {#if item.startTime}<span class="dot">·</span><span
                              >{formatDate(item.startTime)}</span
                            >{/if}
                        </div>
                      </div>
                    </li>
                  {/each}
                </ul>
              {/if}
            </div>
          {/if}
        {/each}
      </div>
    {/if}
  {/if}

  <!-- ===== シリーズタブ ===== -->
  {#if activeTab === 'series'}
    {#if seriesLoading}
      <div class="muted">読み込み中…</div>
    {:else if seriesError}
      <div class="error">エラー: {seriesError}</div>
    {:else if seriesList.length === 0}
      <div class="muted">シリーズはありません。</div>
    {:else}
      <div class="card-list">
        {#each seriesList as sr (sr.id)}
          <div
            class="list-card"
            role="button"
            tabindex="0"
            onclick={() => toggleSeries(sr.id)}
            onkeydown={(e) => e.key === 'Enter' && toggleSeries(sr.id)}
          >
            <div class="list-card-thumb">
              {#if sr.thumbnailUrl}
                <img src={sr.thumbnailUrl} alt="" loading="lazy" />
              {:else}
                <div class="list-card-thumb placeholder">
                  <svg viewBox="0 0 24 24" width="24" height="24"
                    ><path
                      d="M4 6H2v14c0 1.1.9 2 2 2h14v-2H4V6zm16-4H8c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm0 14H8V4h12v12zm-8-2l6-4-6-4v8z"
                      fill="currentColor"
                    /></svg
                  >
                </div>
              {/if}
            </div>
            <div class="list-card-body">
              <div class="list-card-label">シリーズ</div>
              <div class="list-card-title">{sr.title}</div>
              {#if sr.description}
                <div class="list-card-desc">{sr.description}</div>
              {/if}
              <div class="list-card-meta">
                {#if sr.itemsCount != null}{sr.itemsCount} 本の動画{/if}
              </div>
            </div>
            <span class="list-card-arrow" aria-hidden="true"
              >{expandedSeriesId === sr.id ? '▾' : '›'}</span
            >
          </div>
          {#if expandedSeriesId === sr.id}
            <div class="expanded-list">
              {#if seriesVideosLoading}
                <div class="muted">読み込み中…</div>
              {:else if seriesVideos.length === 0}
                <div class="muted">動画が見つかりませんでした。</div>
              {:else}
                <ul class="results">
                  {#each seriesVideos as item (item.contentId)}
                    <li class="hit compact">
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
                        </div>
                        <div class="row-meta muted">
                          {#if item.lengthSeconds != null}<span
                              >{formatDuration(item.lengthSeconds)}</span
                            >{/if}
                          {#if item.startTime}<span class="dot">·</span><span
                              >{formatDate(item.startTime)}</span
                            >{/if}
                        </div>
                      </div>
                    </li>
                  {/each}
                </ul>
              {/if}
            </div>
          {/if}
        {/each}
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

  /* ---- Tabs ---- */
  .tabs {
    display: flex;
    gap: 2px;
    margin-bottom: 14px;
    border-bottom: 1px solid #2a2a2a;
  }
  .tab {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: #9a9a9a;
    padding: 8px 16px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
    margin-bottom: -1px;
  }
  .tab:hover {
    color: #eaeaea;
  }
  .tab.active {
    color: #93c5fd;
    border-bottom-color: #93c5fd;
  }

  /* ---- Toolbar ---- */
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

  /* ---- Video list ---- */
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
  .hit.compact {
    grid-template-columns: 120px 1fr;
    gap: 8px;
    padding: 6px;
  }
  .thumb {
    width: 160px;
    height: 90px;
    object-fit: cover;
    background: #0a0a0a;
    border-radius: 4px;
  }
  .compact .thumb {
    width: 120px;
    height: 68px;
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
  .compact .row-meta {
    font-size: 11px;
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

  /* ---- Mylist / Series card list ---- */
  .card-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .list-card {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    background: #161616;
    border: 1px solid #1f1f1f;
    border-radius: 8px;
    cursor: pointer;
    transition:
      background 0.15s,
      border-color 0.15s;
  }
  .list-card:hover {
    background: #1c1c1c;
    border-color: #2a2a2a;
  }
  .list-card:focus-visible {
    outline: 2px solid #3a5a8a;
    outline-offset: -2px;
  }
  .list-card-thumb {
    flex-shrink: 0;
    line-height: 0;
  }
  .list-card-thumb img {
    width: 96px;
    height: 54px;
    object-fit: cover;
    border-radius: 4px;
    background: #0a0a0a;
  }
  .list-card-thumb.placeholder {
    width: 96px;
    height: 54px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #1a2235;
    border: 1px dashed #2a4a6a;
    border-radius: 4px;
    color: #4a7ab5;
  }
  .list-card-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .list-card-label {
    font-size: 10px;
    color: #6ea8fe;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-weight: 600;
  }
  .list-card-title {
    font-size: 14px;
    font-weight: 600;
    color: #eaeaea;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .list-card-desc {
    font-size: 11px;
    color: #9a9a9a;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .list-card-meta {
    font-size: 11px;
    color: #7a8a9a;
  }
  .list-card-arrow {
    flex-shrink: 0;
    font-size: 18px;
    color: #555;
    margin-left: 4px;
  }
  .list-card:hover .list-card-arrow {
    color: #6ea8fe;
  }

  /* ---- Expanded video list ---- */
  .expanded-list {
    margin: 0 0 4px 12px;
    padding-left: 12px;
    border-left: 2px solid #2a3a4a;
  }
</style>
