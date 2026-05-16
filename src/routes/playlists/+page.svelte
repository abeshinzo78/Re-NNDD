<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    createMylist,
    deleteMylist,
    listMylists,
    removeFromMylist,
    renameMylist,
    subscribeMylists,
    type Mylist,
  } from '$lib/stores/mylists';
  import { formatDate, formatDuration, formatNumber } from '$lib/format';

  let mylists = $state<Mylist[]>([]);
  let selectedId = $state<string | null>(null);
  let newName = $state('');
  let editingId = $state<string | null>(null);
  let editingName = $state('');

  function refresh() {
    mylists = listMylists();
    if (!selectedId || !mylists.some((m) => m.id === selectedId)) {
      selectedId = mylists[0]?.id ?? null;
    }
  }

  let unsub: (() => void) | null = null;
  onMount(() => {
    refresh();
    unsub = subscribeMylists(refresh);
  });
  onDestroy(() => unsub?.());

  let selected = $derived(mylists.find((m) => m.id === selectedId) ?? null);

  function onCreate(e: Event) {
    e.preventDefault();
    const name = newName.trim();
    if (!name) return;
    const m = createMylist(name);
    selectedId = m.id;
    newName = '';
  }

  function startRename(m: Mylist) {
    editingId = m.id;
    editingName = m.name;
  }

  function commitRename() {
    if (editingId && editingName.trim()) {
      renameMylist(editingId, editingName.trim());
    }
    editingId = null;
    editingName = '';
  }

  function cancelRename() {
    editingId = null;
    editingName = '';
  }

  function onDelete(m: Mylist) {
    if (m.builtin) return;
    if (!confirm(`「${m.name}」を削除しますか？(${m.items.length} 件の動画リンクも消えます)`))
      return;
    deleteMylist(m.id);
  }

  function onRemoveItem(videoId: string) {
    if (!selectedId) return;
    removeFromMylist(selectedId, videoId);
  }
</script>

