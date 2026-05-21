<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import {
    createMylist,
    deleteMylist,
    listMylists,
    removeFromMylist,
    renameMylist,
    subscribeMylists,
    type Mylist,
  } from '$lib/stores/mylists';
  import {
    createSmartPlaylist,
    deleteSmartPlaylist,
    getSmartPlaylist,
    listSmartPlaylists,
    subscribeSmartPlaylists,
    summarizeFilter,
    updateSmartPlaylist,
    type SmartPlaylist,
    type SmartPlaylistFilter,
  } from '$lib/stores/smartPlaylists';
  import { setQueue, itemHref, type PlaybackQueueItem } from '$lib/stores/playbackQueue';
  import { formatDate, formatDuration, formatNumber } from '$lib/format';

  type Tab = 'mylist' | 'smart';
  let tab = $state<Tab>('mylist');

  // ============== マイリスト ==============
  let mylists = $state<Mylist[]>([]);
  let selectedId = $state<string | null>(null);
  let newName = $state('');
  let editingId = $state<string | null>(null);
  let editingName = $state('');

  function refresh() {
    mylists = listMylists();
    if (!selectedId || !mylists.some((m) => m.id === selectedId)) {
      selectedId = mylists[0]?.id ?? null;
    }
  }

  let unsubMylist: (() => void) | null = null;
  let unsubSmart: (() => void) | null = null;
  onMount(() => {
    refresh();
    refreshSmart();
    unsubMylist = subscribeMylists(refresh);
    unsubSmart = subscribeSmartPlaylists(refreshSmart);
    // URL ?mylistId=... または ?tab=smart で初期選択を制御
    const params = page.url.searchParams;
    const mid = params.get('mylistId');
    if (mid && mylists.some((m) => m.id === mid)) {
      selectedId = mid;
    }
    const t = params.get('tab');
    if (t === 'smart') tab = 'smart';
    // ?edit=<id> でスマートプレイリスト詳細ページの「編集」リンクから
    // モーダルを直接開く動線。detail → list へ戻った時の二度手間を省く。
    const editId = params.get('edit');
    if (editId) {
      const target = getSmartPlaylist(editId);
      if (target) {
        tab = 'smart';
        openEditor(target);
      }
    }
  });
  onDestroy(() => {
    unsubMylist?.();
    unsubSmart?.();
  });

  let selected = $derived(mylists.find((m) => m.id === selectedId) ?? null);

  function onCreate(e: Event) {
    e.preventDefault();
    const name = newName.trim();
    if (!name) return;
    const m = createMylist(name);
    selectedId = m.id;
    newName = '';
  }

  function startRename(m: Mylist) {
    editingId = m.id;
    editingName = m.name;
  }

  function commitRename() {
    if (editingId && editingName.trim()) {
      renameMylist(editingId, editingName.trim());
    }
    editingId = null;
    editingName = '';
  }

  function cancelRename() {
    editingId = null;
    editingName = '';
  }

  function onDelete(m: Mylist) {
    if (m.builtin) return;
    if (!confirm(`「${m.name}」を削除しますか？(${m.items.length} 件の動画リンクも消えます)`))
      return;
    deleteMylist(m.id);
  }

  function onRemoveItem(videoId: string) {
    if (!selectedId) return;
    removeFromMylist(selectedId, videoId);
  }

  function startPlayAllMylist(m: Mylist, startIndex = 0) {
    if (m.items.length === 0) return;
    const items: PlaybackQueueItem[] = m.items.map((v) => ({
      videoId: v.videoId,
      title: v.title,
      thumbnailUrl: v.thumbnailUrl,
      lengthSeconds: v.lengthSeconds,
      source: 'online',
    }));
    const idx = Math.max(0, Math.min(items.length - 1, startIndex));
    setQueue('mylist', m.id, m.name, items, idx);
    void goto(itemHref(items[idx]));
  }

  // ============== スマートプレイリスト ==============
  let smartPlaylists = $state<SmartPlaylist[]>([]);
  function refreshSmart() {
    smartPlaylists = listSmartPlaylists();
  }

  let editorOpen = $state(false);
  let editorTargetId = $state<string | null>(null);
  let editorName = $state('');
  let editorDescription = $state('');
  let editorQ = $state('');
  let editorTagsAnd = $state('');
  let editorTagsOr = $state('');
  let editorUploaderId = $state('');
  let editorMinDuration = $state<number | null>(null);
  let editorMaxDuration = $state<number | null>(null);
  let editorResolution = $state('');
  let editorSortBy = $state('downloaded_at');
  let editorSortOrder = $state<'asc' | 'desc'>('desc');
  let editorLimit = $state<number | null>(100);

  function openEditor(target: SmartPlaylist | null) {
    editorTargetId = target?.id ?? null;
    editorName = target?.name ?? '';
    editorDescription = target?.description ?? '';
    const f = target?.filter ?? {};
    editorQ = f.q ?? '';
    editorTagsAnd = (f.tags ?? []).join(', ');
    editorTagsOr = (f.tagsAny ?? []).join(', ');
    editorUploaderId = f.uploaderId ?? '';
    editorMinDuration = f.minDuration ?? null;
    editorMaxDuration = f.maxDuration ?? null;
    editorResolution = f.resolution ?? '';
    editorSortBy = f.sortBy ?? 'downloaded_at';
    editorSortOrder = f.sortOrder ?? 'desc';
    editorLimit = f.limit ?? 100;
    editorOpen = true;
  }

  function closeEditor() {
    editorOpen = false;
    editorTargetId = null;
  }

  function parseCsv(s: string): string[] {
    return s
      .split(',')
      .map((t) => t.trim())
      .filter((t) => t.length > 0);
  }

  function buildEditorFilter(): SmartPlaylistFilter {
    return {
      q: editorQ,
      tags: parseCsv(editorTagsAnd),
      tagsAny: parseCsv(editorTagsOr),
      uploaderId: editorUploaderId,
      minDuration: editorMinDuration ?? undefined,
      maxDuration: editorMaxDuration ?? undefined,
      resolution: editorResolution,
      sortBy: editorSortBy,
      sortOrder: editorSortOrder,
      limit: editorLimit ?? undefined,
    };
  }

  function saveEditor() {
    const name = editorName.trim();
    if (!name) {
      alert('名前を入力してください');
      return;
    }
    const filter = buildEditorFilter();
    if (editorTargetId) {
      updateSmartPlaylist(editorTargetId, {
        name,
        description: editorDescription,
        filter,
      });
    } else {
      createSmartPlaylist(name, filter, editorDescription);
    }
    closeEditor();
  }

  function onDeleteSmart(p: SmartPlaylist) {
    if (!confirm(`スマートプレイリスト「${p.name}」を削除しますか?`)) return;
    deleteSmartPlaylist(p.id);
  }
