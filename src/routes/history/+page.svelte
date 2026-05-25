<script lang="ts">
  import { onMount } from 'svelte';
  import {
    clearHistory,
    getHistory,
    removeHistoryItem,
    type HistoryItem,
    type HistorySource,
  } from '$lib/stores/history';
  import { formatDuration } from '$lib/format';
  import VideoActionMenu from '$lib/VideoActionMenu.svelte';

  let history = $state<HistoryItem[]>([]);
  let filter = $state<'all' | HistorySource>('all');
  let filterShort = $state(false);
  let searchQuery = $state('');

  onMount(() => {
    history = getHistory();
  });

  let visible = $derived.by(() => {
    let list = history;
    if (filterShort) {
      list = list.filter((h) => h.videoId.startsWith('ss'));
    }
    if (filter !== 'all') {
      list = list.filter((h) => (h.source ?? 'online') === filter);
    }
    if (searchQuery.trim()) {
      const q = searchQuery.trim().toLowerCase();
      list = list.filter(
        (h) =>
          h.title.toLowerCase().includes(q) ||
          h.videoId.toLowerCase().includes(q) ||
          (h.uploaderName ?? '').toLowerCase().includes(q),
      );
    }
    return list;
  });

  let counts = $derived.by(() => {
    let online = 0;
    let local = 0;
    for (const h of history) {
      if ((h.source ?? 'online') === 'local') local++;
      else online++;
    }
    return { online, local, all: history.length };
  });

  function handleClear() {
    if (confirm('履歴をすべて削除しますか？')) {
      clearHistory();
      history = [];
    }
  }

  function hrefFor(item: HistoryItem): string {
    return (item.source ?? 'online') === 'local'
      ? `/library/${item.videoId}?from=history`
      : `/video/${item.videoId}?from=history`;
  }

  function getResumeSeconds(videoId: string): number {
    try {
      return Number(localStorage.getItem(`resume:${videoId}`)) || 0;
    } catch {
      return 0;
    }
  }

  function resumePercent(videoId: string, duration?: number): number | null {
    if (!duration || duration <= 0) return null;
    const pos = getResumeSeconds(videoId);
    if (pos <= 0) return null;
    const pct = Math.min(100, (pos / duration) * 100);
    return pct < 3 ? null : pct;
  }

  function handleDeleteItem(videoId: string, source?: string) {
    removeHistoryItem(videoId, source as HistorySource | undefined);
    history = getHistory();
  }
</script>

