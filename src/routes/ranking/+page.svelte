<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { extractAndParse, GENRE_KEY_BY_NAME, type RankingItem } from '@kongyo2/nicoran-api';
  import { formatNumber, formatDate, formatDuration, videoUrl } from '$lib/format';
  import { quickDownload } from '$lib/quickDownload';
  import {
    addNgRule,
    listNgRules,
    subscribeNgRules,
    type NgRule,
    type NgTargetType,
  } from '$lib/stores/ngRules';

  type GenreName = keyof typeof GENRE_KEY_BY_NAME;
  type Term = 'hour' | '24h' | 'week' | 'month' | 'total';

  const GENRES: { value: GenreName; label: string }[] = [
    { value: 'all', label: '総合' },
    { value: 'game', label: 'ゲーム' },
    { value: 'anime', label: 'アニメ' },
    { value: 'vocaloid', label: 'VOCALOID' },
    { value: 'voicesynth', label: '音声合成' },
    { value: 'entertainment', label: 'エンタメ' },
    { value: 'music', label: '音楽・サウンド' },
    { value: 'sing', label: '歌ってみた' },
    { value: 'dance', label: '踊ってみた' },
    { value: 'play', label: '演奏してみた' },
    { value: 'commentary', label: '解説' },
    { value: 'cooking', label: '料理' },
    { value: 'travel', label: '旅行' },
    { value: 'nature', label: '自然' },
    { value: 'vehicle', label: '乗り物' },
    { value: 'technology', label: 'テクノロジー' },
    { value: 'society', label: '社会' },
    { value: 'mmd', label: 'MMD' },
    { value: 'vtuber', label: 'VTuber' },
    { value: 'radio', label: 'ラジオ' },
    { value: 'sports', label: 'スポーツ' },
    { value: 'animal', label: '動物' },
    { value: 'other', label: 'その他' },
  ];

  const TERMS: { value: Term; label: string }[] = [
    { value: 'hour', label: '毎時' },
    { value: '24h', label: '24時間' },
    { value: 'week', label: '週間' },
    { value: 'month', label: '月間' },
    { value: 'total', label: '合計' },
  ];

  let genre = $state<GenreName>('all');
  let term = $state<Term>('24h');
  let page = $state(1);

  let pending = $state(false);
  let error = $state<string | null>(null);
  let items = $state<RankingItem[]>([]);
  let hasNext = $state(false);
  let label = $state('');
  let fetchedAt = $state<string | null>(null);

  let ngRules = $state<NgRule[]>([]);
  let ngUnsub: (() => void) | null = null;

  let displayed = $derived(applyNgFilter(items, ngRules));
  let blockedCount = $derived(items.length - displayed.length);

  function applyNgFilter(rankingItems: RankingItem[], rules: NgRule[]): RankingItem[] {
    const videoIdRules = rules.filter((r) => r.targetType === 'video_id' && r.enabled);
    const titleRules = rules.filter((r) => r.targetType === 'video_title' && r.enabled);

    return rankingItems.filter((item) => {
      for (const r of videoIdRules) {
        if (r.matchMode === 'exact' && item.id === r.pattern) return false;
        if (r.matchMode === 'partial' && item.id.includes(r.pattern)) return false;
      }
      for (const r of titleRules) {
        if (r.matchMode === 'exact' && item.title === r.pattern) return false;
        if (r.matchMode === 'partial' && item.title.includes(r.pattern)) return false;
      }
      return true;
    });
  }

  onMount(() => {
    ngRules = listNgRules();
    ngUnsub = subscribeNgRules(() => (ngRules = listNgRules()));
    void runFetch();

    return () => {
      ngUnsub?.();
    };
  });

  async function runFetch() {
    pending = true;
    error = null;
    try {
      const genreKey = GENRE_KEY_BY_NAME[genre];
      const params = new URLSearchParams({ term, page: String(page) });
      const url = `https://www.nicovideo.jp/ranking/genre/${genreKey}?${params}`;
      const html = await invoke<string>('fetch_ranking_html', { url });
      const { parsed } = extractAndParse(html);
      const ranking = parsed.data.response.$getTeibanRanking.data;
      items = ranking.items;
      hasNext = ranking.hasNext;
      label = ranking.label;
      fetchedAt = new Date().toISOString();
    } catch (e) {
      error = String(e);
      items = [];
      hasNext = false;
    } finally {
      pending = false;
    }
  }

  function onChangeGenre(g: GenreName) {
    genre = g;
    page = 1;
    void runFetch();
  }

  function onChangeTerm(t: Term) {
    term = t;
    page = 1;
    void runFetch();
  }

  function goPage(p: number) {
    page = p;
    void runFetch();
  }

  let dlPending = $state<Set<string>>(new Set());

  async function onDownload(id: string) {
    dlPending = new Set([...dlPending, id]);
    await quickDownload(id);
    dlPending = new Set([...dlPending].filter((x) => x !== id));
  }

  let ngMenuFor = $state<string | null>(null);

  function doNg(targetType: NgTargetType, pattern: string) {
    addNgRule({
      targetType,
      matchMode: targetType === 'video_title' ? 'partial' : 'exact',
      pattern,
      scopeRanking: true,
      scopeSearch: false,
      scopeComment: false,
      enabled: true,
    });
    ngMenuFor = null;
  }

  $effect(() => {
    if (ngMenuFor != null) {
      const localMenuFor = ngMenuFor;
      setTimeout(() => {
        if (ngMenuFor === localMenuFor) {
          const handler = (e: MouseEvent) => {
            if ((e.target as HTMLElement).closest('.ng-btn') != null) return;
            if ((e.target as HTMLElement).closest('.ng-menu') != null) return;
            ngMenuFor = null;
            document.removeEventListener('click', handler);
          };
          document.addEventListener('click', handler, { once: true });
        }
      }, 0);
    }
  });
