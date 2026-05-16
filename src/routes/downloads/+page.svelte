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
    color: var(--text-heading);
  }
  .head .muted {
    margin: 0 0 16px;
    font-size: 12px;
  }
  .muted {
    color: var(--text-muted);
  }
  .empty {
    padding: 24px;
    text-align: center;
    border: 1px dashed var(--border-2);
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
    background: var(--surface-2);
    border: 1px solid var(--border-2);
    color: var(--text);
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
  }
  .enqueue input:focus {
    outline: none;
    border-color: var(--link);
  }
  .enqueue button {
    background: var(--accent);
    color: var(--accent-text);
    border: none;
    padding: 8px 14px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
  }
  .enqueue button:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .enqueue button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .enqueue button.ghost {
    background: transparent;
    border: 1px solid var(--border-2);
    color: var(--text-2);
  }
  .enqueue button.ghost:hover {
    background: var(--surface-3);
  }
  .error {
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    color: var(--error-text);
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
    color: var(--text-muted);
    font-weight: 500;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
  }
  tbody td {
    padding: 8px 10px;
    border-bottom: 1px solid var(--surface-3);
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
    background: var(--error-bg);
  }
  .badge {
    display: inline-block;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    background: var(--surface-hover);
    color: var(--text-2);
  }
  .badge.pending {
    background: var(--badge-blue-bg-soft);
    color: var(--badge-blue-text);
  }
  .badge.downloading {
    background: var(--success-icon-bg);
    color: var(--success-strong);
  }
  .badge.done {
    background: var(--success-icon-bg);
    color: var(--success-icon-text);
  }
  .badge.error {
    background: var(--error-bg);
    color: var(--error-text);
  }
  .badge.paused {
    background: var(--warn-bg);
    color: var(--warn-text);
  }
  .retry {
    margin-left: 6px;
    color: var(--error-text);
    font-size: 11px;
  }
  .progress-wrap {
    position: relative;
    height: 14px;
    background: var(--surface-3);
    border-radius: 6px;
    overflow: hidden;
  }
  .progress-bar {
    height: 100%;
    background: var(--accent);
    transition: width 0.3s ease;
  }
  .progress-num {
    position: absolute;
    inset: 0;
    text-align: center;
    font-size: 10px;
    color: var(--text);
    line-height: 14px;
    text-shadow: 0 0 2px rgba(0, 0, 0, 0.6);
  }
  .err-msg {
    margin-top: 4px;
    color: var(--error-text);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }
  .times {
    color: var(--text-muted);
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
    background: var(--success-icon-bg);
    border: 1px solid var(--success-icon-border);
    color: var(--success-icon-text);
    padding: 4px 10px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 12px;
  }
  .start:hover {
    background: var(--success-icon-border);
  }
  .cancel {
    background: transparent;
    border: 1px solid var(--error-border);
    color: var(--error-text);
    padding: 4px 10px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 12px;
  }
  .cancel:hover {
    background: var(--error-bg);
  }
  code {
    background: var(--surface-2);
    border: 1px solid var(--border);
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 12px;
  }
  .toast {
    position: fixed;
    bottom: 24px;
    right: 24px;
    background: var(--success-icon-bg);
    color: var(--success-icon-text);
    border: 1px solid var(--success-icon-border);
    padding: 6px 14px;
    border-radius: 6px;
    font-size: 12px;
    z-index: 1000;
    pointer-events: none;
  }
</style>
