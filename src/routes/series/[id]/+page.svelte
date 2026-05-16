<script lang="ts">
  import { page } from '$app/state';
  import { fetchSeriesVideos, type UserVideoItem } from '$lib/api';
  import { formatDate, formatDuration, formatNumber } from '$lib/format';

  let seriesId = $derived(page.params.id ?? '');

  let items = $state<UserVideoItem[]>([]);
  let totalCount = $state(0);
  let loading = $state(true);
  let error = $state<string | null>(null);

  let seriesTitle = $state('');
  let seriesDescription = $state('');
  let seriesThumbnailUrl = $state('');

  function videoHref(id: string): string {
    return `/video/${id}?from=series&seriesId=${encodeURIComponent(seriesId)}&seriesTitle=${encodeURIComponent(seriesTitle)}`;
  }

  async function load() {
    if (!seriesId) return;
    loading = true;
    error = null;
    try {
      const resp = await fetchSeriesVideos(seriesId, 1, 100);
      items = resp.items;
      totalCount = resp.totalCount;
      seriesTitle = resp.seriesTitle ?? '';
      seriesDescription = resp.seriesDescription ?? '';
      seriesThumbnailUrl = resp.seriesThumbnailUrl ?? '';
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void seriesId;
    load();
  });
</script>

<section class="page">
  <header class="header">
    <div class="series-thumb">
      {#if seriesThumbnailUrl}
        <img src={seriesThumbnailUrl} alt="" loading="lazy" />
      {:else}
        <div class="series-thumb-placeholder">
          <svg viewBox="0 0 24 24" width="32" height="32"
            ><path
              d="M4 6H2v14c0 1.1.9 2 2 2h14v-2H4V6zm16-4H8c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm0 14H8V4h12v12zm-8-2l6-4-6-4v8z"
              fill="currentColor"
            /></svg
          >
        </div>
      {/if}
    </div>
    <div class="info">
      <div class="label">シリーズ</div>
      <h1 class="title">{seriesTitle || seriesId}</h1>
      {#if seriesDescription}
        <p class="desc">{seriesDescription}</p>
      {/if}
      <p class="count">{totalCount} 本の動画</p>
    </div>
  </header>

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
  {/if}
</section>

<style>
  .page {
    max-width: 800px;
    margin: 0 auto;
    padding: 24px 16px;
  }
  .header {
    display: flex;
    gap: 16px;
    margin-bottom: 24px;
    padding-bottom: 20px;
    border-bottom: 1px solid #2a2a2a;
  }
  .series-thumb img {
    width: 160px;
    height: 90px;
    object-fit: cover;
    border-radius: 6px;
    background: #0a0a0a;
  }
  .series-thumb-placeholder {
    width: 160px;
    height: 90px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #1a2235;
    border: 1px dashed #2a4a6a;
    border-radius: 6px;
    color: #4a7ab5;
  }
  .header .info {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .label {
    font-size: 11px;
    color: #6ea8fe;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-weight: 600;
  }
  .title {
    font-size: 20px;
    font-weight: 700;
    color: #eaeaea;
    margin: 0;
  }
  .desc {
    font-size: 13px;
    color: #9a9a9a;
    margin: 0;
    line-height: 1.5;
  }
  .count {
    font-size: 12px;
    color: #7a8a9a;
    margin: 0;
  }
  .muted {
    color: #9a9a9a;
    font-size: 13px;
    text-align: center;
    padding: 40px 0;
  }
  .error {
    color: #f87171;
    font-size: 13px;
    text-align: center;
    padding: 40px 0;
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
  .hit .info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .title {
    font-size: 14px;
    font-weight: 600;
  }
  .title a {
    color: #eaeaea;
    text-decoration: none;
  }
  .title a:hover {
    color: #93c5fd;
  }
  .row-meta {
    font-size: 12px;
    display: flex;
    align-items: center;
    gap: 4px;
    color: #cfcfcf;
  }
  .dot {
    color: #555;
  }
</style>
