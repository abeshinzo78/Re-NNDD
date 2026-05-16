<script lang="ts">
  import { tick, onMount, onDestroy } from 'svelte';
  import { formatDuration } from '$lib/format';
  import { addNgRule } from '$lib/stores/ngRules';
  import type { PlayerComment } from './types';

  type Props = {
    comments: PlayerComment[];
    currentTime: number;
    onSeek: (t: number) => void;
  };

  let { comments, currentTime, onSeek }: Props = $props();

  let listEl = $state<HTMLUListElement | null>(null);
  let autoScroll = $state(true);

  let menuFor = $state<string | null>(null); // comment id with open menu
  let menuPos = $state<{ x: number; y: number }>({ x: 0, y: 0 });
  let toast = $state<string | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;
  let menuEl: HTMLDivElement | null = $state(null);

  // Keep comments sorted by vpos (defensive — server returns sorted but
  // multiple forks can interleave).
  let sorted = $derived([...comments].sort((a, b) => a.vposMs - b.vposMs));

  let activeIndex = $derived.by(() => {
    if (sorted.length === 0) return -1;
    const cur = currentTime * 1000;
    let lo = 0;
    let hi = sorted.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >>> 1;
      if (sorted[mid].vposMs <= cur) lo = mid;
      else hi = mid - 1;
    }
    return sorted[lo].vposMs <= cur ? lo : -1;
  });

  let lastScrolled = $state(-1);

  $effect(() => {
    if (!autoScroll || activeIndex < 0 || !listEl) return;
    if (activeIndex === lastScrolled) return;
    lastScrolled = activeIndex;
    void tick().then(() => {
      if (!listEl) return;
      const node = listEl.querySelector<HTMLLIElement>(`li[data-i="${activeIndex}"]`);
      if (!node) return;
      const listRect = listEl.getBoundingClientRect();
      const nodeRect = node.getBoundingClientRect();
      const relTop = nodeRect.top - listRect.top;
      if (relTop < 0 || relTop + nodeRect.height > listRect.height) {
        const target = listEl.scrollTop + relTop - listRect.height / 2 + nodeRect.height / 2;
        listEl.scrollTo({ top: Math.max(0, target), behavior: 'smooth' });
      }
    });
  });

  function showToast(msg: string) {
    toast = msg;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 1800);
  }

  function openMenu(c: PlayerComment, e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    menuFor = c.id;
    menuPos = { x: e.clientX, y: e.clientY };
  }

  function closeMenu() {
    menuFor = null;
  }

  function ngBody(c: PlayerComment) {
    addNgRule({
      targetType: 'comment_body',
      matchMode: 'partial',
      pattern: c.content,
      scopeRanking: false,
      scopeSearch: false,
      scopeComment: true,
      enabled: true,
      note: `クイック追加: 「${c.content.slice(0, 24)}」`,
    });
    showToast('NG: コメ本文');
    closeMenu();
  }

  function ngBodyExact(c: PlayerComment) {
    addNgRule({
      targetType: 'comment_body',
      matchMode: 'exact',
      pattern: c.content,
      scopeRanking: false,
      scopeSearch: false,
      scopeComment: true,
      enabled: true,
      note: `クイック追加: 「${c.content.slice(0, 24)}」`,
    });
    showToast('NG: コメ本文（完全一致）');
    closeMenu();
  }

  function ngUser(c: PlayerComment) {
    if (!c.userId) return;
    addNgRule({
      targetType: 'comment_user',
      matchMode: 'exact',
      pattern: c.userId,
      scopeRanking: false,
      scopeSearch: false,
      scopeComment: true,
      enabled: true,
      note: `クイック追加: ユーザ ${c.userId}`,
    });
    showToast('NG: ユーザ');
    closeMenu();
  }

  function onDocClick(e: MouseEvent) {
    if (menuFor == null) return;
    if (menuEl?.contains(e.target as Node)) return;
    closeMenu();
  }

  onMount(() => document.addEventListener('mousedown', onDocClick));
  onDestroy(() => {
    document.removeEventListener('mousedown', onDocClick);
    if (toastTimer) clearTimeout(toastTimer);
  });

  let activeComment = $derived(menuFor ? (sorted.find((c) => c.id === menuFor) ?? null) : null);
</script>

