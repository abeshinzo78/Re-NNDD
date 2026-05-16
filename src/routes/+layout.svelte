<script lang="ts">
  import { page } from '$app/state';
  import { onMount } from 'svelte';
  import { installConsoleBridge } from '$lib/consoleBridge';
  import { get, loadSettings } from '$lib/stores/settings.svelte';
  import { applyTheme, DEFAULT_THEME } from '$lib/theme';
  import MiniPlayer from '$lib/player/MiniPlayer.svelte';

  let { children } = $props();

  onMount(() => {
    installConsoleBridge();
    // 設定 load 前に既定テーマを当てておく (チラつき防止)。
    applyTheme(DEFAULT_THEME);
    void loadSettings();
  });

  // 設定キャッシュは $state なので、変更が `get` 経由で reactive に反映される。
  // 設定画面でテーマを切り替えると即座にここで再適用される。
  $effect(() => {
    const id = String(get('appearance.theme') ?? DEFAULT_THEME);
    applyTheme(id);
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
  /* テーマ用の CSS 変数。
     JS (applyTheme) でも投入するが、SSR / 初期描画時のチラつきを抑えるため
     ここにも静的に書いておく。data-theme 属性で切り替わる。 */
  :global([data-theme='niconico']),
  :global(html) {
    --color-scheme: dark;
    --bg: #000000;
    --surface: #121212;
    --surface-2: #161616;
    --surface-3: #1a1a1a;
    --surface-hover: #1f1f1f;
    --surface-active: #2a2a2a;
    --input-bg: #0f0f0f;
    --code-bg: #0a0a0a;
    --border: #1f1f1f;
    --border-2: #2a2a2a;
    --border-3: #2f2f2f;
    --border-strong: #3a3a3a;
    --text: #eaeaea;
    --text-2: #cfcfcf;
    --text-3: #b0b0b0;
    --text-muted: #9a9a9a;
    --text-dim: #6a6a6a;
    --text-faint: #555555;
    --text-heading: #f5f5f5;
    --accent: #2563eb;
    --accent-hover: #3b78f0;
    --accent-text: #ffffff;
    --accent-soft-bg: rgba(37, 99, 235, 0.15);
    --accent-soft-border: #2a3f5a;
    --accent-soft-text: #c5d8f5;
    --link: #6ea8fe;
    --link-strong: #93c5fd;
    --badge-blue-bg: #1f2a44;
    --badge-blue-text: #93c5fd;
    --badge-blue-bg-soft: #1a2a44;
    --badge-blue-border: #2a3f5a;
    --success-bg: #102d20;
    --success-text: #bbf7d0;
    --success-border: #1e6b48;
    --success-strong: #4ade80;
    --success-icon-bg: #1a3a26;
    --success-icon-text: #b3f5b3;
    --success-icon-border: #2a5a3a;
    --warn-bg: #2a2410;
    --warn-text: #fde68a;
    --warn-border: #5a4a1a;
    --error-bg: #2a1212;
    --error-text: #f5b3b3;
    --error-border: #5a2222;
    --error-strong: #f87171;
    --error-hover-bg: #3a1a1a;
    --tag-bg: #1f1f1f;
    --tag-text: #c0c0c0;
    --tag-hover-bg: #2a2a2a;
    --tag-hover-text: #ffffff;
    --tag-hover-border: #3a3a3a;
    --tag-locked-bg: #1e2a3a;
    --tag-locked-text: #b3c5ff;
    --tag-locked-hover-bg: #243246;
    --menu-bg: #181818;
    --menu-border: #333333;
    --menu-shadow: 0 8px 24px rgba(0, 0, 0, 0.6);
    --owner-card-bg: #161616;
    --resume-bg-overlay: rgba(0, 0, 0, 0.7);
    --resume-bar: #333333;
    --resume-bar-fill: #93c5fd;
    --resume-text: #c5d8f5;
    --select-option-bg: #ffffff;
    --select-option-text: #000000;
  }

  :global([data-theme='dark']) {
    --color-scheme: dark;
    --bg: #000000;
    --surface: #121212;
    --surface-2: #161616;
    --surface-3: #1a1a1a;
    --surface-hover: #1f1f1f;
    --surface-active: #2a2a2a;
    --input-bg: #0f0f0f;
    --code-bg: #0a0a0a;
    --border: #1f1f1f;
    --border-2: #2a2a2a;
    --border-3: #2f2f2f;
    --border-strong: #3a3a3a;
    --text: #eaeaea;
    --text-2: #cfcfcf;
    --text-3: #b0b0b0;
    --text-muted: #9a9a9a;
    --text-dim: #6a6a6a;
    --text-faint: #555555;
    --text-heading: #f5f5f5;
    --accent: #2563eb;
    --accent-hover: #3b78f0;
    --accent-text: #ffffff;
    --accent-soft-bg: rgba(37, 99, 235, 0.15);
    --accent-soft-border: #2a3f5a;
    --accent-soft-text: #c5d8f5;
    --link: #6ea8fe;
    --link-strong: #93c5fd;
    --badge-blue-bg: #1f2a44;
    --badge-blue-text: #93c5fd;
    --badge-blue-bg-soft: #1a2a44;
    --badge-blue-border: #2a3f5a;
    --success-bg: #102d20;
    --success-text: #bbf7d0;
    --success-border: #1e6b48;
    --success-strong: #4ade80;
    --success-icon-bg: #1a3a26;
    --success-icon-text: #b3f5b3;
    --success-icon-border: #2a5a3a;
    --warn-bg: #2a2410;
    --warn-text: #fde68a;
    --warn-border: #5a4a1a;
    --error-bg: #2a1212;
    --error-text: #f5b3b3;
    --error-border: #5a2222;
    --error-strong: #f87171;
    --error-hover-bg: #3a1a1a;
    --tag-bg: #1f1f1f;
    --tag-text: #c0c0c0;
    --tag-hover-bg: #2a2a2a;
    --tag-hover-text: #ffffff;
    --tag-hover-border: #3a3a3a;
    --tag-locked-bg: #1e2a3a;
    --tag-locked-text: #b3c5ff;
    --tag-locked-hover-bg: #243246;
    --menu-bg: #181818;
    --menu-border: #333333;
    --menu-shadow: 0 8px 24px rgba(0, 0, 0, 0.6);
    --owner-card-bg: #161616;
    --resume-bg-overlay: rgba(0, 0, 0, 0.7);
    --resume-bar: #333333;
    --resume-bar-fill: #93c5fd;
    --resume-text: #c5d8f5;
    --select-option-bg: #ffffff;
    --select-option-text: #000000;
  }

  :global([data-theme='light']) {
    --color-scheme: light;
    --bg: #f5f7fa;
    --surface: #ffffff;
    --surface-2: #ffffff;
    --surface-3: #f3f5f8;
    --surface-hover: #e9edf2;
    --surface-active: #dde3eb;
    --input-bg: #ffffff;
    --code-bg: #f1f3f7;
    --border: #e2e6ec;
    --border-2: #d3d8e0;
    --border-3: #c8cdd5;
    --border-strong: #b3b9c2;
    --text: #1f2937;
    --text-2: #374151;
    --text-3: #4b5563;
    --text-muted: #6b7280;
    --text-dim: #9ca3af;
    --text-faint: #b9bfc8;
    --text-heading: #111827;
    --accent: #2563eb;
    --accent-hover: #1d4ed8;
    --accent-text: #ffffff;
    --accent-soft-bg: rgba(37, 99, 235, 0.12);
    --accent-soft-border: #bfd2f5;
    --accent-soft-text: #1e40af;
    --link: #1d4ed8;
    --link-strong: #1e40af;
    --badge-blue-bg: #dbeafe;
    --badge-blue-text: #1d4ed8;
    --badge-blue-bg-soft: #e0eaff;
    --badge-blue-border: #bfd2f5;
    --success-bg: #dcfce7;
    --success-text: #065f46;
    --success-border: #86efac;
    --success-strong: #16a34a;
    --success-icon-bg: #d1fae5;
    --success-icon-text: #065f46;
    --success-icon-border: #6ee7b7;
    --warn-bg: #fef3c7;
    --warn-text: #854d0e;
    --warn-border: #fcd34d;
    --error-bg: #fee2e2;
    --error-text: #991b1b;
    --error-border: #fca5a5;
    --error-strong: #dc2626;
    --error-hover-bg: #fecaca;
    --tag-bg: #eef0f4;
    --tag-text: #374151;
    --tag-hover-bg: #dde2e8;
    --tag-hover-text: #111827;
    --tag-hover-border: #c8cdd5;
    --tag-locked-bg: #e0eaff;
    --tag-locked-text: #1e40af;
    --tag-locked-hover-bg: #cad9fa;
    --menu-bg: #ffffff;
    --menu-border: #d3d8e0;
    --menu-shadow: 0 8px 24px rgba(15, 23, 42, 0.15);
    --owner-card-bg: #ffffff;
    --resume-bg-overlay: rgba(15, 23, 42, 0.55);
    --resume-bar: #cbd5e1;
    --resume-bar-fill: #1d4ed8;
    --resume-text: #e0eaff;
    --select-option-bg: #ffffff;
    --select-option-text: #000000;
  }

  :global(html) {
    /* スクロールバーや autofill 用のスキーム宣言。
       テーマで dark/light を切り替え。 */
    color-scheme: var(--color-scheme, dark);
  }
  :global(html, body) {
    margin: 0;
    padding: 0;
    height: 100%;
    background: var(--bg);
    color: var(--text);
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Hiragino Sans', 'Yu Gothic',
      sans-serif;
  }
  /* <select> の popup (option) は OS ネイティブの見た目を維持。
     ライトテーマでもダークでも常に読みやすい白背景 / 黒文字に固定。 */
  :global(select option) {
    background: var(--select-option-bg);
    color: var(--select-option-text);
  }
  :global(input::placeholder) {
    color: var(--text-dim);
  }

  .app {
    display: grid;
    grid-template-columns: 200px 1fr;
    height: 100vh;
  }

  .sidebar {
    background: var(--surface);
    border-right: 1px solid var(--surface-active);
    padding: 16px 12px;
    overflow-y: auto;
  }

  .brand {
    font-size: 18px;
    font-weight: 600;
    margin: 0 0 16px;
    padding: 0 8px;
    color: var(--text-heading);
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
    color: var(--text-muted);
    background: transparent;
    border: none;
    border-radius: 6px;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    margin-bottom: 8px;
  }
  .back-btn:hover {
    background: var(--surface-hover);
    color: var(--text);
  }

  .nav-item {
    display: block;
    padding: 8px 12px;
    color: var(--text-2);
    text-decoration: none;
    border-radius: 6px;
    font-size: 14px;
  }

  .nav-item:hover {
    background: var(--surface-hover);
    color: var(--text-heading);
  }

  .nav-item.active {
    background: var(--surface-active);
    color: var(--text-heading);
  }

  .content {
    overflow: auto;
    padding: 24px;
    background: var(--bg);
  }
</style>
