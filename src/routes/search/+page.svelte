<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { page } from '$app/state';
  import { searchVideosOnline } from '$lib/api';
  import type { SearchField, SearchResponse, SearchTarget } from '$lib/api';
  import { formatNumber } from '$lib/format';
  import SearchHitCard from '$lib/SearchHitCard.svelte';
  import {
    loadSearchState,
    saveSearchState,
    sortByPopularity,
    type SortKey,
  } from '$lib/stores/searchState';
  import {
    filterSearchHits,
    listNgRules,
    subscribeNgRules,
    type NgRule,
  } from '$lib/stores/ngRules';

  let query = $state('');
  let targets = $state(new SvelteSet(['title']) as SvelteSet<SearchTarget>);
  let sortField = $state<SortKey>('popularity');
  let sortDir = $state<'desc' | 'asc'>('desc');
  let limit = $state(20);

  let pending = $state(false);
  let error = $state<string | null>(null);
  let response = $state<SearchResponse | null>(null);
  let lastQuery = $state<string | null>(null);

  let ngRules = $state<NgRule[]>([]);
  let ngUnsub: (() => void) | null = null;

  let displayed = $derived(
    response ? { ...response, data: filterSearchHits(ngRules, response.data) } : null,
  );
  let blockedCount = $derived(
    response && displayed ? response.data.length - displayed.data.length : 0,
  );

  // Restore prior search if returning from a video detail page.
  // URL params (?q=&targets=&autorun=1) take precedence — that's how tag
  // clicks navigate here.
  onMount(async () => {
    ngRules = listNgRules();
    ngUnsub = subscribeNgRules(() => (ngRules = listNgRules()));

    const params = page.url.searchParams;
    const urlQ = params.get('q');
    if (urlQ) {
      query = urlQ;
      const t = params.get('targets');
      if (t) {
        const parts = t.split(',').filter(Boolean) as SearchTarget[];
        if (parts.length) targets = new SvelteSet(parts);
      }
      const sort = params.get('sort') as SortKey | null;
      if (sort) sortField = sort;
      void runSearch();
      return;
    }

    const prev = loadSearchState();
    if (!prev) return;
    query = prev.query;
    targets = new SvelteSet(prev.targets);
    sortField = prev.sortField;
    sortDir = prev.sortDir;
    limit = prev.limit;
    response = prev.response;
    lastQuery = prev.lastQuery;
    await tick();
    if (prev.scrollY) {
      // The content scroller is the <main> element in the layout.
      const main = document.querySelector('main.content') as HTMLElement | null;
      if (main) main.scrollTop = prev.scrollY;
      else window.scrollTo(0, prev.scrollY);
    }
  });

  onDestroy(() => ngUnsub?.());

  function persist() {
    const main = document.querySelector('main.content') as HTMLElement | null;
    saveSearchState({
      query,
      targets: Array.from(targets),
      sortField,
      sortDir,
      limit,
      response,
      lastQuery,
      scrollY: main?.scrollTop ?? window.scrollY,
    });
  }

  function rememberOnNavigate() {
    persist();
  }

  function toggleTarget(t: SearchTarget) {
    const next = new SvelteSet(targets);
    if (next.has(t)) next.delete(t);
    else next.add(t);
    if (next.size === 0) next.add(t); // keep at least one
    targets = next;
  }

  /** Snapshot Search has no native popularity field, so popularity is
   * implemented as `viewCounter desc` server-side then re-ranked
   * client-side by a weighted score (see {@link sortByPopularity}). */
  function toApiSort(): { field: SearchField; direction: 'asc' | 'desc' } {
    if (sortField === 'popularity') {
      return { field: 'viewCounter', direction: 'desc' };
    }
    return { field: sortField, direction: sortDir };
  }

  async function runSearch(event?: Event) {
    event?.preventDefault();
    if (!query.trim()) return;
    pending = true;
    error = null;
    try {
      const apiResp = await searchVideosOnline({
        q: query,
        targets: Array.from(targets),
        fields: [
          'contentId',
          'title',
          'viewCounter',
          'commentCounter',
          'mylistCounter',
          'lengthSeconds',
          'thumbnailUrl',
          'startTime',
          'tags',
          'userId',
          'channelId',
        ],
        sort: toApiSort(),
        limit,
        offset: 0,
      });
      if (sortField === 'popularity') {
        apiResp.data = sortByPopularity(apiResp.data);
      }
      response = apiResp;
      lastQuery = query;
      persist();
    } catch (e) {
      error = String(e);
      response = null;
    } finally {
      pending = false;
    }
  }
</script>

