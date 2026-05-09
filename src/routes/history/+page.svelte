<script lang="ts">
  import { onMount } from 'svelte';
  import { clearHistory, getHistory, type HistoryItem, type HistorySource } from '$lib/stores/history';
  import { formatDuration } from '$lib/format';

  let history = $state<HistoryItem[]>([]);
  let filter = $state<'all' | HistorySource>('all');

  onMount(() => {
    history = getHistory();
  });

  let visible = $derived.by(() => {
    if (filter === 'all') return history;
    return history.filter((h) => (h.source ?? 'online') === filter);
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
      ? `/library/${item.videoId}`
      : `/video/${item.videoId}`;
  }
</script>

<section>
  <div class="head">
    <h2>再生履歴</h2>
    <div class="head-tools">
      <div class="tabs" role="tablist" aria-label="履歴フィルタ">
        <button
          type="button"
          role="tab"
          aria-selected={filter === 'all'}
          class:active={filter === 'all'}
          onclick={() => (filter = 'all')}
        >すべて ({counts.all})</button>
        <button
          type="button"
          role="tab"
          aria-selected={filter === 'online'}
          class:active={filter === 'online'}
          onclick={() => (filter = 'online')}
        >オンライン ({counts.online})</button>
        <button
          type="button"
          role="tab"
          aria-selected={filter === 'local'}
          class:active={filter === 'local'}
          onclick={() => (filter = 'local')}
        >ローカル ({counts.local})</button>
      </div>
      <button type="button" class="clear-btn" onclick={handleClear} disabled={history.length === 0}>
        履歴をクリア
      </button>
    </div>
  </div>

  {#if visible.length === 0}
    <p class="muted">
      {#if filter === 'local'}ローカル再生の履歴はありません。
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
              {#if item.duration}<span class="dot">·</span><span>{formatDuration(item.duration)}</span>{/if}
              {#if item.uploaderName}<span class="dot">·</span><span>{item.uploaderName}</span>{/if}
            </div>
            <div class="meta muted">
              <span>視聴日時: {new Date(item.playedAt).toLocaleString()}</span>
            </div>
          </div>
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
  .tabs {
    display: inline-flex;
    background: #161616;
    border: 1px solid #1f1f1f;
    border-radius: 6px;
    overflow: hidden;
  }
  .tabs button {
    background: transparent;
    border: none;
    color: #cfcfcf;
    padding: 6px 12px;
    cursor: pointer;
    font-size: 12px;
    border-right: 1px solid #1f1f1f;
  }
  .tabs button:last-child { border-right: none; }
  .tabs button:hover { background: #1f1f1f; }
  .tabs button.active { background: #1f2a44; color: #93c5fd; }
  .clear-btn {
    background: #2a1212;
    color: #f5b3b3;
    border: 1px solid #5a2222;
    padding: 6px 12px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
  }
  .clear-btn:hover:not(:disabled) { background: #3a1a1a; }
  .clear-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .muted { color: #9a9a9a; }
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
    background: #161616;
    border: 1px solid #1f1f1f;
    padding: 8px;
    border-radius: 8px;
  }
  .thumb {
    width: 160px;
    height: 90px;
    object-fit: cover;
    border-radius: 4px;
    background: #0a0a0a;
  }
  .thumb.placeholder { border: 1px dashed #2a2a2a; }
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
    color: #eaeaea;
    text-decoration: none;
    font-weight: 600;
  }
  .title:hover { text-decoration: underline; }
  .src-tag {
    display: inline-block;
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 500;
    flex-shrink: 0;
  }
  .src-tag.local {
    background: #1a3a26;
    color: #b3f5b3;
    border: 1px solid #2a5a3a;
  }
  .src-tag.online {
    background: #1a2a44;
    color: #93c5fd;
    border: 1px solid #2a3f5a;
  }
  .meta {
    font-size: 12px;
    display: flex;
    gap: 4px;
  }
  .dot { color: #555; }
</style>
