<script lang="ts">
  import { onDestroy } from 'svelte';
  import { goto } from '$app/navigation';
  import {
    advanceQueue,
    clearQueue,
    getQueue,
    itemHref,
    rewindQueue,
    subscribeQueue,
    type PlaybackQueue,
    type PlaybackQueueItem,
  } from '$lib/stores/playbackQueue';

  type Props = {
    /** 表示中の動画 ID。キューに無ければバナーは出ない。 */
    videoId: string;
  };

  let { videoId }: Props = $props();

  let queue = $state<PlaybackQueue | null>(getQueue());
  const unsub = subscribeQueue(() => (queue = getQueue()));
  onDestroy(unsub);

  let currentIdx = $derived(queue ? queue.items.findIndex((it) => it.videoId === videoId) : -1);
  let isInQueue = $derived(currentIdx >= 0);
  let current = $derived<PlaybackQueueItem | null>(
    queue && currentIdx >= 0 ? queue.items[currentIdx] : null,
  );
  let next = $derived<PlaybackQueueItem | null>(
    queue && currentIdx >= 0 ? (queue.items[currentIdx + 1] ?? null) : null,
  );
  let prev = $derived<PlaybackQueueItem | null>(
    queue && currentIdx > 0 ? (queue.items[currentIdx - 1] ?? null) : null,
  );

  function go(item: PlaybackQueueItem) {
    void goto(itemHref(item));
  }

  function onPrev() {
    if (!prev) return;
    rewindQueue();
    go(prev);
  }

  function onNext() {
    if (!next) return;
    advanceQueue();
    go(next);
  }

  function onStop() {
    clearQueue();
  }

  function contextLabel(q: PlaybackQueue): string {
    switch (q.context) {
      case 'series':
        return 'シリーズ';
      case 'mylist':
        return 'マイリスト';
      case 'smart':
        return 'スマートプレイリスト';
      case 'library':
        return 'ライブラリ';
      case 'user':
        return '投稿動画';
      default:
        return 'プレイリスト';
    }
  }
</script>

{#if queue && isInQueue && current}
  <div class="queue-banner" data-testid="queue-banner">
    <div class="queue-info">
      <span class="queue-kind">{contextLabel(queue)}</span>
      <span class="queue-label" title={queue.label}>{queue.label}</span>
      <span class="queue-progress">{currentIdx + 1} / {queue.items.length}</span>
    </div>
    <div class="queue-actions">
      <button
        type="button"
        class="qbtn"
        onclick={onPrev}
        disabled={!prev}
        title={prev ? `前: ${prev.title ?? prev.videoId}` : '先頭の動画'}
      >
        ← 前
      </button>
      <button
        type="button"
        class="qbtn primary"
        onclick={onNext}
        disabled={!next}
        title={next ? `次: ${next.title ?? next.videoId}` : '末尾の動画'}
      >
        次 →
      </button>
      <button type="button" class="qbtn ghost" onclick={onStop} title="連続再生を停止">
        ■ 停止
      </button>
    </div>
  </div>
{/if}

<style>
  .queue-banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 10px;
    background: var(--theme-accent-bg);
    border: 1px solid var(--theme-accent-border);
    border-radius: 6px;
    margin: 8px 0;
    font-size: 12px;
    flex-wrap: wrap;
  }
  .queue-info {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .queue-kind {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    padding: 1px 6px;
    border-radius: 999px;
    font-weight: 600;
  }
  .queue-label {
    color: var(--theme-text);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 360px;
  }
  .queue-progress {
    color: var(--theme-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .queue-actions {
    display: flex;
    gap: 6px;
  }
  .qbtn {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    padding: 4px 10px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 12px;
  }
  .qbtn:hover:not(:disabled) {
    background: var(--theme-surface-hover);
  }
  .qbtn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .qbtn.primary {
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    border-color: var(--theme-accent);
  }
  .qbtn.primary:hover:not(:disabled) {
    background: var(--theme-accent-hover);
  }
  .qbtn.ghost {
    background: transparent;
  }
</style>