<section>
  <h2>オンライン検索</h2>
  <p class="muted">
    niconico スナップショット検索 API v2 を直接叩く。データは 5:00 JST に日次更新。
  </p>

  <form class="search-form" onsubmit={runSearch}>
    <input
      class="q"
      type="search"
      placeholder="検索キーワード"
      bind:value={query}
      autocomplete="off"
      aria-label="検索キーワード"
    />
    <div class="targets" role="group" aria-label="検索対象">
      {#each [['title', 'タイトル'], ['description', '説明文'], ['tags', 'タグ'], ['tagsExact', 'タグ完全一致']] as [t, label] (t)}
        <label class="chip" class:active={targets.has(t as SearchTarget)}>
          <input
            type="checkbox"
            checked={targets.has(t as SearchTarget)}
            onchange={() => toggleTarget(t as SearchTarget)}
          />
          {label}
        </label>
      {/each}
    </div>
    <div class="row">
      <label>
        並び替え
        <select bind:value={sortField}>
          <option value="popularity">人気が高い順</option>
          <option value="viewCounter">再生数</option>
          <option value="commentCounter">コメ数</option>
          <option value="mylistCounter">マイリスト数</option>
          <option value="startTime">投稿日時</option>
          <option value="lengthSeconds">再生時間</option>
        </select>
      </label>
      <label class:disabled={sortField === 'popularity'}>
        順序
        <select bind:value={sortDir} disabled={sortField === 'popularity'}>
          <option value="desc">降順</option>
          <option value="asc">昇順</option>
        </select>
      </label>
      <label>
        件数
        <select bind:value={limit}>
          <option value={10}>10</option>
          <option value={20}>20</option>
          <option value={50}>50</option>
          <option value={100}>100</option>
        </select>
      </label>
      <button type="submit" disabled={pending || !query.trim()}>
        {pending ? '検索中…' : '検索'}
      </button>
    </div>
  </form>

  {#if error}
    <div class="error" role="alert">エラー: {error}</div>
  {/if}

  {#if displayed}
    <div class="meta">
      <span>「{lastQuery}」</span>
      <span class="dot">·</span>
      <span>合計 {formatNumber(displayed.meta.totalCount)} 件</span>
      <span class="dot">·</span>
      <span class="muted">表示: {displayed.data.length}</span>
      {#if blockedCount > 0}
        <span class="dot">·</span>
        <span class="ng-note">NG: {blockedCount} 件除外中</span>
      {/if}
    </div>

    {#if displayed.data.length === 0}
      <p class="muted">該当なし</p>
    {:else}
      <ul class="results">
        {#each displayed.data as hit (hit.contentId ?? Math.random())}
          <SearchHitCard {hit} onClick={rememberOnNavigate} />
        {/each}
      </ul>
    {/if}
  {:else if !pending}
    <p class="muted">キーワードを入力して検索してください。</p>
  {/if}
</section>

<style>
  h2 {
    margin-top: 0;
  }
  .muted {
    color: #9a9a9a;
  }
  .search-form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-bottom: 16px;
  }
  .q {
    background: #0f0f0f;
    border: 1px solid #2f2f2f;
    color: #f5f5f5;
    border-radius: 6px;
    padding: 10px 12px;
    font-size: 15px;
  }
  .q:focus {
    outline: none;
    border-color: #5a5a5a;
  }
  .targets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border: 1px solid #2f2f2f;
    border-radius: 999px;
    background: #161616;
    font-size: 13px;
    cursor: pointer;
    user-select: none;
  }
  .chip.active {
    background: #2a2a2a;
    border-color: #4a4a4a;
    color: #fff;
  }
  .chip input {
    display: none;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    align-items: end;
  }
  .row label {
    display: flex;
    flex-direction: column;
    font-size: 12px;
    color: #b0b0b0;
    gap: 4px;
  }
  .row label.disabled {
    opacity: 0.5;
  }
  select {
    background: #eaeaea;
    border: 1px solid #2f2f2f;
    color: #111;
    border-radius: 6px;
    padding: 6px 8px;
    font-size: 13px;
  }
  select:disabled {
    background: #2a2a2a;
    color: #777;
    cursor: not-allowed;
  }
  select option {
    background: #eaeaea;
    color: #111;
  }
  button[type='submit'] {
    background: #2563eb;
    color: white;
    border: none;
    border-radius: 6px;
    padding: 8px 18px;
    font-size: 14px;
    cursor: pointer;
  }
  button[type='submit']:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .error {
    background: #2a1212;
    border: 1px solid #5a2222;
    color: #f5b3b3;
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
    color: #555;
  }
  .ng-note {
    background: #2a1f1a;
    color: #f5b3b3;
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 11px;
  }
  .results {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
</style>