<section>
  <div class="head">
    <h2>再生履歴</h2>
    <div class="head-tools">
      <input
        type="search"
        class="search-box"
        placeholder="動画名で検索…"
        bind:value={searchQuery}
      />
      <div class="tabs" role="tablist" aria-label="履歴フィルタ">
        <button
          type="button"
          role="tab"
          aria-selected={filter === 'all'}
          class:active={filter === 'all'}
          onclick={() => (filter = 'all')}>すべて ({counts.all})</button
        >
        <button
          type="button"
          role="tab"
          aria-selected={filter === 'online'}
          class:active={filter === 'online'}
          onclick={() => (filter = 'online')}>オンライン ({counts.online})</button
        >
        <button
          type="button"
          role="tab"
          aria-selected={filter === 'local'}
          class:active={filter === 'local'}
          onclick={() => (filter = 'local')}>ローカル ({counts.local})</button
        >
        <button
          type="button"
          role="tab"
          aria-selected={filterShort}
          class:active={filterShort}
          onclick={() => (filterShort = !filterShort)}>ショート</button
        >
      </div>
      <button type="button" class="clear-btn" onclick={handleClear} disabled={history.length === 0}>
        履歴をクリア
      </button>
    </div>
  </div>

  {#if visible.length === 0}
    <p class="muted">
      {#if searchQuery.trim()}
        「{searchQuery}」に一致する履歴はありません。
      {:else if filter === 'local'}ローカル再生の履歴はありません。
      {:else if filter === 'online'}オンライン再生の履歴はありません。
      {:else}履歴はありません。
      {/if}
    </p>
  {:else}
    <ul class="list">
      {#each visible as item (item.videoId + '@' + (item.source ?? 'online'))}
        <li class="item">
          <a href={hrefFor(item)} class="thumb-link">
            {#if item.thumbnailUrl}
              <img src={item.thumbnailUrl} alt="" class="thumb" loading="lazy" />
            {:else}
              <div class="thumb placeholder"></div>
            {/if}
            {#if resumePercent(item.videoId, item.duration)}
              {@const pct = resumePercent(item.videoId, item.duration)!}
              <div class="resume-overlay">
                <div class="resume-bar">
                  <div class="resume-bar-inner" style:width="{pct}%"></div>
                </div>
                <span class="resume-time">{formatDuration(getResumeSeconds(item.videoId))}</span>
              </div>
            {/if}
          </a>
          <div class="info">
            <div class="title-row">
              <a href={hrefFor(item)} class="title">{item.title}</a>
              {#if (item.source ?? 'online') === 'local'}
                <span class="src-tag local">ローカル</span>
              {:else}
                <span class="src-tag online">オンライン</span>
              {/if}
            </div>
            <div class="meta muted">
              <span>{item.videoId}</span>
              {#if item.duration}<span class="dot">·</span><span
                  >{formatDuration(item.duration)}</span
                >{/if}
              {#if item.uploaderName}<span class="dot">·</span><span>{item.uploaderName}</span>{/if}
            </div>
            <div class="meta muted">
              <span>視聴日時: {new Date(item.playedAt).toLocaleString()}</span>
            </div>
          </div>
          <button
            type="button"
            class="del-btn"
            onclick={() => handleDeleteItem(item.videoId, item.source)}
            title="この履歴を削除"
            aria-label="削除">✕</button
          >
          <VideoActionMenu
            video={{
              contentId: item.videoId,
              videoId: item.videoId,
              title: item.title,
              thumbnailUrl: item.thumbnailUrl ?? null,
              lengthSeconds: item.duration ?? null,
              source: 'history',
              historySource: item.source,
              playedAt: item.playedAt,
            }}
            compact={true}
          />
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 16px;
    flex-wrap: wrap;
    gap: 10px;
  }
  .head-tools {
    display: flex;
    gap: 12px;
    align-items: center;
    flex-wrap: wrap;
  }
  h2 {
    margin: 0;
  }
  .search-box {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    padding: 6px 12px;
    border-radius: 6px;
    font-size: 13px;
    width: 200px;
    outline: none;
    transition: border-color 0.15s;
  }
  .search-box::placeholder {
    color: var(--theme-text-faint);
  }
  .search-box:focus {
    border-color: var(--theme-accent-border);
  }
  .tabs {
    display: inline-flex;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 6px;
    overflow: hidden;
  }
  .tabs button {
    background: transparent;
    border: none;
    color: var(--theme-text-soft);
    padding: 6px 12px;
    cursor: pointer;
    font-size: 12px;
    border-right: 1px solid var(--theme-border);
  }
  .tabs button:last-child {
    border-right: none;
  }
  .tabs button:hover {
    background: var(--theme-border);
  }
  .tabs button.active {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
  }
  .clear-btn {
    background: var(--theme-danger-bg);
    color: var(--theme-danger-text);
    border: 1px solid var(--theme-danger-border);
    padding: 6px 12px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
  }
  .clear-btn:hover:not(:disabled) {
    background: var(--theme-danger-bg-2);
  }
  .clear-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .muted {
    color: var(--theme-text-muted);
  }
  .list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .item {
    display: flex;
    gap: 12px;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    padding: 8px;
    border-radius: 8px;
  }
  .thumb {
    width: 160px;
    height: 90px;
    object-fit: cover;
    border-radius: 4px;
    background: var(--theme-bg);
  }
  .thumb.placeholder {
    border: 1px dashed var(--theme-border-strong);
  }
  .thumb-link {
    position: relative;
    flex-shrink: 0;
  }
  .resume-overlay {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    /* サムネ上の暗オーバレイ。文字色も on-overlay (白) で固定して
       classic 時に accent-soft(暗青) で読めなくなる問題を防ぐ。 */
    background: var(--theme-overlay-medium);
    border-radius: 0 0 4px 4px;
    padding: 2px 6px;
    height: 18px;
  }
  .resume-bar {
    flex: 1;
    height: 3px;
    background: rgba(255, 255, 255, 0.25);
    border-radius: 2px;
    overflow: hidden;
  }
  .resume-bar-inner {
    height: 100%;
    background: var(--theme-on-overlay);
    border-radius: 2px;
  }
  .resume-time {
    font-size: 10px;
    color: var(--theme-on-overlay);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .title-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .title {
    color: var(--theme-text);
    text-decoration: none;
    font-weight: 600;
  }
  .title:hover {
    text-decoration: underline;
  }
  .src-tag {
    display: inline-block;
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 500;
    flex-shrink: 0;
  }
  .src-tag.local {
    background: var(--theme-success-bg);
    color: var(--theme-success-text);
    border: 1px solid var(--theme-success-border);
  }
  .src-tag.online {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
    border: 1px solid var(--theme-accent-border);
  }
  .meta {
    font-size: 12px;
    display: flex;
    gap: 4px;
  }
  .dot {
    color: var(--theme-text-faint);
  }
  .item {
    position: relative;
  }
  .del-btn {
    position: absolute;
    top: 4px;
    right: 4px;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 4px;
    /* サムネ上に重ねる削除ボタン。テーマに関係なく暗オーバレイ +
       白系文字でコントラストを担保 (旧: text-muted が classic で
       灰になり半黒地にほぼ見えない問題)。 */
    background: var(--theme-overlay-medium);
    color: var(--theme-on-overlay-muted);
    font-size: 12px;
    cursor: pointer;
    opacity: 0;
    transition:
      opacity 0.15s,
      background 0.15s;
  }
  .item:hover .del-btn {
    opacity: 1;
  }
  .del-btn:hover {
    background: var(--theme-danger-border);
    color: var(--theme-danger-text);
  }
</style>