<aside class="comment-list">
  <header>
    <span class="count">{sorted.length} 件</span>
    <label class="follow">
      <input type="checkbox" bind:checked={autoScroll} />
      追従
    </label>
  </header>
  <ul bind:this={listEl}>
    {#each sorted as c, i (c.id)}
      <li
        data-i={i}
        class:active={i === activeIndex}
        class:owner={c.isOwner}
        oncontextmenu={(e) => openMenu(c, e)}
      >
        <button type="button" class="time" onclick={() => onSeek(c.vposMs / 1000)}
          >{formatDuration(c.vposMs / 1000)}</button
        >
        {#if c.isOwner}<span class="badge owner">投稿者</span>{/if}
        <span class="content">{c.content}</span>
        <button
          type="button"
          class="ng-btn"
          title="NG メニュー（右クリックでも開けます）"
          aria-label="NG メニュー"
          onclick={(e) => openMenu(c, e)}>⋯</button
        >
      </li>
    {/each}
  </ul>
</aside>

{#if menuFor && activeComment}
  <div
    class="menu"
    bind:this={menuEl}
    role="menu"
    style:left="{menuPos.x}px"
    style:top="{menuPos.y}px"
  >
    <div class="menu-head">NG に追加</div>
    <div class="menu-preview">
      「{activeComment.content.slice(0, 40)}{activeComment.content.length > 40 ? '…' : ''}」
    </div>
    <button type="button" onclick={() => ngBody(activeComment!)}
      >このコメ本文を NG（部分一致）</button
    >
    <button type="button" onclick={() => ngBodyExact(activeComment!)}
      >このコメ本文を NG（完全一致）</button
    >
    <button
      type="button"
      onclick={() => ngUser(activeComment!)}
      disabled={!activeComment.userId}
      title={activeComment.userId ?? 'ユーザ ID 不明'}
    >
      このユーザを NG{activeComment.userId ? ` (${activeComment.userId.slice(0, 8)}…)` : ''}
    </button>
  </div>
{/if}

{#if toast}
  <div class="toast" role="status">{toast}</div>
{/if}

<style>
  .comment-list {
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border-left: 1px solid var(--border);
    overflow: hidden;
    width: 320px;
    flex: 0 0 320px;
    max-height: 70vh;
    align-self: stretch;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    color: var(--text-3);
  }
  .follow {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    cursor: pointer;
  }
  ul {
    flex: 1;
    overflow-y: auto;
    list-style: none;
    padding: 0;
    margin: 0;
  }
  li {
    display: grid;
    grid-template-columns: 56px max-content 1fr auto;
    gap: 8px;
    padding: 4px 12px;
    font-size: 12px;
    border-bottom: 1px solid var(--surface-3);
    align-items: baseline;
    position: relative;
  }
  li.active {
    background: var(--badge-blue-bg);
  }
  li.owner .content {
    color: #ffcc66;
  }
  .time {
    background: transparent;
    border: none;
    color: var(--link);
    cursor: pointer;
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    padding: 0;
    text-align: left;
  }
  .time:hover {
    text-decoration: underline;
  }
  .badge.owner {
    background: #5a3f00;
    color: #ffd58a;
    padding: 0 6px;
    border-radius: 999px;
    font-size: 10px;
  }
  .content {
    word-break: break-word;
    color: var(--text);
  }
  .ng-btn {
    background: transparent;
    border: none;
    color: var(--text-faint);
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
    padding: 0 4px;
    opacity: 0;
    transition: opacity 0.1s;
  }
  li:hover .ng-btn,
  li:focus-within .ng-btn {
    opacity: 1;
  }
  .ng-btn:hover {
    color: var(--error-text);
  }
  .menu {
    position: fixed;
    z-index: 1000;
    width: 280px;
    background: var(--menu-bg);
    border: 1px solid var(--menu-border);
    border-radius: 8px;
    box-shadow: var(--menu-shadow);
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    transform: translate(-95%, 0);
  }
  .menu-head {
    font-size: 11px;
    color: var(--text-muted);
    padding: 4px 8px 4px;
  }
  .menu-preview {
    font-size: 11px;
    color: var(--text-2);
    padding: 0 8px 6px;
    border-bottom: 1px solid var(--border-2);
    margin-bottom: 4px;
    word-break: break-all;
  }
  .menu button {
    text-align: left;
    background: transparent;
    border: none;
    color: var(--text);
    padding: 6px 8px;
    border-radius: 4px;
    font-size: 12px;
    cursor: pointer;
  }
  .menu button:hover:not(:disabled) {
    background: var(--surface-active);
  }
  .menu button:disabled {
    color: var(--text-faint);
    cursor: not-allowed;
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