</script>

<section>
  <h2>ランキング</h2>
  <p class="muted">ジャンル別のニコニコ動画ランキングを表示。</p>

  <div class="controls">
    <div class="genre-chips">
      {#each GENRES as g (g.value)}
        <button
          class="chip"
          class:active={genre === g.value}
          onclick={() => onChangeGenre(g.value)}
        >
          {g.label}
        </button>
      {/each}
    </div>

    <div class="row">
      <div class="term-tabs">
        {#each TERMS as t (t.value)}
          <button class="tab" class:active={term === t.value} onclick={() => onChangeTerm(t.value)}>
            {t.label}
          </button>
        {/each}
      </div>
    </div>
  </div>

  {#if error}
    <div class="error" role="alert">エラー: {error}</div>
  {/if}

  {#if displayed.length > 0}
    <div class="meta">
      <span>{label}</span>
      <span class="dot">&middot;</span>
      <span>{displayed.length} 件</span>
      {#if blockedCount > 0}
        <span class="dot">&middot;</span>
        <span class="ng-note">NG: {blockedCount} 件除外中</span>
      {/if}
      {#if fetchedAt}
        <span class="dot">&middot;</span>
        <span class="muted">{formatDate(fetchedAt)} 取得</span>
      {/if}
    </div>

    <ol class="ranking">
      {#each displayed as item, i (item.id)}
        <li class="rank-item">
          <span class="rank-num">{i + 1 + (page - 1) * 100}</span>

          {#if item.thumbnail?.url ?? item.thumbnail?.listingUrl ?? item.thumbnail?.middleUrl}
            <a href="/video/{item.id}?from=ranking">
              <img
                class="thumb"
                src={item.thumbnail?.url ??
                  item.thumbnail?.listingUrl ??
                  item.thumbnail?.middleUrl ??
                  ''}
                alt=""
                loading="lazy"
                decoding="async"
              />
            </a>
          {:else}
            <div class="thumb placeholder"></div>
          {/if}

          <div class="info">
            <div class="title">
              <a href="/video/{item.id}?from=ranking">{item.title}</a>
              <a
                class="external"
                href={videoUrl(item.id)}
                target="_blank"
                rel="noreferrer noopener"
                title="ニコニコで開く">&nearr;</a
              >
            </div>
            <div class="row-meta muted">
              <span>{item.id}</span>
              {#if item.duration != null}
                <span class="dot">&middot;</span>
                <span>{formatDuration(item.duration)}</span>
              {/if}
              {#if item.registeredAt}
                <span class="dot">&middot;</span>
                <span>{formatDate(item.registeredAt!)}</span>
              {/if}
              {#if item.owner?.name}
                <span class="dot">&middot;</span>
                <span>{item.owner.name}</span>
              {/if}
            </div>
            <div class="row-meta">
              {#if item.count?.view != null}
                <span>再生 {formatNumber(item.count.view)}</span>
              {/if}
              {#if item.count?.comment != null}
                <span class="dot">&middot;</span>
                <span>コメ {formatNumber(item.count.comment)}</span>
              {/if}
              {#if item.count?.mylist != null}
                <span class="dot">&middot;</span>
                <span>マイリスト {formatNumber(item.count.mylist)}</span>
              {/if}
              {#if item.count?.like != null}
                <span class="dot">&middot;</span>
                <span>いいね {formatNumber(item.count.like)}</span>
              {/if}
            </div>
          </div>

          <div class="actions">
            <button
              type="button"
              class="ng-btn"
              onclick={() => (ngMenuFor = ngMenuFor === item.id ? null : item.id)}
              aria-label="NG に追加"
              title="NG に追加">&#x1F6AB;</button
            >
            {#if ngMenuFor === item.id}
              <div class="ng-menu" role="menu">
                <button
                  type="button"
                  class="ng-menu-item"
                  onclick={() => doNg('video_id', item.id)}
                >
                  この動画を NG
                </button>
                <button
                  type="button"
                  class="ng-menu-item"
                  onclick={() => doNg('video_title', item.title)}
                >
                  このタイトルで NG（部分一致）
                </button>
                {#if item.owner?.id}
                  <button
                    type="button"
                    class="ng-menu-item"
                    onclick={() =>
                      doNg('uploader', `${item.owner!.ownerType ?? 'user'}/${item.owner!.id}`)}
                  >
                    この投稿者 ({item.owner!.name ?? item.owner!.id}) を NG
                  </button>
                {/if}
              </div>
            {/if}
            <button
              type="button"
              class="dl-btn"
              disabled={dlPending.has(item.id)}
              onclick={() => onDownload(item.id)}
              aria-label="DL"
              title="ライブラリにダウンロード"
              >{dlPending.has(item.id) ? '\u23F3' : '\u2B07'}</button
            >
          </div>
        </li>
      {/each}
    </ol>

    <div class="pagination">
      {#if page > 1}
        <button type="button" onclick={() => goPage(page - 1)}>&larr; 前へ</button>
      {/if}
      <span class="muted">ページ {page}</span>
      {#if hasNext}
        <button type="button" onclick={() => goPage(page + 1)}>次へ &rarr;</button>
      {/if}
    </div>
  {:else if !pending}
    <p class="muted">結果なし</p>
  {/if}

  {#if pending && displayed.length === 0}
    <p class="muted">読み込み中…</p>
  {/if}
</section>

<style>
  h2 {
    margin-top: 0;
  }
  .muted {
    color: var(--theme-text-muted);
  }
  .controls {
    margin-bottom: 16px;
  }
  .genre-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 10px;
  }
  .chip {
    padding: 4px 10px;
    border: 1px solid var(--theme-border-strong);
    border-radius: 999px;
    background: var(--theme-surface-2);
    color: var(--theme-text-soft);
    font-size: 13px;
    cursor: pointer;
  }
  .chip:hover {
    background: var(--theme-border-strong);
    border-color: var(--theme-border-focus);
  }
  .chip.active {
    background: var(--theme-accent);
    color: white;
    border-color: var(--theme-accent);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: center;
  }
  .term-tabs {
    display: flex;
    gap: 4px;
  }
  .tab {
    padding: 5px 12px;
    border: 1px solid var(--theme-border-strong);
    border-radius: 6px;
    background: var(--theme-surface-2);
    color: var(--theme-text-soft);
    font-size: 13px;
    cursor: pointer;
  }
  .tab:hover {
    background: var(--theme-border-strong);
  }
  .tab.active {
    background: var(--theme-accent);
    color: white;
    border-color: var(--theme-accent);
  }
  .error {
    background: var(--theme-danger-bg);
    border: 1px solid var(--theme-danger-border);
    color: var(--theme-danger-text);
    padding: 10px 12px;
    border-radius: 6px;
    margin-bottom: 12px;
    font-size: 13px;
    white-space: pre-wrap;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    margin: 12px 0;
    font-size: 13px;
  }
  .dot {
    color: var(--theme-text-faint);
  }
  .ng-note {
    background: var(--theme-danger-bg-2);
    color: var(--theme-danger-text);
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 11px;
  }
  .ranking {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    counter-reset: none;
  }
  .rank-item {
    position: relative;
    display: grid;
    grid-template-columns: 36px 160px 1fr;
    gap: 8px 10px;
    padding: 8px;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 8px;
    align-items: start;
  }
  .rank-num {
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 16px;
    font-weight: 700;
    color: var(--theme-text-muted);
    min-height: 90px;
  }
  .rank-item:nth-child(1) .rank-num {
    color: #ffd700;
  }
  .rank-item:nth-child(2) .rank-num {
    color: #c0c0c0;
  }
  .rank-item:nth-child(3) .rank-num {
    color: #cd7f32;
  }
  .thumb {
    width: 160px;
    height: 90px;
    object-fit: cover;
    background: var(--theme-bg);
    border-radius: 4px;
  }
  .thumb.placeholder {
    border: 1px dashed var(--theme-border-strong);
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .title {
    padding-right: 68px;
  }
  .title a {
    color: var(--theme-text);
    text-decoration: none;
    font-weight: 600;
  }
  .title a:hover {
    text-decoration: underline;
  }
  .title .external {
    color: var(--theme-accent-soft);
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
  .actions {
    position: absolute;
    top: 6px;
    right: 6px;
    display: flex;
    gap: 6px;
    align-items: flex-start;
  }
  .dl-btn {
    background: rgba(26, 58, 38, 0.85);
    color: var(--theme-success-text);
    border: 1px solid var(--theme-success-border);
    border-radius: 6px;
    padding: 0 8px;
    font-size: 14px;
    line-height: 22px;
    cursor: pointer;
  }
  .dl-btn:hover:not(:disabled) {
    background: var(--theme-success-border);
    color: var(--theme-surface-2);
  }
  .dl-btn:disabled {
    opacity: 0.6;
    cursor: wait;
  }
  .ng-btn {
    background: none;
    color: var(--theme-text-muted);
    border: none;
    padding: 0;
    font-size: 14px;
    line-height: 22px;
    cursor: pointer;
    width: 28px;
    text-align: center;
  }
  .ng-btn:hover {
    color: var(--theme-danger-text);
  }
  .ng-menu {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    z-index: 20;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border-strong);
    border-radius: 8px;
    padding: 4px;
    min-width: 200px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
  }
  .ng-menu-item {
    display: block;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    color: var(--theme-text);
    font-size: 12px;
    padding: 6px 8px;
    border-radius: 4px;
    cursor: pointer;
  }
  .ng-menu-item:hover {
    background: var(--theme-border-strong);
  }
  .pagination {
    display: flex;
    gap: 12px;
    align-items: center;
    margin-top: 16px;
  }
  .pagination button {
    background: var(--theme-accent);
    color: white;
    border: none;
    border-radius: 6px;
    padding: 6px 14px;
    font-size: 13px;
    cursor: pointer;
  }
  .pagination button:hover {
    background: var(--theme-accent-hover);
  }
</style>
