<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import type { SearchHit } from '$lib/api';
  import { formatDate, formatDuration, formatNumber, videoUrl } from '$lib/format';
  import { addNgRule, type NgTargetType } from '$lib/stores/ngRules';
  import { quickDownload } from '$lib/quickDownload';

  type Props = {
    hit: SearchHit;
    compact?: boolean;
    onClick?: () => void;
  };

  let { hit, compact = false, onClick }: Props = $props();

  let playerHref = $derived(hit.contentId ? `/video/${hit.contentId}` : null);
  let externalHref = $derived(videoUrl(hit.contentId));

  let menuOpen = $state(false);
  let menuButtonEl: HTMLButtonElement | null = $state(null);
  let menuEl: HTMLDivElement | null = $state(null);
  let toast = $state<string | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  let tagsList = $derived(hit.tags ? hit.tags.split(/\s+/).filter(Boolean) : []);
  let uploaderPattern = $derived(
    hit.userId != null
      ? `user/${hit.userId}`
      : hit.channelId != null
        ? `channel/${hit.channelId}`
        : null,
  );

  let uploaderHref = $derived(
    hit.userId != null
      ? `/user/${hit.userId}?kind=user`
      : hit.channelId != null
        ? `/user/${hit.channelId}?kind=channel`
        : null,
  );

  function showToast(msg: string) {
    toast = msg;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 1800);
  }

  let dlPending = $state(false);
  async function onDownload() {
    if (!hit.contentId) return;
    dlPending = true;
    const r = await quickDownload(hit.contentId);
    showToast(r.message);
    dlPending = false;
  }

  function ng(targetType: NgTargetType, pattern: string, label: string) {
    const scopeComment = targetType === 'comment_body' || targetType === 'comment_user';
    addNgRule({
      targetType,
      matchMode: targetType === 'video_title' ? 'partial' : 'exact',
      pattern,
      scopeRanking: !scopeComment,
      scopeSearch: !scopeComment,
      scopeComment,
      enabled: true,
      note: `クイック追加: ${label}`,
    });
    showToast(`NG 追加: ${label}`);
    menuOpen = false;
  }

  function onDocClick(e: MouseEvent) {
    if (!menuOpen) return;
    const t = e.target as Node;
    if (menuEl?.contains(t) || menuButtonEl?.contains(t)) return;
    menuOpen = false;
  }

  onMount(() => document.addEventListener('mousedown', onDocClick));
  onDestroy(() => {
    document.removeEventListener('mousedown', onDocClick);
    if (toastTimer) clearTimeout(toastTimer);
  });
</script>