<section>
  <h2>マイリスト</h2>
  <p class="muted">
    ローカルに保存される独自マイリスト。「マイリスト」は組み込みのブックマーク用。
  </p>

  <div class="layout">
    <aside class="sidebar">
      <ul class="mylist-list">
        {#each mylists as m (m.id)}
          <li>
            <div
              role="button"
              tabindex="0"
              class="ml"
              class:active={m.id === selectedId}
              onclick={() => (selectedId = m.id)}
              onkeydown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  selectedId = m.id;
                }
              }}
            >
              {#if editingId === m.id}
                <input
                  class="rename"
                  type="text"
                  bind:value={editingName}
                  onclick={(e) => e.stopPropagation()}
                  onblur={commitRename}
                  onkeydown={(e) => {
                    if (e.key === 'Enter') commitRename();
                    if (e.key === 'Escape') cancelRename();
                  }}
                />
              {:else}
                <span class="ml-name">{m.name}</span>
                {#if m.builtin}<span class="badge">標準</span>{/if}
                <span class="ml-count">{m.items.length}</span>
              {/if}
            </div>
          </li>
        {/each}
      </ul>

      <form class="create" onsubmit={onCreate}>
        <input type="text" placeholder="新しいマイリスト名" bind:value={newName} maxlength="60" />
        <button type="submit" disabled={!newName.trim()}>作成</button>
      </form>
    </aside>

    <div class="detail">
      {#if selected}
        <div class="detail-head">
          <h3>{selected.name}</h3>
          <div class="actions">
            {#if !selected.builtin}
              <button type="button" onclick={() => startRename(selected!)}>名前変更</button>
              <button type="button" class="danger" onclick={() => onDelete(selected!)}>削除</button>
            {/if}
          </div>
        </div>
        <p class="muted small">
          {formatNumber(selected.items.length)} 件 · 更新 {formatDate(
            new Date(selected.updatedAt).toISOString(),
          )}
        </p>
        {#if selected.items.length === 0}
          <p class="muted">
            まだ動画がありません。動画ページの「＋ マイリスト」ボタンから追加できます。
          </p>
        {:else}
          <ul class="items">
            {#each selected.items as item (item.videoId)}
              <li class="item">
                <a class="thumb-link" href="/video/{item.videoId}">
                  {#if item.thumbnailUrl}
                    <img src={item.thumbnailUrl} alt="" loading="lazy" />
                  {:else}
                    <div class="thumb-placeholder"></div>
                  {/if}
                </a>
                <div class="info">
                  <a class="title" href="/video/{item.videoId}">{item.title}</a>
                  <div class="row-meta muted">
                    <span>{item.videoId}</span>
                    {#if item.lengthSeconds != null}
                      <span class="dot">·</span><span>{formatDuration(item.lengthSeconds)}</span>
                    {/if}
                    {#if item.viewCounter != null}
                      <span class="dot">·</span><span>再生 {formatNumber(item.viewCounter)}</span>
                    {/if}
                    {#if item.uploaderName}
                      <span class="dot">·</span><span>{item.uploaderName}</span>
                    {/if}
                  </div>
                </div>
                <button type="button" class="remove" onclick={() => onRemoveItem(item.videoId)}>
                  外す
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      {:else}
        <p class="muted">マイリストを選択してください。</p>
      {/if}
    </div>
  </div>
</section>

<style>
  h2 {
    margin-top: 0;
    color: var(--text-heading);
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: 12px;
  }
  .layout {
    display: grid;
    grid-template-columns: 240px 1fr;
    gap: 16px;
    margin-top: 16px;
  }
  @media (max-width: 800px) {
    .layout {
      grid-template-columns: 1fr;
    }
  }
  .sidebar {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px;
    align-self: start;
  }
  .mylist-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ml {
    width: 100%;
    text-align: left;
    background: transparent;
    border: 1px solid transparent;
    color: var(--text-2);
    border-radius: 6px;
    padding: 8px 10px;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  .ml:hover {
    background: var(--surface-hover);
  }
  .ml.active {
    background: var(--surface-active);
    border-color: var(--border-strong);
    color: var(--text-heading);
  }
  .ml-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ml-count {
    color: var(--text-dim);
    font-size: 11px;
  }
  .badge {
    background: var(--accent);
    color: var(--accent-text);
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
  }
  .rename {
    flex: 1;
    background: var(--input-bg);
    border: 1px solid var(--border-3);
    color: var(--text-heading);
    border-radius: 4px;
    padding: 2px 6px;
    font-size: 13px;
    min-width: 0;
  }
  .create {
    display: flex;
    gap: 6px;
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border-2);
  }
  .create input {
    flex: 1;
    background: var(--input-bg);
    border: 1px solid var(--border-3);
    color: var(--text-heading);
    border-radius: 4px;
    padding: 4px 8px;
    font-size: 12px;
    min-width: 0;
  }
  .create button,
  .actions button {
    background: var(--surface-hover);
    border: 1px solid var(--menu-border);
    color: var(--text);
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .create button:hover,
  .actions button:hover {
    background: var(--surface-active);
  }
  .create button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .danger {
    border-color: var(--error-border) !important;
    color: var(--error-text) !important;
  }
  .danger:hover {
    background: var(--error-bg) !important;
  }
  .detail {
    min-width: 0;
  }
  .detail-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .detail-head h3 {
    margin: 0;
    font-size: 16px;
    color: var(--text-heading);
  }
  .actions {
    display: flex;
    gap: 6px;
  }
  .items {
    list-style: none;
    padding: 0;
    margin: 12px 0 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .item {
    display: grid;
    grid-template-columns: 140px 1fr auto;
    gap: 12px;
    align-items: center;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px;
  }
  .thumb-link img,
  .thumb-placeholder {
    width: 140px;
    height: 78px;
    object-fit: cover;
    background: var(--code-bg);
    border-radius: 4px;
    display: block;
  }
  .thumb-placeholder {
    border: 1px dashed var(--border-2);
  }
  .info {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .title {
    color: var(--text);
    text-decoration: none;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title:hover {
    text-decoration: underline;
  }
  .row-meta {
    font-size: 12px;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    align-items: center;
  }
  .dot {
    color: var(--text-faint);
  }
  .remove {
    background: transparent;
    border: 1px solid var(--menu-border);
    color: var(--text-3);
    border-radius: 6px;
    padding: 4px 8px;
    font-size: 12px;
    cursor: pointer;
  }
  .remove:hover {
    background: var(--surface-hover);
    color: var(--error-text);
    border-color: var(--error-border);
  }
</style>
