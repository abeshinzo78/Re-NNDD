<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    cancelDownload,
    clearFinishedDownloads,
    enqueueDownload,
    listDownloads,
    startDownload,
    type DownloadQueueItem,
    type DownloadStatus,
  } from '$lib/api';
  import { formatDate } from '$lib/format';

  let items = $state<DownloadQueueItem[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);

  let videoIdInput = $state('');
  let enqueueing = $state(false);
  let toast = $state<string | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // 段階1 では実 DL がまだ無いので、進捗が無いまま pending が並ぶだけ。
  // 段階2 で worker が動き出したら startedAt / progress が更新される想定で、
  // UI 側はとりあえず低頻度ポーリングで状態を反映する。
  let pollTimer: ReturnType<typeof setInterval> | null = null;

  function showToast(msg: string) {
    toast = msg;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 2200);
  }

  async function refresh() {
    try {
      items = await listDownloads();
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  async function onEnqueue(e: Event) {
    e.preventDefault();
    const id = videoIdInput.trim();
    if (!id) return;
    if (!/^[A-Za-z0-9]+$/.test(id)) {
      showToast('動画 ID は英数字のみ（例: sm9, so12345）');
      return;
    }
    enqueueing = true;
    try {
      await enqueueDownload(id);
      videoIdInput = '';
      showToast(`${id} をキューに追加`);
      await refresh();
    } catch (err) {
      showToast(`追加に失敗: ${err}`);
    } finally {
      enqueueing = false;
    }
  }

  async function onCancel(item: DownloadQueueItem) {
    const ok = confirm(`${item.videoId} のジョブを削除しますか？`);
    if (!ok) return;
    try {
      await cancelDownload(item.id);
      await refresh();
    } catch (err) {
      showToast(`キャンセル失敗: ${err}`);
    }
  }

  async function onStart(item: DownloadQueueItem) {
    try {
      await startDownload(item.id);
      showToast(`${item.videoId} の DL を開始`);
      await refresh();
    } catch (err) {
      showToast(`DL 開始失敗: ${err}`);
    }
  }

  function canStart(s: DownloadStatus): boolean {
    return s === 'pending' || s === 'paused' || s === 'error';
  }

  async function onClearFinished() {
    try {
      const n = await clearFinishedDownloads();
      showToast(n > 0 ? `${n} 件削除` : '削除対象なし');
      await refresh();
    } catch (err) {
      showToast(`掃除失敗: ${err}`);
    }
  }

  function statusLabel(s: DownloadStatus): string {
    switch (s) {
      case 'pending':
        return '待機中';
      case 'downloading':
        return 'DL 中';
      case 'done':
        return '完了';
      case 'error':
        return 'エラー';
      case 'paused':
        return '一時停止';
      default:
        return s;
    }
  }

  function progressPct(p: number): number {
    return Math.round(Math.max(0, Math.min(1, p)) * 100);
  }

  onMount(async () => {
    loading = true;
    await refresh();
    loading = false;
    pollTimer = setInterval(refresh, 3000);
  });

  onDestroy(() => {
    if (pollTimer) clearInterval(pollTimer);
    if (toastTimer) clearTimeout(toastTimer);
  });
</script>

<section class="page">
  <header class="head">
    <h2>ダウンロード</h2>
    <p class="muted">
      「DL 開始」で <code>{'app_data_dir/videos/{videoId}/'}</code> 配下に
      <code>video.mp4</code> / <code>audio.mp4</code> / <code>thumbnail.jpg</code> /
      <code>description.txt</code> / <code>meta.json</code> を保存し、初期コメ スナップショットをライブラリに取り込みます。AES-128
      暗号化セグメントも対応。
    </p>
  </header>

  <form class="enqueue" onsubmit={onEnqueue}>
    <input
      type="text"
      placeholder="動画 ID (例: sm9)"
      bind:value={videoIdInput}
      disabled={enqueueing}
    />
    <button type="submit" disabled={enqueueing || !videoIdInput.trim()}> キューに追加 </button>
    <button type="button" class="ghost" onclick={onClearFinished}> 完了/失敗を掃除 </button>
  </form>

  {#if error}
    <div class="error">エラー: {error}</div>
  {/if}

  {#if loading && items.length === 0}
    <div class="muted">読み込み中…</div>
  {:else if items.length === 0}
    <div class="muted empty">キューは空です。動画 ID を入れて追加してください。</div>
  {:else}
    <table class="queue">
      <thead>
        <tr>
          <th class="col-status">状態</th>
          <th class="col-video">動画 ID</th>
          <th class="col-progress">進捗</th>
          <th class="col-time">予約 / 開始 / 完了</th>
          <th class="col-actions"></th>
        </tr>
      </thead>
      <tbody>
        {#each items as item (item.id)}
          <tr class="row" class:err={item.status === 'error'}>
            <td>
              <span class="badge {item.status}">{statusLabel(item.status)}</span>
              {#if item.retryCount > 0}
                <span class="retry" title="リトライ回数">×{item.retryCount}</span>
              {/if}
            </td>
            <td><code>{item.videoId}</code></td>
            <td>
              <div class="progress-wrap" title={`${progressPct(item.progress)}%`}>
                <div class="progress-bar" style:width="{progressPct(item.progress)}%"></div>
                <span class="progress-num">{progressPct(item.progress)}%</span>
              </div>
              {#if item.errorMessage}
                <div class="err-msg" title={item.errorMessage}>{item.errorMessage}</div>
              {/if}
            </td>
            <td class="times">
              {#if item.scheduledAt}
                <div>予 {formatDate(new Date(item.scheduledAt * 1000).toISOString())}</div>
              {/if}
              {#if item.startedAt}
                <div>開 {formatDate(new Date(item.startedAt * 1000).toISOString())}</div>
              {/if}
              {#if item.finishedAt}
                <div>完 {formatDate(new Date(item.finishedAt * 1000).toISOString())}</div>
              {/if}
              {#if !item.scheduledAt && !item.startedAt && !item.finishedAt}
                <span class="muted">—</span>
              {/if}
            </td>
            <td class="actions">
              {#if canStart(item.status)}
                <button type="button" class="start" onclick={() => onStart(item)}> DL 開始 </button>
              {/if}
              <button type="button" class="cancel" onclick={() => onCancel(item)}> 削除 </button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}

  {#if toast}
    <div class="toast" role="status">{toast}</div>
  {/if}
</section>

<style>
  .page {
    max-width: 1100px;
  }
  .head h2 {
    margin: 0 0 4px;
  }
  .head .muted {
    margin: 0 0 16px;
    font-size: 12px;
  }
  .muted {
    color: var(--theme-text-muted);
  }
  .empty {
    padding: 24px;
    text-align: center;
    border: 1px dashed var(--theme-border-strong);
    border-radius: 8px;
    margin-top: 16px;
  }
  .enqueue {
    display: flex;
    gap: 8px;
    margin-bottom: 16px;
    flex-wrap: wrap;
  }
  .enqueue input[type='text'] {
    flex: 1 1 240px;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
  }
  .enqueue input:focus {
    outline: none;
    border-color: var(--theme-accent-soft);
  }
  .enqueue button {
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    border: none;
    padding: 8px 14px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
  }
  .enqueue button:hover:not(:disabled) {
    background: var(--theme-accent-hover);
  }
  .enqueue button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .enqueue button.ghost {
    background: transparent;
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text-soft);
  }
  .enqueue button.ghost:hover {
    background: var(--theme-surface-3);
  }
  .error {
    background: var(--theme-danger-bg);
    border: 1px solid var(--theme-danger-border);
    color: var(--theme-danger-text);
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
    margin-bottom: 12px;
  }
  table.queue {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  thead th {
    text-align: left;
    color: var(--theme-text-muted);
    font-weight: 500;
    padding: 8px 10px;
    border-bottom: 1px solid var(--theme-border);
  }
  tbody td {
    padding: 8px 10px;
    border-bottom: 1px solid var(--theme-surface-4);
    vertical-align: top;
  }
  .col-status {
    width: 110px;
  }
  .col-video {
    width: 140px;
  }
  .col-progress {
    min-width: 200px;
  }
  .col-time {
    width: 220px;
  }
  .col-actions {
    width: 80px;
    text-align: right;
  }
  .row.err td {
    background: var(--theme-danger-bg-2);
  }
  .badge {
    display: inline-block;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    background: var(--theme-border);
    color: var(--theme-text-soft);
  }
  .badge.pending {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
  }
  .badge.downloading {
    background: var(--theme-success-bg);
    color: var(--theme-success-text);
  }
  .badge.done {
    background: var(--theme-success-bg-2);
    color: var(--theme-success-text);
  }
  .badge.error {
    background: var(--theme-danger-bg);
    color: var(--theme-danger-text);
  }
  .badge.paused {
    background: var(--theme-warning-bg);
    color: var(--theme-warning-text);
  }
  .retry {
    margin-left: 6px;
    color: var(--theme-danger-text);
    font-size: 11px;
  }
  .progress-wrap {
    position: relative;
    height: 14px;
    background: var(--theme-surface-3);
    border-radius: 6px;
    overflow: hidden;
  }
  .progress-bar {
    height: 100%;
    background: var(--theme-accent);
    transition: width 0.3s ease;
  }
  .progress-num {
    position: absolute;
    inset: 0;
    text-align: center;
    font-size: 10px;
    color: var(--theme-text);
    line-height: 14px;
    text-shadow: 0 0 2px rgba(0, 0, 0, 0.6);
  }
  .err-msg {
    margin-top: 4px;
    color: var(--theme-danger-text);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }
  .times {
    color: var(--theme-text-muted);
    font-size: 11px;
    line-height: 1.5;
  }
  .actions {
    text-align: right;
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: flex-end;
  }
  .start {
    background: var(--theme-success-bg);
    border: 1px solid var(--theme-success-border);
    color: var(--theme-success-text);
    padding: 4px 10px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 12px;
  }
  .start:hover {
    background: var(--theme-success-border);
  }
  .cancel {
    background: transparent;
    border: 1px solid var(--theme-danger-border);
    color: var(--theme-danger-text);
    padding: 4px 10px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 12px;
  }
  .cancel:hover {
    background: var(--theme-danger-bg);
  }
  code {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 12px;
  }
  .toast {
    position: fixed;
    bottom: 24px;
    right: 24px;
    background: var(--theme-success-bg-2);
    color: var(--theme-success-text);
    border: 1px solid var(--theme-success-border-2);
    padding: 6px 14px;
    border-radius: 6px;
    font-size: 12px;
    z-index: 1000;
    pointer-events: none;
  }
</style>
