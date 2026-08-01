<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import {
    cleanupStorage,
    deleteLibraryVideo,
    getLibraryStats,
    listLibraryResolutions,
    listLibraryTags,
    listLibraryUploaders,
    queryLibraryVideos,
    searchLibraryComments,
    type CommentSearchHit,
    type LibraryStats,
    type LibraryVideoRow,
    type UploaderInfo,
  } from '$lib/api';
  import { formatDuration, formatNumber } from '$lib/format';
  import { setQueue, itemHref, type PlaybackQueueItem } from '$lib/stores/playbackQueue';
  import { createSmartPlaylist } from '$lib/stores/smartPlaylists';
  import {
    activeFilterCount,
    addTagFilter,
    buildLibraryQuery,
    canSearchComments,
    clampPage,
    checkSmartPlaylistSave,
    clearLibraryFilters,
    commentHitHref,
    COMMENT_SEARCH_MIN_CHARS,
    LIBRARY_PAGE_SIZES,
    LIBRARY_SORT_OPTIONS,
    loadLibraryFilter,
    newRandomSeed,
    patchLibraryFilter,
    pageCount,
    removeTagFilter,
    resultRangeLabel,
    saveLibraryFilter,
    summarizeLibraryFilter,
    toSmartPlaylistFilter,
    type LibraryFilterState,
    type LibrarySortKey,
  } from '$lib/stores/libraryQuery';
  import { thumbFallback } from '$lib/thumbnail';
  import VideoActionMenu from '$lib/VideoActionMenu.svelte';

  let filter = $state<LibraryFilterState>(loadLibraryFilter());

  let items = $state<LibraryVideoRow[]>([]);
  let totalCount = $state(0);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let deleting = $state<string | null>(null);

  // コメント横断検索
  let commentHits = $state<CommentSearchHit[]>([]);
  let commentTotal = $state(0);

  // 絞り込み候補 (ライブラリの実データから引く)
  let allTags = $state<string[]>([]);
  let resolutions = $state<string[]>([]);
  let uploaders = $state<UploaderInfo[]>([]);
  let stats = $state<LibraryStats | null>(null);
  let statsOpen = $state(false);
  let filtersOpen = $state(false);
  let tagInput = $state('');

  /** `list_library_uploaders` がまとめて返せる上限 (Rust 側で 200 に丸められる)。 */
  const UPLOADER_LIST_LIMIT = 200;

  let activeFilters = $derived(activeFilterCount(filter));
  /** 選択中の投稿者が候補一覧に居ない場合 (200 件の枠外 / カードから直接指定)
   *  でも選択が消えないよう、その ID を候補へ足す。 */
  let uploaderOptions = $derived(
    filter.uploaderId &&
      !uploaders.some(
        (u) =>
          u.uploaderId === filter.uploaderId &&
          (u.uploaderType ?? '') === (filter.uploaderType ?? ''),
      )
      ? [
          {
            uploaderId: filter.uploaderId,
            uploaderName: null,
            videoCount: 0,
            totalDurationSec: 0,
            uploaderType: filter.uploaderType || null,
          } satisfies UploaderInfo,
          ...uploaders,
        ]
      : uploaders,
  );
  let totalPages = $derived(
    pageCount(filter.mode === 'comments' ? commentTotal : totalCount, filter.pageSize),
  );
  let rangeLabel = $derived(
    filter.mode === 'comments'
      ? resultRangeLabel(commentTotal, filter.page * filter.pageSize, commentHits.length)
      : resultRangeLabel(totalCount, filter.page * filter.pageSize, items.length),
  );
  let commentQueryTooShort = $derived(filter.mode === 'comments' && !canSearchComments(filter.q));
  // コメント検索はタグ/解像度の絞り込みを受け付けない (バックエンドが本文
  // クエリしか取らない)。押しても効かないチップを押せる状態にしない。
  let facetsDisabled = $derived(filter.mode === 'comments');
  const facetDisabledHint = 'タグ・解像度の絞り込みは「動画」検索でのみ使えます。';

  /** 検索条件を更新して即座に再取得する。`page` 以外の変更はページを 0 に戻す。 */
  function update(patch: Partial<LibraryFilterState>) {
    filter = patchLibraryFilter(filter, patch);
    saveLibraryFilter(filter);
    void refresh();
  }

  // 入力のたびに走る検索なので、遅れて返ってきた古い応答で新しい結果を
  // 上書きしないよう世代番号で捨てる (連打・高速入力での取り違え防止)。
  let requestSeq = 0;

  async function refresh() {
    const seq = ++requestSeq;
    loading = true;
    try {
      if (filter.mode === 'comments') {
        await refreshComments(seq);
      } else {
        await refreshVideos(seq);
      }
      if (seq !== requestSeq) return;
      error = null;
    } catch (e) {
      if (seq !== requestSeq) return;
      error = String(e);
    } finally {
      if (seq === requestSeq) loading = false;
    }
  }

  async function refreshVideos(seq: number) {
    const result = await queryLibraryVideos(buildLibraryQuery(filter));
    if (seq !== requestSeq) return;
    items = result.items;
    totalCount = result.totalCount;
    // 削除などで総件数が減り、現在ページが範囲外になったら末尾へ寄せて引き直す。
    const clamped = clampPage(filter, result.totalCount);
    if (clamped !== filter) {
      filter = clamped;
      saveLibraryFilter(filter);
      const retry = await queryLibraryVideos(buildLibraryQuery(filter));
      if (seq !== requestSeq) return;
      items = retry.items;
      totalCount = retry.totalCount;
    }
  }

  async function refreshComments(seq: number) {
    if (!canSearchComments(filter.q)) {
      commentHits = [];
      commentTotal = 0;
      return;
    }
    const result = await searchLibraryComments(
      filter.q.trim(),
      filter.page * filter.pageSize,
      filter.pageSize,
    );
    if (seq !== requestSeq) return;
    commentHits = result.items;
    commentTotal = result.totalCount;
    // 動画側と同じく、スナップショット削除などでヒット数が減って現在ページが
    // 範囲外になったら末尾へ寄せて引き直す (空ページ + 「3 / 1」表示の防止)。
    const clamped = clampPage(filter, result.totalCount);
    if (clamped !== filter) {
      filter = clamped;
      saveLibraryFilter(filter);
      const retry = await searchLibraryComments(
        filter.q.trim(),
        filter.page * filter.pageSize,
        filter.pageSize,
      );
      if (seq !== requestSeq) return;
      commentHits = retry.items;
      commentTotal = retry.totalCount;
    }
  }

  /** タグ / 解像度 / 投稿者の候補と集計。ライブラリ更新のたびに引き直す。
   *  投稿者はバックエンドが返せる上限 (200) までまとめて取る。 */
  async function refreshFacets() {
    try {
      const [tags, res, ups, st] = await Promise.all([
        listLibraryTags(),
        listLibraryResolutions(),
        listLibraryUploaders(UPLOADER_LIST_LIMIT),
        getLibraryStats(),
      ]);
      allTags = tags;
      resolutions = res;
      uploaders = ups;
      stats = st;
    } catch (e) {
      // 候補が引けなくても検索自体は使えるので、致命扱いにはしない。
      console.error('[library] facets の取得に失敗:', e);
    }
  }

  let searchTimeout: ReturnType<typeof setTimeout> | null = null;
  function onSearchInput() {
    if (searchTimeout) clearTimeout(searchTimeout);
    searchTimeout = setTimeout(() => update({ q: filter.q }), 300);
  }

  function setMode(mode: LibraryFilterState['mode']) {
    if (filter.mode === mode) return;
    update({ mode });
  }

  function onAddTag() {
    const next = addTagFilter(filter, tagInput);
    tagInput = '';
    if (next === filter) return;
    filter = next;
    saveLibraryFilter(filter);
    void refresh();
  }

  function onTagKeydown(e: KeyboardEvent) {
    if (e.key !== 'Enter') return;
    e.preventDefault();
    onAddTag();
  }

  function onRemoveTag(tag: string) {
    const next = removeTagFilter(filter, tag);
    if (next === filter) return;
    filter = next;
    saveLibraryFilter(filter);
    void refresh();
  }

  /** 集計パネルのタグチップ: 付いていれば外す、無ければ AND 条件に足す。 */
  function toggleTagFilter(tag: string) {
    filtersOpen = true;
    const next = filter.tags.includes(tag)
      ? removeTagFilter(filter, tag)
      : addTagFilter(filter, tag);
    if (next === filter) return;
    filter = next;
    saveLibraryFilter(filter);
    void refresh();
  }

  function onClearFilters() {
    filter = clearLibraryFilters(filter);
    saveLibraryFilter(filter);
    void refresh();
  }

  function toggleSortOrder() {
    update({ sortOrder: filter.sortOrder === 'desc' ? 'asc' : 'desc' });
  }

  /** カードから直接その投稿者で絞り込む。候補一覧 (上限 200 件) に載らない
   *  投稿者へも辿り着けるようにするための導線。もう一度押すと解除。 */
  function filterByUploader(item: LibraryVideoRow, e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (!item.uploaderId) return;
    filtersOpen = true;
    if (filter.uploaderId === item.uploaderId) {
      update({ uploaderId: '', uploaderType: '' });
      return;
    }
    update({ uploaderId: item.uploaderId, uploaderType: item.uploaderType ?? '' });
  }

  /** 投稿者の選択。オンライン検索へ持ち出す時に必要な種別も一緒に控える。 */
  function onUploaderChange(e: Event & { currentTarget: HTMLSelectElement }) {
    const key = e.currentTarget.value;
    if (!key) {
      update({ uploaderId: '', uploaderType: '' });
      return;
    }
    const sep = key.indexOf(':');
    update({ uploaderType: key.slice(0, sep), uploaderId: key.slice(sep + 1) });
  }

  /** 投稿者セレクトの値。ユーザ ID とチャンネル ID は別名前空間で数値が
   *  衝突しうるので、種別込みの複合キー (`user:123` / `:123`) で扱う。 */
  function uploaderKey(uploaderId: string, uploaderType: string | null | undefined): string {
    return uploaderId ? `${uploaderType ?? ''}:${uploaderId}` : '';
  }

  /** 投稿者の表示名。種別が分かるものはチャンネルだと分かるようにする。 */
  function uploaderLabel(u: UploaderInfo): string {
    const name = u.uploaderName ?? `ID:${u.uploaderId}`;
    const kind = u.uploaderType === 'channel' ? '[ch] ' : '';
    return u.videoCount > 0 ? `${kind}${name} (${u.videoCount})` : `${kind}${name}`;
  }

  /** 並び順の変更。ランダムを選び直したらシャッフルし直す。 */
  function onSortChange(sortBy: LibrarySortKey) {
    update(sortBy === 'random' ? { sortBy, randomSeed: newRandomSeed() } : { sortBy });
  }

  /** 更新ボタン。ランダム順のときは並び直したいので種も引き直す。 */
  function onManualRefresh() {
    if (filter.sortBy === 'random' && filter.mode === 'videos') {
      update({ randomSeed: newRandomSeed() });
    } else {
      void refresh();
    }
    void refreshFacets();
  }

  function goToPage(page: number) {
    const last = totalPages - 1;
    update({ page: Math.min(Math.max(0, page), Math.max(0, last)) });
  }

  let cleaning = $state(false);
  let cleanupMsg = $state<string | null>(null);
  async function onCleanup() {
    if (!confirm('既存 DL 物から不要なサイドカー(古い meta.json 等)を削除します。')) return;
    cleaning = true;
    cleanupMsg = null;
    try {
      const bytes = await cleanupStorage();
      const mb = (bytes / 1024 / 1024).toFixed(2);
      cleanupMsg = bytes > 0 ? `${mb} MB 削除しました` : '削除対象なし';
    } catch (e) {
      cleanupMsg = `失敗: ${e}`;
    } finally {
      cleaning = false;
    }
  }

  async function onDelete(item: LibraryVideoRow, e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (
      !confirm(
        `「${item.title}」(${item.id}) を完全に削除しますか？\nファイル + DB 両方削除されます。`,
      )
    )
      return;
    deleting = item.id;
    try {
      await deleteLibraryVideo(item.id);
      await refresh();
      await refreshFacets();
    } catch (err) {
      error = `削除失敗: ${err}`;
    } finally {
      deleting = null;
    }
  }

  /** タグ名で each するので重複は潰す。バックエンド側でも DISTINCT している
   *  が、tags の主キーが (video_id, name, source) である以上、別経路で重複が
   *  来ると each_key_duplicate でグリッドごと落ちるため UI 側でも防ぐ。 */
  function uniqueTags(tags: string[]): string[] {
    return Array.from(new Set(tags));
  }

  function thumbSrc(item: LibraryVideoRow): string | undefined {
    if (item.localThumbnailPath) return convertFileSrc(item.localThumbnailPath);
    return item.thumbnailUrl ?? undefined;
  }

  function relativeDate(unix: number | null): string {
    if (!unix) return '';
    const d = new Date(unix * 1000);
    return d.toLocaleDateString('ja-JP', { year: 'numeric', month: '2-digit', day: '2-digit' });
  }

  /** "1280x720" → "720p" などに整形。マッチしない時は元文字列をそのまま返す。 */
  function shortResolution(res: string | null): string | null {
    if (!res) return null;
    const m = /^\s*(\d+)x(\d+)\s*$/.exec(res);
    if (!m) return res;
    const h = Number(m[2]);
    return Number.isFinite(h) ? `${h}p` : res;
  }

  /** 合計再生時間 (秒) を「12 時間 34 分」形式に。 */
  function formatTotalDuration(sec: number): string {
    const h = Math.floor(sec / 3600);
    const m = Math.floor((sec % 3600) / 60);
    if (h > 0) return `${formatNumber(h)} 時間 ${m} 分`;
    return `${m} 分`;
  }

  function toQueueItems(): PlaybackQueueItem[] {
    return items.map((it) => ({
      videoId: it.id,
      title: it.title,
      thumbnailUrl: it.thumbnailUrl ?? undefined,
      lengthSeconds: it.durationSec,
      source: 'local',
    }));
  }

  function startPlayAll() {
    const queueItems = toQueueItems();
    if (queueItems.length === 0) return;
    const summary = summarizeLibraryFilter(filter);
    const label = summary === '条件なし' ? 'ライブラリ' : `ライブラリ (${summary})`;
    setQueue('library', 'library', label, queueItems, 0);
    void goto(itemHref(queueItems[0]));
  }

  /** 現在の検索条件をスマートプレイリスト (＝オンライン検索の保存) にする。
   *  オンライン検索へ持ち出せない条件は保存前に明示して確認する。 */
  function saveAsSmartPlaylist() {
    const { canSave, dropped, notes } = checkSmartPlaylistSave(filter);
    if (!canSave) {
      alert(
        'スマートプレイリストはオンライン検索の条件を保存する機能です。\n' +
          'キーワードかタグを指定してください (投稿者だけでは検索できません)。',
      );
      return;
    }
    const warnings = [
      ...(dropped.length > 0 ? [`保存されない条件: ${dropped.join('・')}`] : []),
      ...notes,
    ];
    if (warnings.length > 0 && !confirm(`${warnings.join('\n')}\n\nこの内容で保存しますか？`))
      return;

    const summary = summarizeLibraryFilter(filter);
    const name = window.prompt(
      'スマートプレイリスト名',
      summary === '条件なし' ? '保存検索' : `検索: ${summary}`,
    );
    if (!name) return;
    const p = createSmartPlaylist(name, toSmartPlaylistFilter(filter));
    void goto(`/playlists/smart/${p.id}`);
  }

  onMount(() => {
    void refresh();
    void refreshFacets();
  });
