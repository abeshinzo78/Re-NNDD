<script lang="ts">
  import { onMount } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import { invoke } from '@tauri-apps/api/core';
  import { extractAndParse, GENRE_KEY_BY_NAME, type RankingItem } from '@kongyo2/nicoran-api';
  import { formatNumber, formatDate, formatDuration, videoUrl } from '$lib/format';
  import { quickDownload } from '$lib/quickDownload';
  import {
    addNgRule,
    isRankingItemBlocked,
    listNgRules,
    subscribeNgRules,
    type NgRule,
    type NgTagKind,
    type NgTargetType,
    type RankingTagInfo,
  } from '$lib/stores/ngRules';
  import { fetchTagsBulk, getCachedTags } from '$lib/rankingTags';
  import RankingNgPanel from '$lib/RankingNgPanel.svelte';
  import VideoActionMenu from '$lib/VideoActionMenu.svelte';

  type GenreName = keyof typeof GENRE_KEY_BY_NAME;
  type Term = 'hour' | '24h' | 'week' | 'month' | 'total';
  type ShortSortKey =
    | 'hot'
    | 'viewCount'
    | 'registeredAt'
    | 'commentCount'
    | 'mylistCount'
    | 'likeCount';

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

  // ショートランキング (nvapi /v2/search/video) で指定できるソートキー。
  // hot は内部的に sortOrder=none、その他は desc 固定 (バックエンド側で付与)。
  const SHORT_SORTS: { value: ShortSortKey; label: string }[] = [
    { value: 'hot', label: '人気' },
    { value: 'viewCount', label: '再生数' },
    { value: 'registeredAt', label: '新着' },
    { value: 'commentCount', label: 'コメ数' },
    { value: 'mylistCount', label: 'マイリスト数' },
    { value: 'likeCount', label: 'いいね数' },
  ];

  // nvapi /v2/search/video?selectContentType=short が `genres` で受け付ける
  // 値。公式ランキングの一部ジャンル (vocaloid 等) は受け付けないので、
  // ショートモード時にだけ無効化する。`'all'` は API に渡さない (=全ジャンル)。
  // バックエンドの `SHORT_GENRES` (commands.rs) と同期させること。
  const SHORT_GENRE_MAP: Partial<Record<GenreName, string>> = {
    all: '', // 空 = ジャンル指定無し
    anime: 'anime',
    game: 'game',
    music: 'music_sound',
    entertainment: 'entertainment',
    dance: 'dance',
    commentary: 'commentary_lecture',
    cooking: 'cooking',
    nature: 'nature',
    vehicle: 'vehicle',
    radio: 'radio',
    sports: 'sports',
    animal: 'animal',
    other: 'other',
  };

  let genre = $state<GenreName>('all');
  let term = $state<Term>('24h');
  let page = $state(1);
  let rankShort = $state(false);
  let shortSort = $state<ShortSortKey>('hot');

  let pending = $state(false);
  let error = $state<string | null>(null);
  let items = $state<RankingItem[]>([]);
  let hasNext = $state(false);
  let label = $state('');
  let fetchedAt = $state<string | null>(null);

  let ngRules = $state<NgRule[]>([]);
  let ngUnsub: (() => void) | null = null;
  let showNgPanel = $state(false);
  let rankingRuleCount = $state(0);

  // 動画 ID -> タグ配列。未取得は undefined。
  const tagMap: SvelteMap<string, RankingTagInfo[] | undefined> = new SvelteMap();
  let tagFetching = $state(false);
  let tagFetchDone = $state(0);
  let tagFetchTotal = $state(0);
  let tagAbort: AbortController | null = null;

  // タグ系のランキング NG ルールが 1 件でもあればタグ取得が必要。
  let needsTags = $derived(
    ngRules.some((r) => r.enabled && r.scopeRanking && r.targetType === 'tag'),
  );

  let displayed = $derived(applyNgFilter(items, ngRules, tagMap));
  let blockedCount = $derived(items.length - displayed.length);

  function applyNgFilter(
    rankingItems: RankingItem[],
    rules: NgRule[],
    tags: SvelteMap<string, RankingTagInfo[] | undefined>,
  ): RankingItem[] {
    return rankingItems.filter((item) => {
      const r = isRankingItemBlocked(
        rules,
        {
          id: item.id,
          title: item.title,
          owner: item.owner ?? null,
        },
        tags.get(item.id),
      );
      return !r.blocked;
    });
  }

  onMount(() => {
    ngRules = listNgRules();
    ngUnsub = subscribeNgRules(() => (ngRules = listNgRules()));
    void runFetch();

    return () => {
      ngUnsub?.();
      tagAbort?.abort();
    };
  });

  // ルール変更 or items 変更時にタグを取り直す (必要な場合)
  $effect(() => {
    // 依存を明示
    const _ = needsTags;
    const ids = items.map((i) => i.id);
    if (_ && ids.length > 0) {
      void loadTagsFor(ids);
    } else if (!_ && tagFetching) {
      // タグ系ルールが消えたら進行中の取得を中断
      tagAbort?.abort();
      tagAbort = null;
      tagFetching = false;
    }
  });

  async function loadTagsFor(ids: string[]) {
    // すでにキャッシュ済みのものを Map に流し込む。
    // `tagMap` に id が存在しても値が undefined のもの (= 前回の placeholder
    // か取得失敗) はキャッシュ済みとは見なさず、再取得対象に含める。
    let needsFetch = false;
    for (const id of ids) {
      const cached = getCachedTags(id);
      if (cached) {
        tagMap.set(id, cached);
      } else {
        if (!tagMap.has(id)) tagMap.set(id, undefined);
        needsFetch = true;
      }
    }
    if (!needsFetch) return;

    tagAbort?.abort();
    const ctrl = new AbortController();
    tagAbort = ctrl;
    tagFetching = true;
    tagFetchDone = 0;
    tagFetchTotal = ids.length;
    try {
      const result = await fetchTagsBulk(ids, {
        concurrency: 8,
        signal: ctrl.signal,
        onProgress: (done, total) => {
          if (tagAbort !== ctrl) return;
          tagFetchDone = done;
          tagFetchTotal = total;
        },
      });
      if (ctrl.signal.aborted) return;
      for (const [id, tags] of result) tagMap.set(id, tags);
    } finally {
      // 自分が現役のコントローラのときだけ in-progress フラグを下ろす。
      // 並走中に新しい loadTagsFor が走った場合、古い finally で flag を
      // 落とすと新しい fetch の進行表示が消えてしまうため。
      if (tagAbort === ctrl) {
        tagAbort = null;
        tagFetching = false;
      }
    }
  }

  // ショートランキングは公式ランキング HTML (ジャンル+期間で出し分け) と違い、
  // niconico の検索 API (`/v2/search/video?selectContentType=short`) を裏で
  // 叩く別建ての取得経路。期間タブは利かないので隠す。
  async function runFetch() {
    pending = true;
    error = null;
    try {
      if (rankShort) {
        const apiGenre = SHORT_GENRE_MAP[genre] ?? '';
        const resp = await invoke<{
          items: RankingItem[];
          totalCount: number;
          hasNext: boolean;
        }>('search_short_ranking', {
          sortKey: shortSort,
          genre: apiGenre === '' ? null : apiGenre,
          page,
          pageSize: 50,
        });
        items = resp.items;
        hasNext = resp.hasNext;
        const sortLabel = SHORT_SORTS.find((s) => s.value === shortSort)?.label ?? shortSort;
        const genreLabel = GENRES.find((g) => g.value === genre)?.label ?? '';
        label = `ショート - ${genreLabel} - ${sortLabel}`;
      } else {
        const genreKey = GENRE_KEY_BY_NAME[genre];
        const params = new URLSearchParams({ term, page: String(page) });
        const url = `https://www.nicovideo.jp/ranking/genre/${genreKey}?${params}`;
        const html = await invoke<string>('fetch_ranking_html', { url });
        const { parsed } = extractAndParse(html);
        const ranking = parsed.data.response.$getTeibanRanking.data;
        items = ranking.items;
        hasNext = ranking.hasNext;
        label = ranking.label;
      }
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
    // ショートモードで未対応ジャンルが選ばれた場合、API が 400 を返す前に
    // ガード (UI 側で既に disabled だが、保険として)。
    if (rankShort && !(g in SHORT_GENRE_MAP)) return;
    genre = g;
    page = 1;
    void runFetch();
  }

  function onChangeTerm(t: Term) {
    term = t;
    page = 1;
    void runFetch();
  }

  function onChangeShortSort(s: ShortSortKey) {
    shortSort = s;
    page = 1;
    void runFetch();
  }

  function onToggleShort() {
    rankShort = !rankShort;
    page = 1;
    // ショート未対応ジャンルが選ばれていたら「総合」に戻す。
    if (rankShort && !(genre in SHORT_GENRE_MAP)) genre = 'all';
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

  function doNg(targetType: NgTargetType, pattern: string, opts: { tagKind?: NgTagKind } = {}) {
    addNgRule({
      targetType,
      matchMode: targetType === 'video_title' || targetType === 'tag' ? 'partial' : 'exact',
      pattern,
      scopeRanking: true,
      scopeSearch: false,
      scopeComment: false,
      enabled: true,
      tagKind: opts.tagKind,
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
          class:disabled={rankShort && !(g.value in SHORT_GENRE_MAP)}
          disabled={rankShort && !(g.value in SHORT_GENRE_MAP)}
          title={rankShort && !(g.value in SHORT_GENRE_MAP)
            ? 'このジャンルはショートで指定できません'
            : ''}
          onclick={() => onChangeGenre(g.value)}
        >
          {g.label}
        </button>
      {/each}
    </div>

    <div class="row">
      {#if rankShort}
        <div class="term-tabs">
          {#each SHORT_SORTS as s (s.value)}
            <button
              class="tab"
              class:active={shortSort === s.value}
              onclick={() => onChangeShortSort(s.value)}
            >
              {s.label}
            </button>
          {/each}
        </div>
      {:else}
        <div class="term-tabs">
          {#each TERMS as t (t.value)}
            <button
              class="tab"
              class:active={term === t.value}
              onclick={() => onChangeTerm(t.value)}
            >
              {t.label}
            </button>
          {/each}
        </div>
      {/if}
      <label class="short-rank-toggle">
        <input type="checkbox" checked={rankShort} onchange={onToggleShort} />
        <span>ショート</span>
      </label>
      <button
        type="button"
        class="ng-panel-toggle"
        class:active={showNgPanel}
        onclick={() => (showNgPanel = !showNgPanel)}
        aria-expanded={showNgPanel}
      >
        <span class="caret">{showNgPanel ? '▼' : '▶'}</span>
        ランキングNG設定 ({rankingRuleCount})
      </button>
    </div>
  </div>

  {#if showNgPanel}
    <RankingNgPanel onChange={(n) => (rankingRuleCount = n)} />
  {/if}

  {#if error}
    <div class="error" role="alert">エラー: {error}</div>
  {/if}

  {#if tagFetching}
    <div class="tag-progress">
      タグ取得中… {tagFetchDone} / {tagFetchTotal}
    </div>
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
          <span class="rank-num">{i + 1 + (page - 1) * (rankShort ? 50 : 100)}</span>

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
                {#if item.owner?.name}
                  <button
                    type="button"
                    class="ng-menu-item"
                    onclick={() => doNg('uploader_name', item.owner!.name!)}
                  >
                    投稿者名「{item.owner!.name}」を NG（完全一致）
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
            <!-- \u30D7\u30E9\u30B0\u30A4\u30F3\u304C\u3042\u308B\u5834\u5408\u3060\u3051 \u22EF \u30DC\u30BF\u30F3\u304C\u73FE\u308C\u308B (0 \u4EF6\u306A\u3089 DOM \u3054\u3068\u7121\u3057)\u3002 -->
            <VideoActionMenu
              video={{
                contentId: item.id,
                videoId: item.id,
                title: item.title,
                thumbnailUrl:
                  item.thumbnail?.url ??
                  item.thumbnail?.listingUrl ??
                  item.thumbnail?.middleUrl ??
                  null,
                lengthSeconds: item.duration ?? null,
                viewCounter: item.count?.view ?? null,
                source: 'ranking',
              }}
              compact={true}
            />
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
    color: var(--theme-accent-fg);
    border-color: var(--theme-accent);
  }
  .chip.disabled,
  .chip:disabled {
    opacity: 0.35;
    cursor: not-allowed;
    background: var(--theme-surface-2);
    color: var(--theme-text-muted);
  }
  .short-rank-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 13px;
    color: var(--theme-text-soft);
    cursor: pointer;
    user-select: none;
    padding: 4px 8px;
    border: 1px solid var(--theme-border-strong);
    border-radius: 6px;
    background: var(--theme-surface-2);
  }
  .short-rank-toggle:hover {
    background: var(--theme-border-strong);
  }
  .short-rank-toggle input {
    accent-color: var(--theme-accent);
    margin: 0;
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
    color: var(--theme-accent-fg);
    border-color: var(--theme-accent);
  }
  .ng-panel-toggle {
    margin-left: auto;
    padding: 5px 12px;
    border: 1px solid var(--theme-accent-soft, rgba(99, 102, 241, 0.4));
    border-radius: 6px;
    background: var(--theme-accent-bg, rgba(99, 102, 241, 0.18));
    color: var(--theme-accent-soft, #a5b4fc);
    font-size: 13px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .ng-panel-toggle:hover {
    background: var(--theme-accent-bg, rgba(99, 102, 241, 0.28));
  }
  .ng-panel-toggle.active {
    background: var(--theme-accent);
    border-color: var(--theme-accent);
    color: var(--theme-accent-fg);
  }
  .ng-panel-toggle .caret {
    font-size: 10px;
  }
  .tag-progress {
    background: var(--theme-accent-bg, rgba(99, 102, 241, 0.18));
    border: 1px solid var(--theme-accent-soft, rgba(99, 102, 241, 0.4));
    color: var(--theme-accent-soft, #a5b4fc);
    padding: 6px 10px;
    border-radius: 6px;
    margin: 8px 0;
    font-size: 12px;
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
    color: var(--theme-medal-gold);
  }
  .rank-item:nth-child(2) .rank-num {
    color: var(--theme-medal-silver);
  }
  .rank-item:nth-child(3) .rank-num {
    color: var(--theme-medal-bronze);
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
    background: var(--theme-success-bg);
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
    color: var(--theme-text);
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
    box-shadow: var(--theme-menu-shadow);
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
    color: var(--theme-accent-fg);
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
