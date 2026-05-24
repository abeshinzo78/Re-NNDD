<script lang="ts">
  /**
   * ランキング画面の「NG 設定」パネル。
   * 既存の `ngRules` ストアに対して、`scopeRanking=true` のルールだけを
   * 追加・編集・削除する UI。
   *
   *  - タグ (lock / user / both, exact / partial)
   *  - 投稿者 ID / 投稿者名 (exact / partial)
   *  - 動画タイトル (exact / partial)
   *  - 動画 ID
   */
  import {
    addNgRule,
    deleteNgRule,
    listNgRules,
    subscribeNgRules,
    updateNgRule,
    type NgMatchMode,
    type NgRule,
    type NgTagKind,
  } from '$lib/stores/ngRules';
  import { onMount, onDestroy } from 'svelte';

  let rules = $state<NgRule[]>([]);
  let unsub: (() => void) | null = null;
  onMount(() => {
    rules = listNgRules();
    unsub = subscribeNgRules(() => (rules = listNgRules()));
  });
  onDestroy(() => unsub?.());

  let rankingRules = $derived(rules.filter((r) => r.scopeRanking));
  let tagRules = $derived(rankingRules.filter((r) => r.targetType === 'tag'));
  let uploaderIdRules = $derived(rankingRules.filter((r) => r.targetType === 'uploader'));
  let uploaderNameRules = $derived(rankingRules.filter((r) => r.targetType === 'uploader_name'));
  let titleRules = $derived(rankingRules.filter((r) => r.targetType === 'video_title'));
  let videoIdRules = $derived(rankingRules.filter((r) => r.targetType === 'video_id'));

  // ---- タグ ----
  let tagKind = $state<NgTagKind>('both');
  let tagMode = $state<NgMatchMode>('partial');
  let tagInput = $state('');
  let tagBulkOpen = $state(false);
  let tagBulkText = $state('');

  function addTag() {
    const p = tagInput.trim();
    if (!p) return;
    addNgRule({
      targetType: 'tag',
      matchMode: tagMode,
      pattern: p,
      scopeRanking: true,
      scopeSearch: false,
      scopeComment: false,
      enabled: true,
      tagKind,
    });
    tagInput = '';
  }

  function addTagBulk() {
    const lines = tagBulkText
      .split(/\r?\n|,|、/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (lines.length === 0) return;
    for (const p of lines) {
      addNgRule({
        targetType: 'tag',
        matchMode: tagMode,
        pattern: p,
        scopeRanking: true,
        scopeSearch: false,
        scopeComment: false,
        enabled: true,
        tagKind,
      });
    }
    tagBulkText = '';
    tagBulkOpen = false;
  }

  // ---- 投稿者 ID ----
  let upIdInput = $state('');
  let upIdBulkOpen = $state(false);
  let upIdBulkText = $state('');

  function isNumericId(s: string): boolean {
    return /^\d+$/.test(s);
  }

  function addUpId() {
    const p = upIdInput.trim();
    if (!isNumericId(p)) return;
    addNgRule({
      targetType: 'uploader',
      matchMode: 'exact',
      pattern: p,
      scopeRanking: true,
      scopeSearch: false,
      scopeComment: false,
      enabled: true,
    });
    upIdInput = '';
  }

  function addUpIdBulk() {
    const lines = upIdBulkText
      .split(/\r?\n|,|、|\s+/)
      .map((s) => s.trim())
      .filter((s) => isNumericId(s));
    if (lines.length === 0) return;
    for (const p of lines) {
      addNgRule({
        targetType: 'uploader',
        matchMode: 'exact',
        pattern: p,
        scopeRanking: true,
        scopeSearch: false,
        scopeComment: false,
        enabled: true,
      });
    }
    upIdBulkText = '';
    upIdBulkOpen = false;
  }

  // ---- 投稿者名 ----
  let upNameMode = $state<NgMatchMode>('exact');
  let upNameInput = $state('');
  let upNameBulkOpen = $state(false);
  let upNameBulkText = $state('');

  function addUpName() {
    const p = upNameInput.trim();
    if (!p) return;
    addNgRule({
      targetType: 'uploader_name',
      matchMode: upNameMode,
      pattern: p,
      scopeRanking: true,
      scopeSearch: false,
      scopeComment: false,
      enabled: true,
    });
    upNameInput = '';
  }

  function addUpNameBulk() {
    const lines = upNameBulkText
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (lines.length === 0) return;
    for (const p of lines) {
      addNgRule({
        targetType: 'uploader_name',
        matchMode: upNameMode,
        pattern: p,
        scopeRanking: true,
        scopeSearch: false,
        scopeComment: false,
        enabled: true,
      });
    }
    upNameBulkText = '';
    upNameBulkOpen = false;
  }

  // ---- 動画タイトル ----
  let titleMode = $state<NgMatchMode>('partial');
  let titleInput = $state('');
  let titleBulkOpen = $state(false);
  let titleBulkText = $state('');

  function addTitle() {
    const p = titleInput.trim();
    if (!p) return;
    addNgRule({
      targetType: 'video_title',
      matchMode: titleMode,
      pattern: p,
      scopeRanking: true,
      scopeSearch: false,
      scopeComment: false,
      enabled: true,
    });
    titleInput = '';
  }

  function addTitleBulk() {
    const lines = titleBulkText
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (lines.length === 0) return;
    for (const p of lines) {
      addNgRule({
        targetType: 'video_title',
        matchMode: titleMode,
        pattern: p,
        scopeRanking: true,
        scopeSearch: false,
        scopeComment: false,
        enabled: true,
      });
    }
    titleBulkText = '';
    titleBulkOpen = false;
  }

  // ---- 動画 ID ----
  let vidInput = $state('');
  let vidBulkOpen = $state(false);
  let vidBulkText = $state('');

  function addVid() {
    const p = vidInput.trim();
    if (!p) return;
    addNgRule({
      targetType: 'video_id',
      matchMode: 'exact',
      pattern: p,
      scopeRanking: true,
      scopeSearch: false,
      scopeComment: false,
      enabled: true,
    });
    vidInput = '';
  }

  function addVidBulk() {
    const lines = vidBulkText
      .split(/\r?\n|,|、|\s+/)
      .map((s) => s.trim())
      .filter(Boolean);
    if (lines.length === 0) return;
    for (const p of lines) {
      addNgRule({
        targetType: 'video_id',
        matchMode: 'exact',
        pattern: p,
        scopeRanking: true,
        scopeSearch: false,
        scopeComment: false,
        enabled: true,
      });
    }
    vidBulkText = '';
    vidBulkOpen = false;
  }

  function removeRule(id: string) {
    // パネルはランキング scope のみ責任を持つ。他 scope (search / comment)
    // も併用しているルールは `scopeRanking` だけ落とす。
    const r = rules.find((x) => x.id === id);
    if (!r) return;
    if (r.scopeSearch || r.scopeComment) {
      updateNgRule(id, { scopeRanking: false });
    } else {
      deleteNgRule(id);
    }
  }

  function describeMode(m: NgMatchMode): string {
    return m === 'exact' ? '完全' : m === 'partial' ? '部分' : '正規';
  }
  function describeTagKind(k: NgTagKind | undefined): string {
    return k === 'lock' ? 'ロック' : k === 'user' ? 'ユーザー' : '両方';
  }

  // 親へ通知用 (件数表示)
  let { onChange }: { onChange?: (total: number) => void } = $props();
  $effect(() => {
    onChange?.(rankingRules.length);
  });
</script>

<div class="panel">
  <!-- タグ -->
  <div class="card">
    <h3 class="card-title"><span class="prohibit">&#x1F6AB;</span>タグ</h3>
    <div class="row">
      <label class="radio">
        <input type="radio" bind:group={tagKind} value="lock" /><span>ロックタグ</span>
      </label>
      <label class="radio">
        <input type="radio" bind:group={tagKind} value="user" /><span>ユーザータグ</span>
      </label>
      <label class="radio">
        <input type="radio" bind:group={tagKind} value="both" /><span>両方</span>
      </label>
    </div>
    <div class="row">
      <label class="radio">
        <input type="radio" bind:group={tagMode} value="exact" /><span>完全一致</span>
      </label>
      <label class="radio">
        <input type="radio" bind:group={tagMode} value="partial" /><span>部分一致</span>
      </label>
    </div>
    <div class="row input-row">
      <input
        type="text"
        placeholder="タグ名を入力"
        bind:value={tagInput}
        onkeydown={(e) => e.key === 'Enter' && addTag()}
      />
      <button type="button" class="primary" onclick={addTag}>追加</button>
    </div>
    <button
      type="button"
      class="toggle"
      onclick={() => (tagBulkOpen = !tagBulkOpen)}
      aria-expanded={tagBulkOpen}
    >
      <span class="caret">{tagBulkOpen ? '▼' : '▶'}</span> 複数タグを一括追加
    </button>
    {#if tagBulkOpen}
      <div class="bulk">
        <textarea
          rows="4"
          placeholder="1 行に 1 タグ、または ',' / '、' 区切り"
          bind:value={tagBulkText}
        ></textarea>
        <button type="button" class="primary" onclick={addTagBulk}>一括追加</button>
      </div>
    {/if}
    {#if tagRules.length > 0}
      <ul class="rule-list">
        {#each tagRules as r (r.id)}
          <li>
            <span class="pill">{describeTagKind(r.tagKind)}</span>
            <span class="pill">{describeMode(r.matchMode)}</span>
            <code>{r.pattern}</code>
            <button type="button" class="x" aria-label="削除" onclick={() => removeRule(r.id)}
              >×</button
            >
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <!-- 投稿者 -->
  <div class="card">
    <h3 class="card-title"><span class="prohibit">&#x1F6AB;</span>投稿者</h3>

    <div class="sub-label">ID</div>
    <div class="row input-row">
      <input
        type="text"
        placeholder="投稿者ID（数字）"
        bind:value={upIdInput}
        onkeydown={(e) => e.key === 'Enter' && addUpId()}
      />
      <button type="button" class="primary" onclick={addUpId}>追加</button>
    </div>
    <button
      type="button"
      class="toggle"
      onclick={() => (upIdBulkOpen = !upIdBulkOpen)}
      aria-expanded={upIdBulkOpen}
    >
      <span class="caret">{upIdBulkOpen ? '▼' : '▶'}</span> 複数IDを一括追加
    </button>
    {#if upIdBulkOpen}
      <div class="bulk">
        <textarea
          rows="3"
          placeholder="1 行に 1 ID、または改行 / ',' / 空白区切り"
          bind:value={upIdBulkText}
        ></textarea>
        <button type="button" class="primary" onclick={addUpIdBulk}>一括追加</button>
      </div>
    {/if}
    {#if uploaderIdRules.length > 0}
      <ul class="rule-list">
        {#each uploaderIdRules as r (r.id)}
          <li>
            <code>{r.pattern}</code>
            <button type="button" class="x" aria-label="削除" onclick={() => removeRule(r.id)}
              >×</button
            >
          </li>
        {/each}
      </ul>
    {/if}

    <div class="sub-label">名前</div>
    <div class="row">
      <label class="radio">
        <input type="radio" bind:group={upNameMode} value="exact" /><span>完全一致</span>
      </label>
      <label class="radio">
        <input type="radio" bind:group={upNameMode} value="partial" /><span>部分一致</span>
      </label>
    </div>
    <div class="row input-row">
      <input
        type="text"
        placeholder="投稿者名"
        bind:value={upNameInput}
        onkeydown={(e) => e.key === 'Enter' && addUpName()}
      />
      <button type="button" class="primary" onclick={addUpName}>追加</button>
    </div>
    <button
      type="button"
      class="toggle"
      onclick={() => (upNameBulkOpen = !upNameBulkOpen)}
      aria-expanded={upNameBulkOpen}
    >
      <span class="caret">{upNameBulkOpen ? '▼' : '▶'}</span> 複数名を一括追加
    </button>
    {#if upNameBulkOpen}
      <div class="bulk">
        <textarea rows="3" placeholder="1 行に 1 名前" bind:value={upNameBulkText}></textarea>
        <button type="button" class="primary" onclick={addUpNameBulk}>一括追加</button>
      </div>
    {/if}
    {#if uploaderNameRules.length > 0}
      <ul class="rule-list">
        {#each uploaderNameRules as r (r.id)}
          <li>
            <span class="pill">{describeMode(r.matchMode)}</span>
            <code>{r.pattern}</code>
            <button type="button" class="x" aria-label="削除" onclick={() => removeRule(r.id)}
              >×</button
            >
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <!-- 動画タイトル -->
  <div class="card">
    <h3 class="card-title"><span class="prohibit">&#x1F6AB;</span>動画タイトル</h3>
    <div class="row">
      <label class="radio">
        <input type="radio" bind:group={titleMode} value="exact" /><span>完全一致</span>
      </label>
      <label class="radio">
        <input type="radio" bind:group={titleMode} value="partial" /><span>部分一致</span>
      </label>
    </div>
    <div class="row input-row">
      <input
        type="text"
        placeholder="タイトルを入力"
        bind:value={titleInput}
        onkeydown={(e) => e.key === 'Enter' && addTitle()}
      />
      <button type="button" class="primary" onclick={addTitle}>追加</button>
    </div>
    <button
      type="button"
      class="toggle"
      onclick={() => (titleBulkOpen = !titleBulkOpen)}
      aria-expanded={titleBulkOpen}
    >
      <span class="caret">{titleBulkOpen ? '▼' : '▶'}</span> 複数タイトルを一括追加
    </button>
    {#if titleBulkOpen}
      <div class="bulk">
        <textarea rows="3" placeholder="1 行に 1 タイトル" bind:value={titleBulkText}></textarea>
        <button type="button" class="primary" onclick={addTitleBulk}>一括追加</button>
      </div>
    {/if}
    {#if titleRules.length > 0}
      <ul class="rule-list">
        {#each titleRules as r (r.id)}
          <li>
            <span class="pill">{describeMode(r.matchMode)}</span>
            <code>{r.pattern}</code>
            <button type="button" class="x" aria-label="削除" onclick={() => removeRule(r.id)}
              >×</button
            >
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <!-- 動画 ID -->
  <div class="card">
    <h3 class="card-title"><span class="prohibit">&#x1F6AB;</span>動画ID</h3>
    <div class="row input-row">
      <input
        type="text"
        placeholder="sm12345678"
        bind:value={vidInput}
        onkeydown={(e) => e.key === 'Enter' && addVid()}
      />
      <button type="button" class="primary" onclick={addVid}>追加</button>
    </div>
    <button
      type="button"
      class="toggle"
      onclick={() => (vidBulkOpen = !vidBulkOpen)}
      aria-expanded={vidBulkOpen}
    >
      <span class="caret">{vidBulkOpen ? '▼' : '▶'}</span> 複数IDを一括追加
    </button>
    {#if vidBulkOpen}
      <div class="bulk">
        <textarea
          rows="3"
          placeholder="1 行に 1 ID、または改行 / ',' / 空白区切り"
          bind:value={vidBulkText}
        ></textarea>
        <button type="button" class="primary" onclick={addVidBulk}>一括追加</button>
      </div>
    {/if}
    {#if videoIdRules.length > 0}
      <ul class="rule-list">
        {#each videoIdRules as r (r.id)}
          <li>
            <code>{r.pattern}</code>
            <button type="button" class="x" aria-label="削除" onclick={() => removeRule(r.id)}
              >×</button
            >
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .panel {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 12px;
    margin: 12px 0;
  }
  .card {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 8px;
    padding: 12px;
  }
  .card-title {
    margin: 0 0 10px;
    font-size: 14px;
    font-weight: 700;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--theme-text);
  }
  .prohibit {
    color: var(--theme-danger-text, #e0245e);
    font-size: 16px;
  }
  .sub-label {
    margin-top: 10px;
    margin-bottom: 6px;
    font-size: 12px;
    color: var(--theme-text-soft);
    font-weight: 600;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    margin-bottom: 6px;
  }
  .input-row {
    margin-top: 4px;
    margin-bottom: 6px;
  }
  .input-row input[type='text'] {
    flex: 1;
    min-width: 0;
  }
  input[type='text'] {
    background: var(--theme-input-bg);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    border-radius: 6px;
    padding: 6px 10px;
    font-size: 13px;
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    background: var(--theme-input-bg);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    border-radius: 6px;
    padding: 6px 10px;
    font-size: 13px;
    resize: vertical;
  }
  .radio {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: var(--theme-text-soft);
    cursor: pointer;
  }
  .radio input {
    accent-color: var(--theme-accent);
  }
  button {
    background: var(--theme-border);
    border: 1px solid var(--theme-surface-hover);
    color: var(--theme-text);
    border-radius: 6px;
    padding: 6px 14px;
    font-size: 13px;
    cursor: pointer;
  }
  button:hover {
    background: var(--theme-border-strong);
  }
  button.primary {
    background: var(--theme-accent);
    border-color: var(--theme-accent);
    color: var(--theme-accent-fg);
  }
  button.primary:hover {
    background: var(--theme-accent-hover);
  }
  button.toggle {
    margin-top: 4px;
    background: var(--theme-accent-bg, rgba(99, 102, 241, 0.18));
    border: 1px solid var(--theme-accent-soft, rgba(99, 102, 241, 0.4));
    color: var(--theme-accent-soft, #a5b4fc);
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 12px;
  }
  button.toggle:hover {
    background: var(--theme-accent-bg, rgba(99, 102, 241, 0.28));
  }
  .caret {
    font-size: 10px;
    margin-right: 2px;
  }
  .bulk {
    margin-top: 6px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .bulk button {
    align-self: flex-end;
  }
  .rule-list {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 220px;
    overflow-y: auto;
  }
  .rule-list li {
    display: flex;
    align-items: center;
    gap: 6px;
    background: var(--theme-surface);
    border: 1px solid var(--theme-border);
    border-radius: 4px;
    padding: 3px 6px;
    font-size: 12px;
  }
  .rule-list code {
    background: transparent;
    color: var(--theme-warning-text, #facc15);
    word-break: break-all;
    flex: 1;
  }
  .pill {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
    padding: 1px 6px;
    border-radius: 999px;
    font-size: 10px;
  }
  .x {
    background: transparent;
    border: none;
    color: var(--theme-text-muted);
    padding: 0 4px;
    font-size: 14px;
    cursor: pointer;
  }
  .x:hover {
    color: var(--theme-danger-text);
    background: transparent;
  }
</style>