</script>

<section class="page">
  <header class="head">
    <h2>ライブラリ</h2>
    <div class="head-actions">
      <div class="mode-tabs" role="tablist" aria-label="検索対象">
        <button
          type="button"
          role="tab"
          aria-selected={filter.mode === 'videos'}
          class="mode-tab"
          class:active={filter.mode === 'videos'}
          onclick={() => setMode('videos')}>動画</button
        >
        <button
          type="button"
          role="tab"
          aria-selected={filter.mode === 'comments'}
          class="mode-tab"
          class:active={filter.mode === 'comments'}
          onclick={() => setMode('comments')}
          title="保存済みコメントスナップショットを横断検索">コメント</button
        >
      </div>
      <input
        type="search"
        class="search-box"
        placeholder={filter.mode === 'comments'
          ? 'コメント本文を検索…(3 文字以上)'
          : '動画名・タグ・コメントで検索…'}
        aria-label={filter.mode === 'comments' ? 'コメント検索' : '動画検索'}
        bind:value={filter.q}
        oninput={onSearchInput}
      />
      {#if filter.mode === 'videos'}
        <button
          type="button"
          class="ghost"
          class:on={filtersOpen}
          aria-expanded={filtersOpen}
          onclick={() => (filtersOpen = !filtersOpen)}
        >
          絞り込み{activeFilters > 0 ? ` (${activeFilters})` : ''}
        </button>
      {/if}
      <button
        type="button"
        class="ghost"
        class:on={statsOpen}
        aria-expanded={statsOpen}
        onclick={() => (statsOpen = !statsOpen)}>集計</button
      >
      <button type="button" class="ghost" onclick={onManualRefresh}>更新</button>
      {#if filter.mode === 'videos'}
        <button
          type="button"
          class="primary"
          disabled={items.length === 0}
          onclick={startPlayAll}
          title="現在表示中の動画を全て連続再生"
        >
          ▶ 連続再生
        </button>
        <button
          type="button"
          class="ghost"
          onclick={saveAsSmartPlaylist}
          title="現在の検索条件をスマートプレイリストとして保存"
        >
          スマートプレイリスト保存
        </button>
      {/if}
      <button type="button" class="ghost" disabled={cleaning} onclick={onCleanup}>
        {cleaning ? '掃除中…' : 'ストレージ掃除'}
      </button>
    </div>
  </header>

  {#if statsOpen}
    <div class="panel stats">
      {#if stats}
        <div class="stat-row">
          <div class="stat">
            <span class="k">動画</span><b>{formatNumber(stats.totalVideos)}</b>
          </div>
          <div class="stat">
            <span class="k">総再生時間</span><b>{formatTotalDuration(stats.totalDurationSec)}</b>
          </div>
          <div
            class="stat"
            title="ライブラリ内の動画に付いているコメント数の合計 (niconico 側の値)"
          >
            <span class="k">コメント総数</span><b>{formatNumber(stats.totalComments)}</b>
          </div>
          <div class="stat">
            <span class="k">投稿者</span><b>{formatNumber(stats.uniqueUploaders)}</b>
          </div>
          <div class="stat"><span class="k">タグ</span><b>{formatNumber(stats.uniqueTags)}</b></div>
        </div>
        {#if stats.topTags.length > 0}
          <div class="facet">
            <span class="facet-label">よく付くタグ</span>
            <div class="chips">
              {#each stats.topTags.slice(0, 20) as t (t.name)}
                <button
                  type="button"
                  class="chip"
                  class:on={filter.tags.includes(t.name)}
                  disabled={facetsDisabled}
                  title={facetsDisabled ? facetDisabledHint : `タグ「${t.name}」で絞り込む`}
                  onclick={() => toggleTagFilter(t.name)}
                >
                  {t.name}<span class="count">{t.count}</span>
                </button>
              {/each}
            </div>
          </div>
        {/if}
        {#if stats.resolutionDistribution.length > 0}
          <div class="facet">
            <span class="facet-label">解像度</span>
            <div class="chips">
              {#each stats.resolutionDistribution as r (r.resolution)}
                <button
                  type="button"
                  class="chip"
                  class:on={filter.resolution === r.resolution}
                  disabled={facetsDisabled}
                  title={facetsDisabled ? facetDisabledHint : `解像度 ${r.resolution} で絞り込む`}
                  onclick={() => {
                    filtersOpen = true;
                    update({
                      resolution: filter.resolution === r.resolution ? '' : r.resolution,
                    });
                  }}
                >
                  {r.resolution}<span class="count">{r.count}</span>
                </button>
              {/each}
            </div>
          </div>
        {/if}
        {#if facetsDisabled}
          <p class="muted small facet-note">{facetDisabledHint}</p>
        {/if}
      {:else}
        <div class="muted">集計を読み込み中…</div>
      {/if}
    </div>
  {/if}

  {#if filtersOpen && filter.mode === 'videos'}
    <div class="panel filters">
      <div class="filter-row">
        <div class="field">
          <label class="field-label" for="library-tag-input">タグ (AND)</label>
          <span class="tag-input-wrap">
            <input
              id="library-tag-input"
              class="text-input"
              list="library-tag-options"
              placeholder="タグ名を入力して Enter"
              bind:value={tagInput}
              onkeydown={onTagKeydown}
            />
            <button type="button" class="ghost small-btn" onclick={onAddTag}>追加</button>
          </span>
        </div>
        <datalist id="library-tag-options">
          {#each allTags as t (t)}
            <option value={t}></option>
          {/each}
        </datalist>

        <label class="field">
          <span class="field-label">投稿者</span>
          <select
            class="text-input"
            value={uploaderKey(filter.uploaderId, filter.uploaderType)}
            onchange={onUploaderChange}
          >
            <option value="">すべて</option>
            {#each uploaderOptions as u (uploaderKey(u.uploaderId, u.uploaderType))}
              <option value={uploaderKey(u.uploaderId, u.uploaderType)}>{uploaderLabel(u)}</option>
            {/each}
          </select>
        </label>

        <label class="field">
          <span class="field-label">解像度</span>
          <select
            class="text-input"
            value={filter.resolution}
            onchange={(e) => update({ resolution: e.currentTarget.value })}
          >
            <option value="">すべて</option>
            {#each resolutions as r (r)}
              <option value={r}>{r}</option>
            {/each}
          </select>
        </label>

        <div class="field">
          <label class="field-label" for="library-min-minutes">再生時間 (分)</label>
          <span class="range-wrap">
            <input
              id="library-min-minutes"
              class="text-input num"
              type="number"
              min="0"
              step="1"
              placeholder="最短"
              aria-label="最短の再生時間 (分)"
              value={filter.minMinutes}
              onchange={(e) => update({ minMinutes: e.currentTarget.value })}
            />
            <span class="dash">〜</span>
            <input
              class="text-input num"
              type="number"
              min="0"
              step="1"
              placeholder="最長"
              aria-label="最長の再生時間 (分)"
              value={filter.maxMinutes}
              onchange={(e) => update({ maxMinutes: e.currentTarget.value })}
            />
          </span>
        </div>

        <label class="short-toggle">
          <input
            type="checkbox"
            checked={filter.isShort}
            onchange={(e) => update({ isShort: e.currentTarget.checked })}
          />
          <span>ショートのみ</span>
        </label>

        <button
          type="button"
          class="ghost small-btn"
          disabled={activeFilters === 0}
          onclick={onClearFilters}>条件クリア</button
        >
      </div>

      {#if filter.tags.length > 0}
        <div class="chips selected-tags">
          {#each filter.tags as t (t)}
            <button
              type="button"
              class="chip on"
              title="この絞り込みを外す"
              onclick={() => onRemoveTag(t)}
            >
              {t}<span class="x">×</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  <div class="toolbar">
    <span class="muted count">{loading ? '読み込み中…' : rangeLabel}</span>

    {#if filter.mode === 'videos'}
      <label class="inline-field">
        <span class="muted small">並び順</span>
        <select
          class="text-input"
          value={filter.sortBy}
          onchange={(e) => onSortChange(e.currentTarget.value as LibrarySortKey)}
        >
          {#each LIBRARY_SORT_OPTIONS as opt (opt.value)}
            <option value={opt.value}>{opt.label}</option>
          {/each}
        </select>
      </label>
      <button
        type="button"
        class="ghost small-btn"
        disabled={filter.sortBy === 'random'}
        title={filter.sortBy === 'random' ? 'ランダム順では使いません' : '昇順 / 降順を切り替え'}
        onclick={toggleSortOrder}
      >
        {filter.sortOrder === 'desc' ? '↓ 降順' : '↑ 昇順'}
      </button>
    {/if}

    <label class="inline-field">
      <span class="muted small">表示件数</span>
      <select
        class="text-input"
        value={filter.pageSize}
        onchange={(e) => update({ pageSize: Number(e.currentTarget.value) })}
      >
        {#each LIBRARY_PAGE_SIZES as n (n)}
          <option value={n}>{n}</option>
        {/each}
      </select>
    </label>

    <span class="pager">
      <button
        type="button"
        class="ghost small-btn"
        disabled={filter.page === 0 || loading}
        onclick={() => goToPage(filter.page - 1)}>← 前</button
      >
      <span class="muted small page-label">{filter.page + 1} / {totalPages}</span>
      <button
        type="button"
        class="ghost small-btn"
        disabled={filter.page >= totalPages - 1 || loading}
        onclick={() => goToPage(filter.page + 1)}>次 →</button
      >
    </span>
  </div>

  {#if cleanupMsg}
    <div class="info">{cleanupMsg}</div>
  {/if}

  {#if error}
    <div class="error">エラー: {error}</div>
  {/if}

  {#if filter.mode === 'comments'}
    {#if commentQueryTooShort}
      <div class="empty">
        <p class="muted">
          保存済みコメントの横断検索です。{COMMENT_SEARCH_MIN_CHARS} 文字以上入力してください。
        </p>
        <p class="muted small">
          動画ページで取得したコメントスナップショットが検索対象になります。
        </p>
      </div>
    {:else if loading}
      <div class="muted">読み込み中…</div>
    {:else if commentHits.length === 0}
      <div class="empty"><p class="muted">一致するコメントはありません。</p></div>
    {:else}
      <ul class="comment-list">
        {#each commentHits as hit, i (`${hit.videoId}-${hit.commentNo}-${i}`)}
          <li class="comment-row">
            <a
              class="comment-link"
              href={commentHitHref(hit.videoId, hit.vposMs)}
              title="この位置から再生"
            >
              <span class="vpos">{formatDuration(Math.floor(hit.vposMs / 1000))}</span>
              <span class="comment-body">{hit.content}</span>
              <span class="comment-meta muted small">
                {hit.videoTitle}
                <span class="dot">·</span>
                #{hit.commentNo}
                {#if hit.postedAt}
                  <span class="dot">·</span>{relativeDate(hit.postedAt)}
                {/if}
              </span>
            </a>
          </li>
        {/each}
      </ul>
    {/if}
  {:else if loading}
    <div class="muted">読み込み中…</div>
  {:else if items.length === 0}
    <div class="empty">
      <p class="muted">
        {filter.q.trim() || activeFilters > 0
          ? '検索結果が見つかりません。'
          : 'ダウンロード済みの動画はまだありません。'}
      </p>
      {#if activeFilters > 0}
        <p><button type="button" class="ghost" onclick={onClearFilters}>絞り込みを解除</button></p>
      {:else if !filter.q.trim()}
        <p class="muted">
          <a href="/downloads">ダウンロード</a> ページで動画 ID を追加 → 「DL 開始」で取り込めます。
        </p>
      {/if}
    </div>
  {:else}
    <div class="grid">
      {#each items as item (item.id)}
        <div class="card-wrap">
          <a class="card" href={`/library/${item.id}?from=library`}>
            <div class="thumb-wrap">
              {#if thumbSrc(item)}
                <img
                  class="thumb"
                  src={thumbSrc(item)}
                  alt=""
                  loading="lazy"
                  use:thumbFallback={{ videoId: item.id }}
                />
              {:else}
                <div class="thumb-placeholder">?</div>
              {/if}
              {#if shortResolution(item.resolution)}
                <span class="resolution" title={item.resolution ?? ''}>
                  {shortResolution(item.resolution)}
                </span>
              {/if}
              <span class="duration">{formatDuration(item.durationSec)}</span>
            </div>
            <div class="meta">
              <h3 class="title" title={item.title}>{item.title}</h3>
              <div class="row muted">
                {#if item.uploaderName}<span class="uploader">{item.uploaderName}</span>{/if}
                {#if item.viewCount != null}
                  <span class="dot">·</span>
                  <span>{formatNumber(item.viewCount)} 再生</span>
                {/if}
              </div>
              <div class="row muted small">
                <span>DL {relativeDate(item.downloadedAt)}</span>
                {#if item.resolution}
                  <span class="dot">·</span>
                  <span>{item.resolution}</span>
                {/if}
                {#if item.playCount > 0}
                  <span class="dot">·</span>
                  <span title="ライブラリでの再生回数">▶ {formatNumber(item.playCount)}</span>
                {/if}
              </div>
              {#if item.tags.length > 0}
                {@const tags = uniqueTags(item.tags)}
                <div class="tags">
                  {#each tags.slice(0, 4) as tag (tag)}
                    <span class="tag">{tag}</span>
                  {/each}
                  {#if tags.length > 4}
                    <span class="tag muted">+{tags.length - 4}</span>
                  {/if}
                </div>
              {/if}
            </div>
          </a>
          {#if item.uploaderId}
            <button
              type="button"
              class="uploader-btn"
              class:on={filter.uploaderId === item.uploaderId}
              title={filter.uploaderId === item.uploaderId
                ? '投稿者の絞り込みを外す'
                : `「${item.uploaderName ?? item.uploaderId}」の動画だけ表示`}
              onclick={(e) => filterByUploader(item, e)}>👤</button
            >
          {/if}
          <button
            type="button"
            class="del-btn"
            disabled={deleting === item.id}
            title="ライブラリから完全削除"
            onclick={(e) => onDelete(item, e)}>{deleting === item.id ? '…' : '×'}</button
          >
          <div class="plugin-menu-wrap">
            <VideoActionMenu
              video={{
                contentId: item.id,
                videoId: item.id,
                title: item.title,
                thumbnailUrl: thumbSrc(item),
                lengthSeconds: item.durationSec ?? null,
                viewCounter: item.viewCount ?? null,
                source: 'library',
              }}
              compact={true}
            />
          </div>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .page {
    max-width: 1400px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
    flex-wrap: wrap;
    gap: 10px;
  }
  .head h2 {
    margin: 0;
  }
  .head-actions {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .mode-tabs {
    display: inline-flex;
    border: 1px solid var(--theme-border-strong);
    border-radius: 6px;
    overflow: hidden;
  }
  .mode-tab {
    background: transparent;
    border: none;
    color: var(--theme-text-soft);
    padding: 6px 12px;
    font-size: 13px;
    cursor: pointer;
  }
  .mode-tab:hover {
    background: var(--theme-surface-3);
  }
  .mode-tab.active {
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    font-weight: 600;
  }
  .search-box {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    padding: 6px 12px;
    border-radius: 6px;
    font-size: 13px;
    width: 240px;
    outline: none;
    transition: border-color 0.15s;
  }
  .search-box::placeholder {
    color: var(--theme-text-faint);
  }
  .search-box:focus {
    border-color: var(--theme-accent-border);
  }
  .short-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: var(--theme-text-soft);
    cursor: pointer;
    user-select: none;
  }
  .short-toggle input {
    accent-color: var(--theme-accent);
  }
  .panel {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 8px;
    padding: 12px;
    margin-bottom: 12px;
  }
  .filter-row {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    align-items: flex-end;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--theme-text-soft);
  }
  .field-label {
    font-size: 11px;
    color: var(--theme-text-muted);
  }
  .text-input {
    background: var(--theme-input-bg);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    padding: 5px 8px;
    border-radius: 6px;
    font-size: 13px;
    outline: none;
  }
  .text-input:focus {
    border-color: var(--theme-accent-border);
  }
  .text-input.num {
    width: 78px;
  }
  .tag-input-wrap,
  .range-wrap {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .dash {
    color: var(--theme-text-faint);
  }
  .small-btn {
    padding: 5px 10px;
    font-size: 12px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .selected-tags {
    margin-top: 10px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    background: var(--theme-chip-bg);
    color: var(--theme-chip-text);
    border: 1px solid var(--theme-border-strong);
    border-radius: 999px;
    padding: 2px 10px;
    font-size: 11px;
    cursor: pointer;
  }
  .chip:hover {
    border-color: var(--theme-border-focus);
  }
  .chip.on {
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    border-color: var(--theme-accent);
  }
  .chip .count {
    opacity: 0.65;
    font-variant-numeric: tabular-nums;
  }
  .chip .x {
    opacity: 0.8;
  }
  .stats .stat-row {
    display: flex;
    flex-wrap: wrap;
    gap: 18px;
    margin-bottom: 10px;
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 15px;
  }
  .stat .k {
    font-size: 11px;
    color: var(--theme-text-muted);
  }
  .facet {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    margin-top: 8px;
  }
  .facet-label {
    font-size: 11px;
    color: var(--theme-text-muted);
    white-space: nowrap;
    padding-top: 3px;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    margin-bottom: 12px;
  }
  .toolbar .count {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .inline-field {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .pager {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
  }
  .page-label {
    font-variant-numeric: tabular-nums;
    min-width: 56px;
    text-align: center;
  }
  .info {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
    border: 1px solid var(--theme-accent-border);
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 12px;
    margin-bottom: 12px;
  }
  .ghost {
    background: transparent;
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text-soft);
    padding: 6px 12px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
  }
  .ghost:hover:not(:disabled) {
    background: var(--theme-surface-3);
  }
  .ghost.on {
    border-color: var(--theme-accent-border);
    color: var(--theme-accent-soft);
  }
  .ghost:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .primary {
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    border: 1px solid var(--theme-accent);
    padding: 6px 12px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 600;
  }
  .primary:hover:not(:disabled) {
    background: var(--theme-accent-hover);
  }
  .primary:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .muted {
    color: var(--theme-text-muted);
  }
  .small {
    font-size: 11px;
  }
  .error {
    background: var(--theme-danger-bg);
    border: 1px solid var(--theme-danger-border);
    color: var(--theme-danger-text);
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
    margin-bottom: 12px;
  }
  .empty {
    padding: 32px;
    text-align: center;
    border: 1px dashed var(--theme-border-strong);
    border-radius: 8px;
  }
  .empty a {
    color: var(--theme-accent-soft);
  }
  .comment-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .comment-link {
    display: grid;
    grid-template-columns: 64px 1fr;
    grid-template-areas:
      'vpos body'
      'vpos meta';
    gap: 2px 10px;
    padding: 8px 12px;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 8px;
    text-decoration: none;
    color: inherit;
  }
  .comment-link:hover {
    background: var(--theme-surface-4);
    border-color: var(--theme-border-focus);
  }
  .vpos {
    grid-area: vpos;
    align-self: center;
    text-align: right;
    font-size: 12px;
    color: var(--theme-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .comment-body {
    grid-area: body;
    font-size: 14px;
    word-break: break-word;
  }
  .comment-meta {
    grid-area: meta;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 14px;
  }
  .card-wrap {
    position: relative;
  }
  .del-btn {
    position: absolute;
    top: 6px;
    right: 6px;
    z-index: 2;
    /* サムネ上に重ねるので、テーマに関係なく暗いオーバレイで対比を取る。
       文字色はオーバレイ上専用トークンで白系を維持する。 */
    background: var(--theme-overlay-strong);
    color: var(--theme-on-overlay);
    border: 1px solid var(--theme-danger-border);
    width: 26px;
    height: 26px;
    border-radius: 50%;
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
    padding: 0;
    opacity: 0;
    transition: opacity 0.1s;
  }
  .card-wrap:hover .del-btn {
    opacity: 1;
  }
  .del-btn:hover {
    background: var(--theme-danger-bg);
  }
  .del-btn:disabled {
    opacity: 0.5;
    cursor: wait;
  }
  /* 投稿者フィルタはサムネ左上。候補セレクトの上限を超える投稿者にも
     ここから辿り着ける。絞り込み中のカードは常時表示にして状態が分かるように。 */
  .uploader-btn {
    position: absolute;
    top: 6px;
    left: 6px;
    z-index: 2;
    background: var(--theme-overlay-strong);
    color: var(--theme-on-overlay);
    border: 1px solid var(--theme-border-strong);
    width: 26px;
    height: 26px;
    border-radius: 50%;
    cursor: pointer;
    font-size: 12px;
    line-height: 1;
    padding: 0;
    opacity: 0;
    transition: opacity 0.1s;
  }
  .card-wrap:hover .uploader-btn,
  .uploader-btn.on {
    opacity: 1;
  }
  .uploader-btn.on {
    border-color: var(--theme-accent);
  }
  /* プラグインアクションメニューはサムネ右下に。寄与 0 件のとき VideoActionMenu
     自体が何もレンダリングしないので、この div も空のままで邪魔にならない。 */
  .plugin-menu-wrap {
    position: absolute;
    bottom: 6px;
    right: 6px;
    z-index: 2;
    opacity: 0;
    transition: opacity 0.1s;
  }
  .card-wrap:hover .plugin-menu-wrap {
    opacity: 1;
  }
  .card {
    display: block;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 8px;
    overflow: hidden;
    text-decoration: none;
    color: inherit;
    transition:
      background 0.1s,
      border-color 0.1s,
      transform 0.1s;
  }
  .card:hover {
    background: var(--theme-surface-4);
    border-color: var(--theme-border-focus);
    transform: translateY(-1px);
  }
  .thumb-wrap {
    position: relative;
    aspect-ratio: 16 / 9;
    background: var(--theme-bg);
  }
  .thumb {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .thumb-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    color: var(--theme-text-faint);
    font-size: 32px;
  }
  .duration {
    position: absolute;
    right: 6px;
    bottom: 6px;
    background: var(--theme-overlay-strong);
    color: var(--theme-on-overlay);
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .resolution {
    position: absolute;
    left: 6px;
    bottom: 6px;
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    padding: 1px 6px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 500;
  }
  .meta {
    padding: 10px 12px;
  }
  .title {
    font-size: 14px;
    margin: 0 0 6px;
    line-height: 1.3;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    margin-top: 4px;
  }
  .dot {
    color: var(--theme-text-faint);
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 6px;
  }
  .tag {
    background: var(--theme-border);
    color: var(--theme-chip-text);
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 11px;
  }
</style>
