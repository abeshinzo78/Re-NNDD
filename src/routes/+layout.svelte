<script lang="ts">
  import { page } from '$app/state';
  import { onMount } from 'svelte';
  import { installConsoleBridge } from '$lib/consoleBridge';
  import { loadSettings } from '$lib/stores/settings.svelte';

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
</script>

<div class="app">
  <aside class="sidebar">
    <h1 class="brand">Re:NNDD</h1>
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

<style>
  :global(html) {
    /* スクロールバーや autofill 用に dark スキームを宣言。
       ただし <select> の popup (option list) は下で明示的に light 系に
       戻して、ユーザが選択肢を読めるようにしている。 */
    color-scheme: dark;
  }
  :global(html, body) {
    margin: 0;
    padding: 0;
    height: 100%;
    background: #000;
    color: #eaeaea;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Hiragino Sans', 'Yu Gothic',
      sans-serif;
  }
  /* <select> の popup (option) は **黒文字 / 白背景** にする。
     アプリ自体はダークだが、選択肢ポップアップだけは OS ネイティブの
     ライト表示にして読めるようにする (ユーザリクエスト)。 */
  :global(select option) {
    background: #ffffff;
    color: #000000;
  }
  :global(input::placeholder) {
    color: #6a6a6a;
  }

  .app {
    display: grid;
    grid-template-columns: 200px 1fr;
    height: 100vh;
  }

  .sidebar {
    background: #121212;
    border-right: 1px solid #2a2a2a;
    padding: 16px 12px;
    overflow-y: auto;
  }

  .brand {
    font-size: 18px;
    font-weight: 600;
    margin: 0 0 16px;
    padding: 0 8px;
    color: #f5f5f5;
  }

  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .nav-item {
    display: block;
    padding: 8px 12px;
    color: #cfcfcf;
    text-decoration: none;
    border-radius: 6px;
    font-size: 14px;
  }

  .nav-item:hover {
    background: #1f1f1f;
    color: #fff;
  }

  .nav-item.active {
    background: #2a2a2a;
    color: #fff;
  }

  .content {
    overflow: auto;
    padding: 24px;
    background: #000;
  }
</style>