</script>

<section>
  <div class="header-row">
    <h2>プレイリスト</h2>
    <div class="tabs" role="tablist">
      <button
        type="button"
        role="tab"
        class="tab"
        class:active={tab === 'mylist'}
        aria-selected={tab === 'mylist'}
        onclick={() => (tab = 'mylist')}
      >
        マイリスト
      </button>
      <button
        type="button"
        role="tab"
        class="tab"
        class:active={tab === 'smart'}
        aria-selected={tab === 'smart'}
        onclick={() => (tab = 'smart')}
      >
        スマートプレイリスト
      </button>
    </div>
  </div>

  {#if tab === 'mylist'}
    <p class="muted">
      ローカルに保存される独自マイリスト。「マイリスト」は組み込みのブックマーク用。
    </p>

    <div class="layout">
      <aside class="sidebar">
        <ul class="mylist-list">
          {#each mylists as m (m.id)}
            <li>
              <div
                role="button"
                tabindex="0"
                class="ml"
                class:active={m.id === selectedId}
                onclick={() => (selectedId = m.id)}
                onkeydown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    selectedId = m.id;
                  }
                }}
              >
                {#if editingId === m.id}
                  <input
                    class="rename"
                    type="text"
                    bind:value={editingName}
                    onclick={(e) => e.stopPropagation()}
                    onblur={commitRename}
                    onkeydown={(e) => {
                      if (e.key === 'Enter') commitRename();
                      if (e.key === 'Escape') cancelRename();
                    }}
                  />
                {:else}
                  <span class="ml-name">{m.name}</span>
                  {#if m.builtin}<span class="badge">標準</span>{/if}
                  <span class="ml-count">{m.items.length}</span>
                {/if}
              </div>
            </li>
          {/each}
        </ul>

        <form class="create" onsubmit={onCreate}>
          <input type="text" placeholder="新しいマイリスト名" bind:value={newName} maxlength="60" />
          <button type="submit" disabled={!newName.trim()}>作成</button>
        </form>
      </aside>

      <div class="detail">
        {#if selected}
          <div class="detail-head">
            <h3>{selected.name}</h3>
            <div class="actions">
              <button
                type="button"
                class="primary"
                disabled={selected.items.length === 0}
                onclick={() => startPlayAllMylist(selected!)}
                title="先頭から順に連続再生"
              >
                ▶ 連続再生
              </button>
              {#if !selected.builtin}
                <button type="button" onclick={() => startRename(selected!)}>名前変更</button>
                <button type="button" class="danger" onclick={() => onDelete(selected!)}
                  >削除</button
                >
              {/if}
            </div>
          </div>
          <p class="muted small">
            {formatNumber(selected.items.length)} 件 · 更新 {formatDate(
              new Date(selected.updatedAt).toISOString(),
            )}
          </p>
          {#if selected.items.length === 0}
            <p class="muted">
              まだ動画がありません。動画ページの「＋ マイリスト」ボタンから追加できます。
            </p>
          {:else}
            <ul class="items">
              {#each selected.items as item, i (item.videoId)}
                <li class="item">
                  <button
                    type="button"
                    class="thumb-link"
                    onclick={() => startPlayAllMylist(selected!, i)}
                    title="ここから連続再生"
                  >
                    {#if item.thumbnailUrl}
                      <img src={item.thumbnailUrl} alt="" loading="lazy" />
                    {:else}
                      <div class="thumb-placeholder"></div>
                    {/if}
                  </button>
                  <div class="info">
                    <a class="title" href="/video/{item.videoId}">{item.title}</a>
                    <div class="row-meta muted">
                      <span>{item.videoId}</span>
                      {#if item.lengthSeconds != null}
                        <span class="dot">·</span><span>{formatDuration(item.lengthSeconds)}</span>
                      {/if}
                      {#if item.viewCounter != null}
                        <span class="dot">·</span><span>再生 {formatNumber(item.viewCounter)}</span>
                      {/if}
                      {#if item.uploaderName}
                        <span class="dot">·</span><span>{item.uploaderName}</span>
                      {/if}
                    </div>
                  </div>
                  <button type="button" class="remove" onclick={() => onRemoveItem(item.videoId)}>
                    外す
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        {:else}
          <p class="muted">マイリストを選択してください。</p>
        {/if}
      </div>
    </div>
  {:else}
    <!-- スマートプレイリスト -->
    <p class="muted">
      フィルタ条件を保存しておくと、開いた時点のライブラリから動的にプレイリストを生成します。
    </p>

    <div class="smart-toolbar">
      <button type="button" class="primary" onclick={() => openEditor(null)}> ＋ 新規作成 </button>
    </div>

    {#if smartPlaylists.length === 0}
      <div class="empty">
        <p class="muted">スマートプレイリストはまだありません。</p>
      </div>
    {:else}
      <ul class="smart-list">
        {#each smartPlaylists as p (p.id)}
          <li class="smart-card">
            <a class="smart-link" href={`/playlists/smart/${p.id}`}>
              <h3 class="smart-name">{p.name}</h3>
              {#if p.description}
                <p class="smart-desc">{p.description}</p>
              {/if}
              <p class="smart-summary">{summarizeFilter(p.filter)}</p>
              <p class="muted small">更新 {formatDate(new Date(p.updatedAt).toISOString())}</p>
            </a>
            <div class="smart-actions">
              <a class="action-link" href={`/playlists/smart/${p.id}`}>開く</a>
              <button type="button" onclick={() => openEditor(p)}>編集</button>
              <button type="button" class="danger" onclick={() => onDeleteSmart(p)}>削除</button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}

    {#if editorOpen}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="modal-backdrop" onclick={closeEditor}>
        <div
          class="modal"
          role="dialog"
          aria-modal="true"
          tabindex="-1"
          onclick={(e) => e.stopPropagation()}
          onkeydown={(e) => {
            if (e.key === 'Escape') closeEditor();
          }}
        >
          <header class="modal-head">
            <h3>{editorTargetId ? '編集' : '新規作成'} — スマートプレイリスト</h3>
            <button type="button" class="modal-close" onclick={closeEditor}>×</button>
          </header>

          <div class="modal-body">
            <label class="field">
              <span>名前 <span class="req">*</span></span>
              <input type="text" bind:value={editorName} maxlength="60" />
            </label>

            <label class="field">
              <span>メモ (任意)</span>
              <input type="text" bind:value={editorDescription} maxlength="200" />
            </label>

            <fieldset class="filter-set">
              <legend>フィルタ条件</legend>

              <label class="field">
                <span>キーワード</span>
                <input type="text" bind:value={editorQ} placeholder="タイトル / タグ / コメント" />
              </label>

              <label class="field">
                <span>タグ AND (カンマ区切り、全タグを含む)</span>
                <input type="text" bind:value={editorTagsAnd} placeholder="例: ボカロ, 名作" />
              </label>

              <label class="field">
                <span>タグ OR (カンマ区切り、いずれか)</span>
                <input type="text" bind:value={editorTagsOr} placeholder="例: 替え歌, 弾いてみた" />
              </label>

              <label class="field">
                <span>投稿者 ID</span>
                <input type="text" bind:value={editorUploaderId} placeholder="数字 ID" />
              </label>

              <div class="row-2">
                <label class="field">
                  <span>長さ最小 (秒)</span>
                  <input type="number" min="0" bind:value={editorMinDuration} />
                </label>
                <label class="field">
                  <span>長さ最大 (秒)</span>
                  <input type="number" min="0" bind:value={editorMaxDuration} />
                </label>
              </div>

              <label class="field">
                <span>解像度 (例: 1280x720)</span>
                <input type="text" bind:value={editorResolution} />
              </label>

              <div class="row-2">
                <label class="field">
                  <span>並び順</span>
                  <select bind:value={editorSortBy}>
                    <option value="downloaded_at">DL 日時</option>
                    <option value="posted_at">投稿日時</option>
                    <option value="title">タイトル</option>
                    <option value="view_count">再生回数</option>
                    <option value="comment_count">コメ数</option>
                    <option value="mylist_count">マイリス数</option>
                    <option value="play_count">ローカル再生回数</option>
                    <option value="last_played_at">最終再生</option>
                    <option value="duration_sec">長さ</option>
                    <option value="random">ランダム</option>
                  </select>
                </label>
                <label class="field">
                  <span>方向</span>
                  <select bind:value={editorSortOrder}>
                    <option value="desc">降順</option>
                    <option value="asc">昇順</option>
                  </select>
                </label>
              </div>

              <label class="field">
                <span>上限件数 (空欄=既定 100、最大 500)</span>
                <input type="number" min="1" max="500" bind:value={editorLimit} />
              </label>
            </fieldset>
          </div>

          <footer class="modal-foot">
            <button type="button" onclick={closeEditor}>キャンセル</button>
            <button type="button" class="primary" onclick={saveEditor}>保存</button>
          </footer>
        </div>
      </div>
    {/if}
  {/if}
</section>

<style>
  h2 {
    margin-top: 0;
  }
  .header-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 10px;
  }
  .tabs {
    display: flex;
    gap: 4px;
    border: 1px solid var(--theme-border);
    border-radius: 6px;
    padding: 2px;
    background: var(--theme-surface-2);
  }
  .tab {
    background: transparent;
    border: 0;
    color: var(--theme-text-muted);
    padding: 6px 12px;
    border-radius: 4px;
    font-size: 13px;
    cursor: pointer;
  }
  .tab.active {
    background: var(--theme-accent);
    color: #fff;
  }

  .muted {
    color: var(--theme-text-muted);
  }
  .small {
    font-size: 12px;
  }
  .layout {
    display: grid;
    grid-template-columns: 240px 1fr;
    gap: 16px;
    margin-top: 16px;
  }
  @media (max-width: 800px) {
    .layout {
      grid-template-columns: 1fr;
    }
  }
  .sidebar {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 8px;
    padding: 8px;
    align-self: start;
  }
  .mylist-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ml {
    width: 100%;
    text-align: left;
    background: transparent;
    border: 1px solid transparent;
    color: var(--theme-text-soft);
    border-radius: 6px;
    padding: 8px 10px;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  .ml:hover {
    background: var(--theme-border);
  }
  .ml.active {
    background: var(--theme-border-strong);
    border-color: var(--theme-border-focus);
    color: var(--theme-text);
  }
  .ml-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ml-count {
    color: var(--theme-text-muted);
    font-size: 11px;
  }
  .badge {
    background: var(--theme-accent);
    color: white;
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
  }
  .rename {
    flex: 1;
    background: var(--theme-input-bg);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    border-radius: 4px;
    padding: 2px 6px;
    font-size: 13px;
    min-width: 0;
  }
  .create {
    display: flex;
    gap: 6px;
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--theme-border-strong);
  }
  .create input {
    flex: 1;
    background: var(--theme-input-bg);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    border-radius: 4px;
    padding: 4px 8px;
    font-size: 12px;
    min-width: 0;
  }
  .create button,
  .actions button {
    background: var(--theme-border);
    border: 1px solid var(--theme-surface-hover);
    color: var(--theme-text);
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .create button:hover,
  .actions button:hover {
    background: var(--theme-border-strong);
  }
  .create button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .actions button.primary,
  .smart-toolbar .primary,
  .modal-foot .primary {
    background: var(--theme-accent);
    border-color: var(--theme-accent);
    color: #fff;
    font-weight: 600;
  }
  .actions button.primary:hover,
  .smart-toolbar .primary:hover,
  .modal-foot .primary:hover {
    background: var(--theme-accent-hover);
  }
  .actions button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .danger {
    border-color: var(--theme-danger-border) !important;
    color: var(--theme-danger-text) !important;
  }
  .danger:hover {
    background: var(--theme-danger-bg) !important;
  }
  .detail {
    min-width: 0;
  }
  .detail-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .detail-head h3 {
    margin: 0;
    font-size: 16px;
  }
  .actions {
    display: flex;
    gap: 6px;
  }
  .items {
    list-style: none;
    padding: 0;
    margin: 12px 0 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .item {
    display: grid;
    grid-template-columns: 140px 1fr auto;
    gap: 12px;
    align-items: center;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 8px;
    padding: 8px;
  }
  .thumb-link {
    background: transparent;
    border: 0;
    padding: 0;
    cursor: pointer;
  }
  .thumb-link img,
  .thumb-placeholder {
    width: 140px;
    height: 78px;
    object-fit: cover;
    background: var(--theme-bg);
    border-radius: 4px;
    display: block;
  }
  .thumb-placeholder {
    border: 1px dashed var(--theme-border-strong);
  }
  .info {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .title {
    color: var(--theme-text);
    text-decoration: none;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title:hover {
    text-decoration: underline;
  }
  .row-meta {
    font-size: 12px;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    align-items: center;
  }
  .dot {
    color: var(--theme-text-faint);
  }
  .remove {
    background: transparent;
    border: 1px solid var(--theme-surface-hover);
    color: var(--theme-text-soft);
    border-radius: 6px;
    padding: 4px 8px;
    font-size: 12px;
    cursor: pointer;
  }
  .remove:hover {
    background: var(--theme-border);
    color: var(--theme-danger-text);
    border-color: var(--theme-danger-border);
  }

  /* ---------------- スマートプレイリスト ---------------- */
  .smart-toolbar {
    margin: 12px 0;
  }
  .smart-toolbar button {
    background: var(--theme-accent);
    color: #fff;
    border: 1px solid var(--theme-accent);
    border-radius: 6px;
    padding: 6px 14px;
    font-size: 13px;
    cursor: pointer;
    font-weight: 600;
  }
  .smart-toolbar button:hover {
    background: var(--theme-accent-hover);
  }
  .empty {
    border: 1px dashed var(--theme-border-strong);
    border-radius: 8px;
    padding: 32px;
    text-align: center;
  }
  .smart-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 12px;
  }
  .smart-card {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 8px;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .smart-card:hover {
    border-color: var(--theme-border-focus);
  }
  .smart-link {
    display: block;
    color: inherit;
    text-decoration: none;
  }
  .smart-name {
    font-size: 14px;
    font-weight: 600;
    margin: 0;
    color: var(--theme-text);
  }
  .smart-desc {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--theme-text-muted);
  }
  .smart-summary {
    font-size: 11px;
    color: var(--theme-text-soft);
    margin: 6px 0;
    line-height: 1.5;
    word-break: break-word;
  }
  .smart-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    border-top: 1px solid var(--theme-border);
    padding-top: 8px;
  }
  .smart-actions .action-link {
    background: var(--theme-accent);
    color: #fff;
    border: 1px solid var(--theme-accent);
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 12px;
    text-decoration: none;
    font-weight: 600;
  }
  .smart-actions .action-link:hover {
    background: var(--theme-accent-hover);
  }
  .smart-actions button {
    background: var(--theme-border);
    border: 1px solid var(--theme-surface-hover);
    color: var(--theme-text);
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .smart-actions button:hover {
    background: var(--theme-border-strong);
  }

  /* ---------------- モーダル ---------------- */
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }
  .modal {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border-strong);
    border-radius: 8px;
    width: min(560px, 92vw);
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    color: var(--theme-text);
  }
  .modal-head {
    padding: 12px 16px;
    border-bottom: 1px solid var(--theme-border);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .modal-head h3 {
    margin: 0;
    font-size: 14px;
  }
  .modal-close {
    background: transparent;
    border: 0;
    color: var(--theme-text-muted);
    font-size: 22px;
    cursor: pointer;
    line-height: 1;
    padding: 0 4px;
  }
  .modal-close:hover {
    color: var(--theme-text);
  }
  .modal-body {
    padding: 16px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .modal-foot {
    padding: 12px 16px;
    border-top: 1px solid var(--theme-border);
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .modal-foot button {
    background: var(--theme-border);
    border: 1px solid var(--theme-surface-hover);
    color: var(--theme-text);
    border-radius: 6px;
    padding: 6px 14px;
    font-size: 13px;
    cursor: pointer;
  }
  .modal-foot button:hover {
    background: var(--theme-border-strong);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
  }
  .field span {
    color: var(--theme-text-soft);
  }
  .req {
    color: var(--theme-danger-text);
  }
  .field input,
  .field select {
    background: var(--theme-input-bg);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    border-radius: 4px;
    padding: 6px 8px;
    font-size: 13px;
  }
  .row-2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .filter-set {
    border: 1px solid var(--theme-border);
    border-radius: 6px;
    padding: 10px 12px;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .filter-set legend {
    padding: 0 6px;
    color: var(--theme-text-soft);
    font-size: 12px;
  }
</style>
