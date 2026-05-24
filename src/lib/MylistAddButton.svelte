<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    addToMylist,
    createMylist,
    isInMylist,
    listMylists,
    mylistsContaining,
    removeFromMylist,
    SAVED_MYLIST_ID,
    subscribeMylists,
    type Mylist,
    type MylistVideo,
  } from '$lib/stores/mylists';

  type Props = {
    video: Omit<MylistVideo, 'addedAt'>;
  };
  let { video }: Props = $props();

  let open = $state(false);
  let mylists = $state<Mylist[]>([]);
  let memberOf = $state<Set<string>>(new Set());
  let newName = $state('');
  let buttonEl: HTMLButtonElement | null = $state(null);
  let popoverEl: HTMLDivElement | null = $state(null);

  function refresh() {
    mylists = listMylists();
    memberOf = new Set(mylistsContaining(video.videoId));
  }

  let unsub: (() => void) | null = null;
  onMount(() => {
    refresh();
    unsub = subscribeMylists(refresh);
    document.addEventListener('mousedown', onDocClick);
  });
  onDestroy(() => {
    unsub?.();
    document.removeEventListener('mousedown', onDocClick);
  });

  function onDocClick(e: MouseEvent) {
    if (!open) return;
    const t = e.target as Node;
    if (popoverEl?.contains(t) || buttonEl?.contains(t)) return;
    open = false;
  }

  $effect(() => {
    // refresh when the video id changes
    void video.videoId;
    refresh();
  });

  function toggle(id: string) {
    if (memberOf.has(id)) {
      removeFromMylist(id, video.videoId);
    } else {
      addToMylist(id, video);
    }
  }

  function quickSave() {
    if (isInMylist(SAVED_MYLIST_ID, video.videoId)) {
      removeFromMylist(SAVED_MYLIST_ID, video.videoId);
    } else {
      addToMylist(SAVED_MYLIST_ID, video);
    }
  }

  function createAndAdd() {
    const name = newName.trim();
    if (!name) return;
    const m = createMylist(name);
    addToMylist(m.id, video);
    newName = '';
  }

  let savedActive = $derived(memberOf.has(SAVED_MYLIST_ID));
</script>

<div class="wrap">
  <button
    type="button"
    class="save mylist-add-btn"
    class:active={savedActive}
    onclick={quickSave}
    title={savedActive ? 'マイリストから外す' : 'マイリストに追加'}
  >
    {savedActive ? '★ マイリスト' : '☆ マイリスト'}
  </button>
  <button
    type="button"
    class="more mylist-create-btn"
    bind:this={buttonEl}
    onclick={() => (open = !open)}
    aria-haspopup="true"
    aria-expanded={open}
    title="マイリストに追加"
  >
    ＋ マイリスト
  </button>

  {#if open}
    <div class="popover" bind:this={popoverEl} role="dialog" aria-label="マイリストに追加">
      <div class="header">マイリストに追加</div>
      <ul class="list">
        {#each mylists as m (m.id)}
          <li>
            <label>
              <input type="checkbox" checked={memberOf.has(m.id)} onchange={() => toggle(m.id)} />
              <span class="name">{m.name}</span>
              {#if m.builtin}<span class="badge">標準</span>{/if}
              <span class="count">{m.items.length}</span>
            </label>
          </li>
        {/each}
      </ul>
      <form
        class="create"
        onsubmit={(e) => {
          e.preventDefault();
          createAndAdd();
        }}
      >
        <input type="text" placeholder="新しいマイリスト名" bind:value={newName} maxlength="60" />
        <button type="submit" disabled={!newName.trim()}>作成</button>
      </form>
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
    display: inline-flex;
    gap: 6px;
  }
  button {
    background: var(--theme-border);
    border: 1px solid var(--theme-surface-hover);
    color: var(--theme-text);
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  button:hover {
    background: var(--theme-border-strong);
  }
  .save.active {
    background: var(--theme-warning-border);
    border-color: var(--theme-warning-text);
    color: var(--theme-text);
  }
  .popover {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 50;
    width: 280px;
    background: var(--theme-surface-4);
    border: 1px solid var(--theme-surface-hover);
    border-radius: 8px;
    box-shadow: var(--theme-menu-shadow);
    padding: 8px;
  }
  .header {
    font-size: 12px;
    color: var(--theme-text-muted);
    padding: 4px 6px 8px;
    border-bottom: 1px solid var(--theme-border-strong);
    margin-bottom: 6px;
  }
  .list {
    list-style: none;
    padding: 0;
    margin: 0;
    max-height: 220px;
    overflow-y: auto;
  }
  .list li label {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px;
    cursor: pointer;
    border-radius: 4px;
    font-size: 13px;
  }
  .list li label:hover {
    background: var(--theme-surface-hover);
  }
  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge {
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
  }
  .count {
    color: var(--theme-text-muted);
    font-size: 11px;
  }
  .create {
    display: flex;
    gap: 6px;
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--theme-border-strong);
  }
  .create input {
    flex: 1;
    background: var(--theme-input-bg);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    border-radius: 4px;
    padding: 4px 8px;
    font-size: 12px;
    min-width: 0;
  }
  .create button {
    flex-shrink: 0;
  }
  .create button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
