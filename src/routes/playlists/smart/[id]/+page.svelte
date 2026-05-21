<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { queryLibraryVideos, type LibraryVideoRow } from '$lib/api';
  import {
    filterToQueryParams,
    getSmartPlaylist,
    summarizeFilter,
    type SmartPlaylist,
  } from '$lib/stores/smartPlaylists';
  import { setQueue, itemHref, type PlaybackQueueItem } from '$lib/stores/playbackQueue';
  import { formatDate, formatDuration, formatNumber } from '$lib/format';

  let smartId = $derived(page.params.id ?? '');

  let smart = $state<SmartPlaylist | null>(null);
  let items = $state<LibraryVideoRow[]>([]);
  let totalCount = $state(0);
  let loading = $state(true);
  let error = $state<string | null>(null);

  async function refresh() {
    if (!smartId) return;
    loading = true;
    error = null;
    const sp = getSmartPlaylist(smartId);
    if (!sp) {
      error = `スマートプレイリスト ${smartId} が見つかりません`;
      smart = null;
      items = [];
      totalCount = 0;
      loading = false;
      return;
    }
    smart = sp;
    try {
      const params = filterToQueryParams(sp.filter);
      const result = await queryLibraryVideos(params);
      items = result.items;
      totalCount = result.totalCount;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  // `$effect` は初回マウント時にも 1 度発火するので `onMount(refresh)` を
  // 別途付けるのは二重呼び出しになる (queryLibraryVideos の IPC が 1 ページ
  // ロードで 2 回走る)。effect 単発に揃える (series/[id], library/[id] と同パターン)。
  $effect(() => {
    void smartId;
    void refresh();
  });

  function toQueueItems(): PlaybackQueueItem[] {
    return items.map((it) => ({
      videoId: it.id,
      title: it.title,
      thumbnailUrl: it.thumbnailUrl ?? undefined,
      lengthSeconds: it.durationSec,
      source: 'local',
    }));
  }

  function startPlayAll(startIndex = 0) {
    if (!smart) return;
    const queueItems = toQueueItems();
    if (queueItems.length === 0) return;
    const idx = Math.max(0, Math.min(queueItems.length - 1, startIndex));
    setQueue('smart', smart.id, smart.name, queueItems, idx);
    void goto(itemHref(queueItems[idx]));
  }

  function thumbSrc(item: LibraryVideoRow): string | undefined {
    if (item.localThumbnailPath) return convertFileSrc(item.localThumbnailPath);
    return item.thumbnailUrl ?? undefined;
  }

  function relativeDate(unix: number | null): string {
    if (!unix) return '';
    const d = new Date(unix * 1000);
    return d.toLocaleDateString('ja-JP', { year: 'numeric', month: '2-digit', day: '2-digit' });
  }
</script>

<section class="page">
  <header class="head">
    <a class="back" href="/playlists?tab=smart">← スマートプレイリスト一覧へ</a>
    <h2>{smart?.name ?? smartId}</h2>
  </header>

  {#if smart}
    <div class="meta">
      {#if smart.description}
        <p class="desc">{smart.description}</p>
      {/if}
      <p class="summary">条件: <span class="summary-val">{summarizeFilter(smart.filter)}</span></p>
      <p class="updated">
        作成 {formatDate(new Date(smart.createdAt).toISOString())}
        · 更新 {formatDate(new Date(smart.updatedAt).toISOString())}
        · {totalCount} 件ヒット
      </p>
      <div class="actions">
        <button
          type="button"
          class="play-all"
          disabled={items.length === 0}
          onclick={() => startPlayAll(0)}
        >
          ▶ 連続再生
        </button>
        <a class="edit-link" href={`/playlists?tab=smart&edit=${encodeURIComponent(smart.id)}`}
          >編集</a
        >
      </div>
    </div>
  {/if}

  {#if loading}
    <div class="muted">読み込み中…</div>
  {:else if error}
    <div class="error">エラー: {error}</div>
  {:else if items.length === 0}
    <div class="empty">
      <p class="muted">条件にマッチする動画がありません。</p>
      <p class="muted">
        ライブラリに動画を DL するか、<a href="/playlists?tab=smart">条件を見直して</a>ください。
      </p>
    </div>
  {:else}
    <div class="grid">
      {#each items as item, i (item.id)}
        <button type="button" class="card" onclick={() => startPlayAll(i)} title="ここから連続再生">
          <div class="thumb-wrap">
            {#if thumbSrc(item)}
              <img class="thumb" src={thumbSrc(item)} alt="" loading="lazy" />
            {:else}
              <div class="thumb-placeholder">?</div>
            {/if}
            <span class="duration">{formatDuration(item.durationSec)}</span>
            {#if i === 0}
              <span class="start-badge">先頭から再生</span>
            {/if}
          </div>
          <div class="meta-row">
            <h3 class="title" title={item.title}>{item.title}</h3>
            <div class="row muted">
              {#if item.uploaderName}<span>{item.uploaderName}</span>{/if}
              {#if item.viewCount != null}
                <span class="dot">·</span><span>{formatNumber(item.viewCount)} 再生</span>
              {/if}
            </div>
            <div class="row muted small">
              <span>DL {relativeDate(item.downloadedAt)}</span>
              {#if item.resolution}
                <span class="dot">·</span><span>{item.resolution}</span>
              {/if}
            </div>
          </div>
        </button>
      {/each}
    </div>
  {/if}
</section>

<style>
  .page {
    max-width: 1400px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 16px;
    flex-wrap: wrap;
  }
  .head h2 {
    margin: 0;
  }
  .back {
    color: var(--theme-text-muted);
    text-decoration: none;
    font-size: 13px;
  }
  .back:hover {
    color: var(--theme-text);
    text-decoration: underline;
  }
  .meta {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 8px;
    padding: 12px 16px;
    margin-bottom: 16px;
  }
  .desc {
    margin: 0 0 6px;
    font-size: 13px;
  }
  .summary {
    margin: 6px 0;
    font-size: 12px;
    color: var(--theme-text-muted);
  }
  .summary-val {
    color: var(--theme-text-soft);
  }
  .updated {
    margin: 4px 0 10px;
    font-size: 11px;
    color: var(--theme-text-muted);
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .play-all {
    background: var(--theme-accent);
    color: #fff;
    border: 1px solid var(--theme-accent);
    border-radius: 6px;
    padding: 6px 14px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }
  .play-all:hover:not(:disabled) {
    background: var(--theme-accent-hover);
  }
  .play-all:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .edit-link {
    color: var(--theme-text-muted);
    border: 1px solid var(--theme-border-strong);
    border-radius: 6px;
    padding: 6px 14px;
    font-size: 13px;
    text-decoration: none;
  }
  .edit-link:hover {
    background: var(--theme-surface-hover);
    color: var(--theme-text);
  }
  .muted {
    color: var(--theme-text-muted);
  }
  .small {
    font-size: 11px;
  }
  .error {
    background: var(--theme-danger-bg);
    border: 1px solid var(--theme-danger-border);
    color: var(--theme-danger-text);
    padding: 8px 12px;
    border-radius: 6px;
    margin-bottom: 12px;
  }
  .empty {
    padding: 32px;
    text-align: center;
    border: 1px dashed var(--theme-border-strong);
    border-radius: 8px;
  }
  .empty a {
    color: var(--theme-accent-soft);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 14px;
  }
  .card {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 8px;
    overflow: hidden;
    padding: 0;
    cursor: pointer;
    color: inherit;
    text-align: left;
    font: inherit;
  }
  .card:hover {
    background: var(--theme-surface-4);
    border-color: var(--theme-border-focus);
  }
  .thumb-wrap {
    position: relative;
    aspect-ratio: 16 / 9;
    background: var(--theme-bg);
  }
  .thumb {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .thumb-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    color: var(--theme-text-faint);
    font-size: 32px;
  }
  .duration {
    position: absolute;
    right: 6px;
    bottom: 6px;
    background: rgba(0, 0, 0, 0.78);
    color: var(--theme-text);
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .start-badge {
    position: absolute;
    left: 6px;
    top: 6px;
    background: var(--theme-accent);
    color: #fff;
    padding: 1px 8px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 600;
  }
  .meta-row {
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .title {
    font-size: 13px;
    margin: 0;
    line-height: 1.3;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }
  .dot {
    color: var(--theme-text-faint);
  }
</style>
