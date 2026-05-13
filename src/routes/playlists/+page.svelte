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
            onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); selectedId = m.id; } }}
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
  }
  .muted {
    color: #9a9a9a;
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
    background: #161616;
    border: 1px solid #1f1f1f;
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
    color: #cfcfcf;
    border-radius: 6px;
    padding: 8px 10px;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  .ml:hover {
    background: #1f1f1f;
  }
  .ml.active {
    background: #2a2a2a;
    border-color: #3a3a3a;
    color: #fff;
  }
  .ml-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ml-count {
    color: #888;
    font-size: 11px;
  }
  .badge {
    background: #2563eb;
    color: white;
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
  }
  .rename {
    flex: 1;
    background: #0f0f0f;
    border: 1px solid #2f2f2f;
    color: #f5f5f5;
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
    border-top: 1px solid #2a2a2a;
  }
  .create input {
    flex: 1;
    background: #0f0f0f;
    border: 1px solid #2f2f2f;
    color: #f5f5f5;
    border-radius: 4px;
    padding: 4px 8px;
    font-size: 12px;
    min-width: 0;
  }
  .create button,
  .actions button {
    background: #1f1f1f;
    border: 1px solid #333;
    color: #eaeaea;
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .create button:hover,
  .actions button:hover {
    background: #2a2a2a;
  }
  .create button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .danger {
    border-color: #5a2222 !important;
    color: #f5b3b3 !important;
  }
  .danger:hover {
    background: #2a1212 !important;
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
    background: #161616;
    border: 1px solid #1f1f1f;
    border-radius: 8px;
    padding: 8px;
  }
  .thumb-link img,
  .thumb-placeholder {
    width: 140px;
    height: 78px;
    object-fit: cover;
    background: #0a0a0a;
    border-radius: 4px;
    display: block;
  }
  .thumb-placeholder {
    border: 1px dashed #2a2a2a;
  }
  .info {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .title {
    color: #eaeaea;
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
    color: #555;
  }
  .remove {
    background: transparent;
    border: 1px solid #333;
    color: #b0b0b0;
    border-radius: 6px;
    padding: 4px 8px;
    font-size: 12px;
    cursor: pointer;
  }
  .remove:hover {
    background: #1f1f1f;
    color: #f5b3b3;
    border-color: #5a2222;
  }
</style>
