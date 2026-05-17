<script lang="ts">
  import { page } from '$app/state';
  import { onMount } from 'svelte';
  import { installConsoleBridge } from '$lib/consoleBridge';
  import { getStr, loadSettings } from '$lib/stores/settings.svelte';
  import MiniPlayer from '$lib/player/MiniPlayer.svelte';

  let { children } = $props();

  onMount(() => {
    installConsoleBridge();
    void loadSettings();
  });

  const sections = [
    { href: '/', label: 'ホーム' },
    { href: '/library', label: 'ローカル' },
    { href: '/search', label: '検索' },
    { href: '/playlists', label: 'プレイリスト' },
    { href: '/downloads', label: 'ダウンロード' },
    { href: '/history', label: '履歴' },
    { href: '/ng', label: 'NG' },
    { href: '/settings', label: '設定' },
  ];

  let canGoBack = $derived(
    page.url.pathname !== '/' &&
      !page.url.pathname.startsWith('/video/') &&
      !page.url.pathname.startsWith('/library/'),
  );
  let theme = $derived(getStr('appearance.theme'));

  $effect(() => {
    if (typeof document === 'undefined') return;
    document.documentElement.dataset.theme = theme;
    document.body.dataset.theme = theme;
  });
</script>

<div class="app">
  <aside class="sidebar">
    <h1 class="brand">Re:NNDD</h1>
    {#if canGoBack}
      <button class="back-btn" onclick={() => history.back()}>← 戻る</button>
    {/if}
    <nav>
      {#each sections as section (section.href)}
        <a class="nav-item" class:active={page.url.pathname === section.href} href={section.href}
          >{section.label}</a
        >
      {/each}
    </nav>
  </aside>
  <main class="content">
    {@render children()}
  </main>
</div>

<MiniPlayer />

<style>
  :global(html) {
    color-scheme: dark;
    --theme-font:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Hiragino Sans', 'Yu Gothic',
      sans-serif;
    --theme-bg: #000000;
    --theme-bg-gradient:
      radial-gradient(circle at top, rgba(50, 62, 84, 0.35), transparent 42%), #000000;
    --theme-surface: #121212;
    --theme-surface-2: #161616;
    --theme-surface-3: #1a1a1a;
    --theme-surface-4: #181818;
    --theme-surface-hover: #2a2a2a;
    --theme-input-bg: #0f0f0f;
    --theme-border: #1f1f1f;
    --theme-border-strong: #2a2a2a;
    --theme-border-focus: #5a5a5a;
    --theme-text: #eaeaea;
    --theme-text-soft: #cfcfcf;
    --theme-text-muted: #9a9a9a;
    --theme-text-faint: #666666;
    --theme-placeholder: #6a6a6a;
    --theme-accent: #2563eb;
    --theme-accent-hover: #3b78f0;
    --theme-accent-soft: #93c5fd;
    --theme-accent-bg: #1a2a44;
    --theme-accent-border: #2a3f5a;
    --theme-danger-bg: #2a1212;
    --theme-danger-bg-2: #2a1f1a;
    --theme-danger-border: #5a2222;
    --theme-danger-text: #f5b3b3;
    --theme-success-bg: #1a3a26;
    --theme-success-bg-2: #1a2a1a;
    --theme-success-border: #2a5a3a;
    --theme-success-border-2: #2a5a2a;
    --theme-success-text: #b3f5b3;
    --theme-success-strong: #4ade80;
    --theme-warning-bg: #2a2418;
    --theme-warning-border: #5a4a1a;
    --theme-warning-text: #fde68a;
    --theme-chip-bg: #2a2a2a;
    --theme-chip-text: #c0c0c0;
    --theme-sidebar-bg: linear-gradient(
      180deg,
      rgba(26, 31, 42, 0.96) 0%,
      rgba(12, 12, 12, 0.98) 100%
    );
    --theme-sidebar-border: #2a2a2a;
    --theme-nav-hover: #1f1f1f;
    --theme-nav-active: #2a2a2a;
    --theme-content-bg: var(--theme-bg-gradient);
  }
  :global(html[data-theme='niconico-classic']) {
    color-scheme: light;
    --theme-font: 'Hiragino Sans', 'Yu Gothic', 'Meiryo', 'MS PGothic', 'Segoe UI', sans-serif;
    --theme-bg: #f3f3f3;
    --theme-bg-gradient: linear-gradient(180deg, #f7f7f7 0%, #efefef 100%);
    --theme-surface: #fafafa;
    --theme-surface-2: #ffffff;
    --theme-surface-3: #f5f5f5;
    --theme-surface-4: #ececec;
    --theme-surface-hover: #e4e4e4;
    --theme-input-bg: #ffffff;
    --theme-border: #d3d3d3;
    --theme-border-strong: #bdbdbd;
    --theme-border-focus: #6f98c5;
    --theme-text: #251d17;
    --theme-text-soft: #444444;
    --theme-text-muted: #666666;
    --theme-text-faint: #8d8d8d;
    --theme-placeholder: #9a9a9a;
    --theme-accent: #4b7db8;
    --theme-accent-hover: #5e8fca;
    --theme-accent-soft: #3f73b3;
    --theme-accent-bg: #eef4ff;
    --theme-accent-border: #bfd0e7;
    --theme-danger-bg: #f9e7e7;
    --theme-danger-bg-2: #f6dddd;
    --theme-danger-border: #d8b1b1;
    --theme-danger-text: #7e2020;
    --theme-success-bg: #edf5ea;
    --theme-success-bg-2: #f4f8f2;
    --theme-success-border: #bfd0b3;
    --theme-success-border-2: #bfd0b3;
    --theme-success-text: #355f2e;
    --theme-success-strong: #6b9c4a;
    --theme-warning-bg: #f9f1df;
    --theme-warning-border: #ddc793;
    --theme-warning-text: #7f5a13;
    --theme-chip-bg: #f0f0f0;
    --theme-chip-text: #555555;
    --theme-sidebar-bg: linear-gradient(
      180deg,
      rgba(255, 255, 255, 0.96) 0%,
      rgba(243, 243, 243, 0.98) 100%
    );
    --theme-sidebar-border: #d3d3d3;
    --theme-nav-hover: #ededed;
    --theme-nav-active: #e3e3e3;
    --theme-content-bg: var(--theme-bg-gradient);
  }
  :global(html, body) {
    margin: 0;
    padding: 0;
    height: 100%;
    background: var(--theme-bg);
    color: var(--theme-text);
    font-family: var(--theme-font);
  }
  :global(body) {
    background-image: var(--theme-bg-gradient);
  }
  :global(select option) {
    background: var(--theme-surface-2);
    color: var(--theme-text);
  }
  :global(input::placeholder) {
    color: var(--theme-placeholder);
  }
  :global(a) {
    color: var(--theme-accent-soft);
  }
  :global(button),
  :global(input),
  :global(select),
  :global(textarea) {
    font: inherit;
  }

  .app {
    display: grid;
    grid-template-columns: 200px 1fr;
    height: 100vh;
  }

  .sidebar {
    background: var(--theme-sidebar-bg);
    border-right: 1px solid var(--theme-sidebar-border);
    padding: 16px 12px;
    overflow-y: auto;
    backdrop-filter: blur(6px);
  }

  .brand {
    font-size: 18px;
    font-weight: 600;
    margin: 0 0 16px;
    padding: 0 8px;
    color: var(--theme-text);
  }

  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .back-btn {
    display: block;
    width: 100%;
    padding: 8px 12px;
    color: var(--theme-text-muted);
    background: transparent;
    border: none;
    border-radius: 6px;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    margin-bottom: 8px;
  }
  .back-btn:hover {
    background: var(--theme-nav-hover);
    color: var(--theme-text);
  }

  .nav-item {
    display: block;
    padding: 8px 12px;
    color: var(--theme-text-soft);
    text-decoration: none;
    border-radius: 6px;
    font-size: 14px;
  }

  .nav-item:hover {
    background: var(--theme-nav-hover);
    color: var(--theme-text);
  }

  .nav-item.active {
    background: var(--theme-nav-active);
    color: var(--theme-text);
  }

  .content {
    overflow: auto;
    padding: 24px;
    background: var(--theme-content-bg);
  }
</style>
