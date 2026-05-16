<script lang="ts">
  import { onMount } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import {
    cleanupStorage,
    deleteLibraryVideo,
    listLibraryVideos,
    queryLibraryVideos,
    type LibraryVideoItem,
  } from '$lib/api';
  import { formatDuration, formatNumber } from '$lib/format';

  let items = $state<LibraryVideoItem[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let deleting = $state<string | null>(null);
  let searchQuery = $state('');

  async function refresh() {
    try {
      if (searchQuery.trim()) {
        const result = await queryLibraryVideos({ q: searchQuery.trim() });
        items = result.items.map((r) => ({
          id: r.id,
          title: r.title,
          durationSec: r.durationSec,
          uploaderId: r.uploaderId ?? null,
          uploaderName: r.uploaderName ?? null,
          viewCount: r.viewCount ?? null,
          postedAt: r.postedAt ?? null,
          downloadedAt: r.downloadedAt ?? null,
          resolution: r.resolution ?? null,
          thumbnailUrl: r.thumbnailUrl ?? null,
          localThumbnailPath: r.localThumbnailPath ?? null,
          localVideoPath: r.videoPath ?? null,
          tags: r.tags,
        }));
      } else {
        items = await listLibraryVideos();
      }
      error = null;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  let searchTimeout: ReturnType<typeof setTimeout> | null = null;
  function onSearchInput() {
    if (searchTimeout) clearTimeout(searchTimeout);
    searchTimeout = setTimeout(refresh, 300);
  }

  let cleaning = $state(false);
  let cleanupMsg = $state<string | null>(null);
  async function onCleanup() {
    if (!confirm('既存 DL 物から不要なサイドカー(古い meta.json 等)を削除します。')) return;
    cleaning = true;
    cleanupMsg = null;
    try {
      const bytes = await cleanupStorage();
      const mb = (bytes / 1024 / 1024).toFixed(2);
      cleanupMsg = bytes > 0 ? `${mb} MB 削除しました` : '削除対象なし';
    } catch (e) {
      cleanupMsg = `失敗: ${e}`;
    } finally {
      cleaning = false;
    }
  }

  async function onDelete(item: LibraryVideoItem, e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (
      !confirm(
        `「${item.title}」(${item.id}) を完全に削除しますか？\nファイル + DB 両方削除されます。`,
      )
    )
      return;
    deleting = item.id;
    try {
      await deleteLibraryVideo(item.id);
      await refresh();
    } catch (err) {
      error = `削除失敗: ${err}`;
    } finally {
      deleting = null;
    }
  }

  function thumbSrc(item: LibraryVideoItem): string | undefined {
    if (item.localThumbnailPath) return convertFileSrc(item.localThumbnailPath);
    return item.thumbnailUrl ?? undefined;
  }

  function relativeDate(unix: number | null): string {
    if (!unix) return '';
    const d = new Date(unix * 1000);
    return d.toLocaleDateString('ja-JP', { year: 'numeric', month: '2-digit', day: '2-digit' });
  }

  /** "1280x720" → "720p" などに整形。マッチしない時は元文字列をそのまま返す。 */
  function shortResolution(res: string | null): string | null {
    if (!res) return null;
    const m = /^\s*(\d+)x(\d+)\s*$/.exec(res);
    if (!m) return res;
    const h = Number(m[2]);
    return Number.isFinite(h) ? `${h}p` : res;
  }

  onMount(refresh);
</script>

<section class="page">
  <header class="head">
    <h2>ライブラリ</h2>
    <div class="head-actions">
      <input
        type="search"
        class="search-box"
        placeholder="動画名・タグで検索…"
        bind:value={searchQuery}
        oninput={onSearchInput}
      />
      <button type="button" class="ghost" onclick={refresh}>更新</button>
      <button type="button" class="ghost" disabled={cleaning} onclick={onCleanup}>
        {cleaning ? '掃除中…' : 'ストレージ掃除'}
      </button>
    </div>
  </header>

  {#if cleanupMsg}
    <div class="info">{cleanupMsg}</div>
  {/if}

  {#if error}
    <div class="error">エラー: {error}</div>
  {/if}

  {#if loading}
    <div class="muted">読み込み中…</div>
  {:else if items.length === 0}
    <div class="empty">
      <p class="muted">
        {searchQuery.trim()
          ? '検索結果が見つかりません。'
          : 'ダウンロード済みの動画はまだありません。'}
      </p>
      {#if !searchQuery.trim()}
        <p class="muted">
          <a href="/downloads">ダウンロード</a> ページで動画 ID を追加 → 「DL 開始」で取り込めます。
        </p>
      {/if}
    </div>
  {:else}
    <div class="grid">
      {#each items as item (item.id)}
        <div class="card-wrap">
          <a class="card" href={`/library/${item.id}?from=library`}>
            <div class="thumb-wrap">
              {#if thumbSrc(item)}
                <img class="thumb" src={thumbSrc(item)} alt="" loading="lazy" />
              {:else}
                <div class="thumb-placeholder">?</div>
              {/if}
              {#if shortResolution(item.resolution)}
                <span class="resolution" title={item.resolution ?? ''}>
                  {shortResolution(item.resolution)}
                </span>
              {/if}
              <span class="duration">{formatDuration(item.durationSec)}</span>
            </div>
            <div class="meta">
              <h3 class="title" title={item.title}>{item.title}</h3>
              <div class="row muted">
                {#if item.uploaderName}<span class="uploader">{item.uploaderName}</span>{/if}
                {#if item.viewCount != null}
                  <span class="dot">·</span>
                  <span>{formatNumber(item.viewCount)} 再生</span>
                {/if}
              </div>
              <div class="row muted small">
                <span>DL {relativeDate(item.downloadedAt)}</span>
                {#if item.resolution}
                  <span class="dot">·</span>
                  <span>{item.resolution}</span>
                {/if}
              </div>
              {#if item.tags.length > 0}
                <div class="tags">
                  {#each item.tags.slice(0, 4) as tag (tag)}
                    <span class="tag">{tag}</span>
                  {/each}
                  {#if item.tags.length > 4}
                    <span class="tag muted">+{item.tags.length - 4}</span>
                  {/if}
                </div>
              {/if}
            </div>
          </a>
          <button
            type="button"
            class="del-btn"
            disabled={deleting === item.id}
            title="ライブラリから完全削除"
            onclick={(e) => onDelete(item, e)}>{deleting === item.id ? '…' : '×'}</button
          >
        </div>
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
    justify-content: space-between;
    margin-bottom: 16px;
    flex-wrap: wrap;
    gap: 10px;
  }
  .head h2 {
    margin: 0;
    color: var(--text-heading);
  }
  .head-actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .search-box {
    background: var(--surface-2);
    border: 1px solid var(--border-2);
    color: var(--text);
    padding: 6px 12px;
    border-radius: 6px;
    font-size: 13px;
    width: 220px;
    outline: none;
    transition: border-color 0.15s;
  }
  .search-box::placeholder {
    color: var(--text-dim);
  }
  .search-box:focus {
    border-color: var(--accent-soft-border);
  }
  .info {
    background: var(--badge-blue-bg-soft);
    color: var(--badge-blue-text);
    border: 1px solid var(--badge-blue-border);
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 12px;
    margin-bottom: 12px;
  }
  .ghost {
    background: transparent;
    border: 1px solid var(--border-2);
    color: var(--text-2);
    padding: 6px 12px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
  }
  .ghost:hover {
    background: var(--surface-3);
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: 11px;
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
  .empty {
    padding: 32px;
    text-align: center;
    border: 1px dashed var(--border-2);
    border-radius: 8px;
  }
  .empty a {
    color: var(--link);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 14px;
  }
  .card-wrap {
    position: relative;
  }
  .del-btn {
    position: absolute;
    top: 6px;
    right: 6px;
    z-index: 2;
    background: var(--surface-active);
    color: var(--error-text);
    border: 1px solid var(--error-border);
    width: 26px;
    height: 26px;
    border-radius: 50%;
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
    padding: 0;
    opacity: 0;
    transition: opacity 0.1s;
  }
  .card-wrap:hover .del-btn {
    opacity: 1;
  }
  .del-btn:hover {
    background: var(--error-bg);
  }
  .del-btn:disabled {
    opacity: 0.5;
    cursor: wait;
  }
  .card {
    display: block;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
    text-decoration: none;
    color: inherit;
    transition:
      background 0.1s,
      border-color 0.1s,
      transform 0.1s;
  }
  .card:hover {
    background: var(--surface-hover);
    border-color: var(--border-strong);
    transform: translateY(-1px);
  }
  .thumb-wrap {
    position: relative;
    aspect-ratio: 16 / 9;
    background: #000;
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
    color: var(--text-faint);
    font-size: 32px;
  }
  .duration {
    position: absolute;
    right: 6px;
    bottom: 6px;
    background: rgba(0, 0, 0, 0.78);
    color: #eaeaea;
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .resolution {
    position: absolute;
    left: 6px;
    bottom: 6px;
    background: rgba(37, 99, 235, 0.85);
    color: #ffffff;
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 500;
  }
  .meta {
    padding: 10px 12px;
  }
  .title {
    font-size: 14px;
    margin: 0 0 6px;
    line-height: 1.3;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    color: var(--text);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    margin-top: 4px;
  }
  .dot {
    color: var(--text-faint);
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 6px;
  }
  .tag {
    background: var(--tag-bg);
    color: var(--tag-text);
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 11px;
  }
</style>