<li class="hit" class:compact>
  {#if hit.thumbnailUrl}
    {#if playerHref}
      <a href={playerHref} onclick={onClick}>
        <img class="thumb" src={hit.thumbnailUrl} alt="" loading="lazy" decoding="async" />
      </a>
    {:else}
      <img class="thumb" src={hit.thumbnailUrl} alt="" loading="lazy" decoding="async" />
    {/if}
  {:else}
    <div class="thumb placeholder"></div>
  {/if}
  <div class="info">
    <div class="title">
      {#if playerHref}
        <a href={playerHref} onclick={onClick}>{hit.title ?? '(無題)'}</a>
      {:else}
        {hit.title ?? '(無題)'}
      {/if}
      {#if externalHref}
        <a
          class="external"
          href={externalHref}
          target="_blank"
          rel="noreferrer noopener"
          title="ニコニコで開く">↗</a
        >
      {/if}
    </div>
    <div class="row-meta muted">
      <span>{hit.contentId ?? ''}</span>
      {#if hit.lengthSeconds != null}<span class="dot">·</span><span
          >{formatDuration(hit.lengthSeconds)}</span
        >{/if}
      {#if hit.startTime}<span class="dot">·</span><span>{formatDate(hit.startTime)}</span>{/if}
    </div>
    <div class="row-meta">
      <span>再生 {formatNumber(hit.viewCounter)}</span>
      <span class="dot">·</span>
      <span>コメ {formatNumber(hit.commentCounter)}</span>
      <span class="dot">·</span>
      <span>マイリスト {formatNumber(hit.mylistCounter)}</span>
    </div>
    {#if tagsList.length > 0 && !compact}
      <div class="tags">
        {#each tagsList as tag (tag)}
          <span class="tag">{tag}</span>
        {/each}
      </div>
    {/if}
  </div>

  <div class="menu-wrap">
    {#if hit.contentId}
      <button
        type="button"
        class="dl-icon-btn"
        disabled={dlPending}
        onclick={onDownload}
        aria-label="この動画を DL"
        title="ライブラリにダウンロード"
      >
        {dlPending ? '⏳' : '⬇'}
      </button>
    {/if}
    <button
      type="button"
      class="menu-btn"
      bind:this={menuButtonEl}
      onclick={() => (menuOpen = !menuOpen)}
      aria-label="NG メニュー"
      aria-haspopup="true"
      aria-expanded={menuOpen}
      title="NG に追加">⋯</button
    >
    {#if menuOpen}
      <div class="menu" bind:this={menuEl} role="menu">
        <div class="menu-head">NG に追加</div>
        {#if hit.contentId}
          <button type="button" onclick={() => ng('video_id', hit.contentId!, hit.contentId!)}>
            この動画 ID を NG
          </button>
        {/if}
        {#if hit.title}
          <button
            type="button"
            onclick={() => ng('video_title', hit.title!, `タイトル「${hit.title}」`)}
          >
            このタイトルで NG（部分一致）
          </button>
        {/if}
        {#if uploaderPattern}
          <button type="button" onclick={() => ng('uploader', uploaderPattern!, uploaderPattern!)}>
            この投稿者 ({uploaderPattern}) を NG
          </button>
        {/if}
        {#if uploaderHref}
          <a href={uploaderHref} class="menu-link"> 投稿者の動画一覧 </a>
        {/if}
        {#if tagsList.length > 0}
          <div class="menu-sep">タグ</div>
          {#each tagsList.slice(0, 8) as tag (tag)}
            <button type="button" class="tag-row" onclick={() => ng('tag', tag, `タグ「${tag}」`)}>
              # {tag}
            </button>
          {/each}
        {/if}
      </div>
    {/if}
  </div>

  {#if toast}
    <div class="toast" role="status">{toast}</div>
  {/if}
</li>

<style>
  .hit {
    position: relative;
    display: grid;
    grid-template-columns: 160px 1fr;
    gap: 12px;
    padding: 8px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    list-style: none;
  }
  .hit.compact {
    grid-template-columns: 120px 1fr;
    padding: 6px;
    gap: 8px;
  }
  .thumb {
    width: 160px;
    height: 90px;
    object-fit: cover;
    background: var(--code-bg);
    border-radius: 4px;
  }
  .compact .thumb {
    width: 120px;
    height: 68px;
  }
  .thumb.placeholder {
    border: 1px dashed var(--border-2);
  }
  .muted {
    color: var(--text-muted);
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .title {
    padding-right: 28px;
  }
  .title a {
    color: var(--text);
    text-decoration: none;
    font-weight: 600;
  }
  .title a:hover {
    text-decoration: underline;
  }
  .title .external {
    color: var(--link);
    margin-left: 6px;
    font-weight: 400;
    text-decoration: none;
  }
  .title .external:hover {
    text-decoration: underline;
  }
  .row-meta {
    font-size: 12px;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    align-items: center;
  }
  .compact .row-meta {
    font-size: 11px;
  }
  .dot {
    color: var(--text-faint);
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
  }
  .tag {
    background: var(--tag-bg);
    color: var(--tag-text);
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
  }

  .menu-wrap {
    position: absolute;
    top: 6px;
    right: 6px;
    display: flex;
    gap: 4px;
  }
  .dl-icon-btn {
    background: var(--success-icon-bg);
    color: var(--success-icon-text);
    border: 1px solid var(--success-icon-border);
    border-radius: 6px;
    padding: 0 8px;
    font-size: 14px;
    line-height: 22px;
    cursor: pointer;
  }
  .dl-icon-btn:hover:not(:disabled) {
    background: var(--success-icon-border);
    color: var(--text-heading);
  }
  .dl-icon-btn:disabled {
    opacity: 0.6;
    cursor: wait;
  }
  .menu-btn {
    background: var(--surface-3);
    color: var(--text-2);
    border: 1px solid var(--border-2);
    border-radius: 6px;
    padding: 0 8px;
    font-size: 16px;
    line-height: 22px;
    cursor: pointer;
  }
  .menu-btn:hover {
    background: var(--surface-active);
    color: var(--text-heading);
  }
  .menu {
    position: absolute;
    top: 28px;
    right: 0;
    z-index: 30;
    width: 240px;
    background: var(--menu-bg);
    border: 1px solid var(--menu-border);
    border-radius: 8px;
    box-shadow: var(--menu-shadow);
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .menu-head {
    font-size: 11px;
    color: var(--text-muted);
    padding: 4px 8px 6px;
    border-bottom: 1px solid var(--border-2);
    margin-bottom: 4px;
  }
  .menu button,
  .menu .menu-link {
    display: block;
    text-align: left;
    background: transparent;
    border: none;
    color: var(--text);
    padding: 6px 8px;
    border-radius: 4px;
    font-size: 12px;
    cursor: pointer;
    text-decoration: none;
  }
  .menu button:hover,
  .menu .menu-link:hover {
    background: var(--surface-active);
  }
  .menu-sep {
    font-size: 10px;
    color: var(--text-dim);
    padding: 6px 8px 2px;
    margin-top: 4px;
    border-top: 1px solid var(--border-2);
    text-transform: uppercase;
  }
  .tag-row {
    color: var(--tag-text) !important;
    font-size: 11px !important;
  }
  .toast {
    position: absolute;
    top: 6px;
    right: 40px;
    background: var(--success-icon-bg);
    color: var(--success-icon-text);
    border: 1px solid var(--success-icon-border);
    padding: 3px 10px;
    border-radius: 6px;
    font-size: 11px;
    pointer-events: none;
    z-index: 40;
  }
</style>
