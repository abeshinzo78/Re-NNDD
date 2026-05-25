<script lang="ts">
  // コマンドパレット UI。`+layout.svelte` の末尾に <CommandPalette /> を 1 つ。
  // グローバル Ctrl/⌘+K で開く。Esc/外側クリックで閉じる。
  //
  // 内部 state は commandPalette.svelte.ts に集約 (テストで isPaletteOpen()
  // を assert したり、プラグインから「コマンドパレットを開く」を呼ぶ等の
  // 拡張に備える)。

  import { onDestroy, onMount, tick } from 'svelte';
  import { pluginCommands } from '$lib/plugins/registry';
  import {
    BUILTIN_COMMANDS,
    closePalette,
    isPaletteOpen,
    openPalette,
    rankCommands,
    type CommandEntry,
  } from '$lib/commandPaletteStore.svelte';
  import { showToast } from '$lib/toastStore.svelte';

  let query = $state('');
  let inputEl: HTMLInputElement | null = $state(null);
  let containerEl: HTMLDivElement | null = $state(null);
  let highlight = $state(0);

  let entries: CommandEntry[] = $derived(rankCommands(query, BUILTIN_COMMANDS, pluginCommands()));

  // highlight の範囲を entries の長さに clamp する。空 query や絞り込みで
  // entries が縮んだとき、ハイライト位置が末尾外に居る問題を防ぐ。
  $effect(() => {
    if (highlight >= entries.length) highlight = Math.max(0, entries.length - 1);
    if (highlight < 0) highlight = 0;
  });

  // パレット open 時に入力にフォーカス + 検索クエリと選択位置をリセット。
  $effect(() => {
    if (isPaletteOpen()) {
      query = '';
      highlight = 0;
      void tick().then(() => inputEl?.focus());
    }
  });

  function onKeydownGlobal(e: KeyboardEvent) {
    // フォーム要素にフォーカス中でも Ctrl/Cmd+K は開ける (ユーザの期待を優先)。
    if ((e.ctrlKey || e.metaKey) && (e.key === 'k' || e.key === 'K')) {
      e.preventDefault();
      if (isPaletteOpen()) {
        closePalette();
      } else {
        openPalette();
      }
    }
  }

  function onKeydownPanel(e: KeyboardEvent) {
    if (!isPaletteOpen()) return;
    switch (e.key) {
      case 'Escape':
        e.preventDefault();
        closePalette();
        return;
      case 'ArrowDown':
        e.preventDefault();
        if (entries.length > 0) highlight = (highlight + 1) % entries.length;
        return;
      case 'ArrowUp':
        e.preventDefault();
        if (entries.length > 0) highlight = (highlight - 1 + entries.length) % entries.length;
        return;
      case 'Enter':
        e.preventDefault();
        void runEntry(entries[highlight]);
        return;
    }
  }

  async function runEntry(entry: CommandEntry | undefined) {
    if (!entry) return;
    closePalette();
    try {
      await entry.handler();
    } catch (e) {
      console.error('[command palette] handler threw:', e);
      const label = entry.source === 'plugin' ? `プラグイン ${entry.pluginId}` : 'コマンド';
      showToast(`${label} の実行でエラー: ${e}`, 'error');
    }
  }

  function onBackdropClick(e: MouseEvent) {
    // パネル内クリックは閉じない。背景クリックで閉じる。
    if (!containerEl) return;
    if (containerEl.contains(e.target as Node)) return;
    closePalette();
  }

  onMount(() => {
    window.addEventListener('keydown', onKeydownGlobal);
  });
  onDestroy(() => {
    window.removeEventListener('keydown', onKeydownGlobal);
  });
</script>

<!-- パレットの ArrowUp/Down/Enter/Esc は <svelte:window> でだけ受ける。
     backdrop / panel にも onkeydown を貼ると、input から bubble してきた
     keydown を 2 度処理して highlight が 2 段ジャンプする (Codex r3298977867)。 -->
<svelte:window onkeydown={onKeydownPanel} />

{#if isPaletteOpen()}
  <!-- backdrop は role=presentation。実フォーカストラップは入力要素自体。 -->
  <div class="backdrop" role="presentation" onmousedown={onBackdropClick}>
    <div
      class="panel"
      bind:this={containerEl}
      role="dialog"
      aria-modal="true"
      aria-label="コマンドパレット"
    >
      <input
        bind:this={inputEl}
        type="text"
        class="search"
        placeholder="コマンドを検索… (Esc で閉じる)"
        bind:value={query}
        aria-controls="palette-list"
      />
      <ul id="palette-list" class="results" role="listbox">
        {#if entries.length === 0}
          <li class="empty">該当なし</li>
        {:else}
          {#each entries as e, i (e.source === 'plugin' ? `p:${e.pluginId}:${e.id}` : `b:${e.id}`)}
            <li
              class="row"
              class:active={i === highlight}
              role="option"
              aria-selected={i === highlight}
            >
              <button
                type="button"
                class="row-btn"
                onmouseenter={() => (highlight = i)}
                onclick={() => runEntry(e)}
              >
                <span class="title">{e.title}</span>
                {#if e.source === 'plugin'}
                  <span class="badge badge-plugin" title={e.pluginId}>プラグイン</span>
                {/if}
                {#if e.hint}<span class="hint">{e.hint}</span>{/if}
              </button>
            </li>
          {/each}
        {/if}
      </ul>
      <div class="footer">
        ↑↓ で移動 · Enter で実行 · Esc で閉じる
        <span class="footer-right">Ctrl/⌘+K で開閉</span>
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    z-index: 9100;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 12vh;
  }
  .panel {
    width: min(640px, 92vw);
    max-height: 70vh;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 12px;
    box-shadow: var(--theme-menu-shadow-strong);
    overflow: hidden;
    display: flex;
    flex-direction: column;
    animation: pop-in 140ms ease-out;
  }
  @keyframes pop-in {
    from {
      transform: translateY(-12px) scale(0.98);
      opacity: 0;
    }
    to {
      transform: translateY(0) scale(1);
      opacity: 1;
    }
  }
  .search {
    width: 100%;
    box-sizing: border-box;
    padding: 14px 16px;
    border: none;
    border-bottom: 1px solid var(--theme-border);
    background: transparent;
    color: var(--theme-text);
    font-size: 15px;
    outline: none;
  }
  .results {
    list-style: none;
    margin: 0;
    padding: 4px 0;
    overflow-y: auto;
    flex: 1;
  }
  .row {
    padding: 0;
  }
  .row-btn {
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    color: var(--theme-text);
    padding: 8px 16px;
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    font-size: 14px;
  }
  .row.active .row-btn,
  .row-btn:focus {
    background: var(--theme-nav-active);
  }
  .row-btn:hover {
    background: var(--theme-nav-hover);
  }
  .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    color: var(--theme-text-muted);
    font-size: 12px;
    flex-shrink: 0;
  }
  .badge {
    background: var(--theme-chip-bg);
    color: var(--theme-chip-text);
    padding: 1px 7px;
    border-radius: 999px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    flex-shrink: 0;
  }
  .badge-plugin {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
    border: 1px solid var(--theme-accent-border);
  }
  .empty {
    padding: 20px 16px;
    color: var(--theme-text-muted);
    text-align: center;
    font-size: 13px;
  }
  .footer {
    padding: 8px 14px;
    border-top: 1px solid var(--theme-border);
    background: var(--theme-surface-3);
    color: var(--theme-text-muted);
    font-size: 11px;
    display: flex;
    justify-content: space-between;
  }
</style>
