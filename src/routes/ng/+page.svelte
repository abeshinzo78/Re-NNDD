<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    addNgRule,
    clearAllNgRules,
    deleteNgRule,
    importNgRules,
    isValidRegex,
    listNgRules,
    subscribeNgRules,
    updateNgRule,
    type NgMatchMode,
    type NgRule,
    type NgTargetType,
  } from '$lib/stores/ngRules';
  import { formatDate, formatNumber } from '$lib/format';

  const TARGET_LABELS: Record<NgTargetType, string> = {
    video_title: '動画タイトル',
    uploader: '投稿者',
    video_id: '動画 ID',
    tag: 'タグ',
    category: 'カテゴリ',
    comment_body: 'コメ本文',
    comment_user: 'コメ投稿者',
  };
  const MODE_LABELS: Record<NgMatchMode, string> = {
    exact: '完全一致',
    partial: '部分一致',
    regex: '正規表現',
  };

  let rules = $state<NgRule[]>([]);
  let unsub: (() => void) | null = null;

  onMount(() => {
    rules = listNgRules();
    unsub = subscribeNgRules(() => (rules = listNgRules()));
  });
  onDestroy(() => unsub?.());

  // New-rule form state
  let nTarget = $state<NgTargetType>('comment_body');
  let nMode = $state<NgMatchMode>('partial');
  let nPattern = $state('');
  let nNote = $state('');
  let nScopeRanking = $state(false);
  let nScopeSearch = $state(true);
  let nScopeComment = $state(true);
  let nError = $state<string | null>(null);

  // Filter UI
  let filterTarget = $state<'' | NgTargetType>('');
  let filterText = $state('');
  let showDisabled = $state(true);

  let visible = $derived(
    rules.filter((r) => {
      if (filterTarget && r.targetType !== filterTarget) return false;
      if (!showDisabled && !r.enabled) return false;
      if (filterText && !r.pattern.includes(filterText) && !(r.note ?? '').includes(filterText)) {
        return false;
      }
      return true;
    }),
  );

  // Default scope by target type
  $effect(() => {
    if (nTarget === 'comment_body' || nTarget === 'comment_user') {
      nScopeComment = true;
      nScopeSearch = false;
      nScopeRanking = false;
    } else {
      nScopeComment = false;
      nScopeSearch = true;
    }
  });

  function onAdd(e: Event) {
    e.preventDefault();
    nError = null;
    const pat = nPattern.trim();
    if (!pat) {
      nError = 'パターンは必須です';
      return;
    }
    if (nMode === 'regex' && !isValidRegex(pat)) {
      nError = '正規表現が不正です';
      return;
    }
    addNgRule({
      targetType: nTarget,
      matchMode: nMode,
      pattern: pat,
      scopeRanking: nScopeRanking,
      scopeSearch: nScopeSearch,
      scopeComment: nScopeComment,
      enabled: true,
      note: nNote.trim() || undefined,
    });
    nPattern = '';
    nNote = '';
  }

  function onDelete(id: string) {
    if (!confirm('このルールを削除しますか？')) return;
    deleteNgRule(id);
  }

  function onClearAll() {
    if (!confirm(`全 ${rules.length} 件のルールを削除しますか？`)) return;
    clearAllNgRules();
  }

  // ---- Import / Export ----
  let importMessage = $state<string | null>(null);

  async function onImport(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    try {
      const text = await file.text();
      const data = file.name.toLowerCase().endsWith('.csv')
        ? parseCsv(text)
        : (JSON.parse(text) as Partial<NgRule>[]);
      const added = importNgRules(data);
      importMessage = `${added} 件をインポートしました`;
    } catch (err) {
      importMessage = `インポート失敗: ${err}`;
    }
  }

  function parseCsv(text: string): Partial<NgRule>[] {
    const lines = text.split(/\r?\n/).filter((l) => l.trim());
    if (lines.length === 0) return [];
    const header = lines[0].split(',').map((h) => h.trim());
    const idx = (k: string) => header.indexOf(k);
    const out: Partial<NgRule>[] = [];
    for (let i = 1; i < lines.length; i++) {
      const cols = splitCsvRow(lines[i]);
      const rule: Partial<NgRule> = {
        targetType: cols[idx('target_type')] as NgTargetType,
        matchMode: cols[idx('match_mode')] as NgMatchMode,
        pattern: cols[idx('pattern')],
        scopeRanking: cols[idx('scope_ranking')] === 'true' || cols[idx('scope_ranking')] === '1',
        scopeSearch: cols[idx('scope_search')] === 'true' || cols[idx('scope_search')] === '1',
        scopeComment: cols[idx('scope_comment')] === 'true' || cols[idx('scope_comment')] === '1',
        note: cols[idx('note')] || undefined,
      };
      out.push(rule);
    }
    return out;
  }

  function splitCsvRow(row: string): string[] {
    const out: string[] = [];
    let cur = '';
    let inQ = false;
    for (let i = 0; i < row.length; i++) {
      const ch = row[i];
      if (inQ) {
        if (ch === '"' && row[i + 1] === '"') {
          cur += '"';
          i++;
        } else if (ch === '"') {
          inQ = false;
        } else {
          cur += ch;
        }
      } else if (ch === '"') {
        inQ = true;
      } else if (ch === ',') {
        out.push(cur);
        cur = '';
      } else {
        cur += ch;
      }
    }
    out.push(cur);
    return out;
  }

  function onExport() {
    const blob = new Blob([JSON.stringify(rules, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `nndd-ng-rules-${Date.now()}.json`;
    a.click();
    URL.revokeObjectURL(url);
  }
</script>

<section>
  <h2>NG ルール</h2>
  <p class="muted">
    動画タイトル / 投稿者 / 動画 ID / タグ / カテゴリ / コメ本文 / コメ投稿者の 7 種ターゲット。
    完全一致・部分一致・正規表現の 3
    種モード。検索結果・コメント描画に即時反映される（データは保持される）。
  </p>

  <form class="add" onsubmit={onAdd}>
    <h3>ルール追加</h3>
    <div class="row">
      <label>
        ターゲット
        <select bind:value={nTarget}>
          {#each Object.entries(TARGET_LABELS) as [key, label] (key)}
            <option value={key}>{label}</option>
          {/each}
        </select>
      </label>
      <label>
        マッチ
        <select bind:value={nMode}>
          {#each Object.entries(MODE_LABELS) as [key, label] (key)}
            <option value={key}>{label}</option>
          {/each}
        </select>
      </label>
      <label class="grow">
        パターン
        <input
          type="text"
          bind:value={nPattern}
          placeholder={nTarget === 'uploader'
            ? 'user/12345 または channel/ch12345'
            : 'NG にする文字列'}
        />
      </label>
    </div>
    <div class="row">
      <label class="grow">
        メモ（任意）
        <input type="text" bind:value={nNote} placeholder="例: 地雷姫リストより" />
      </label>
    </div>
    <div class="row scopes">
      <span class="muted">適用範囲:</span>
      <label class="chip"><input type="checkbox" bind:checked={nScopeSearch} />検索</label>
      <label class="chip"><input type="checkbox" bind:checked={nScopeRanking} />ランキング</label>
      <label class="chip"><input type="checkbox" bind:checked={nScopeComment} />コメ</label>
      <button type="submit" class="primary">追加</button>
    </div>
    {#if nError}<div class="error">{nError}</div>{/if}
  </form>

  <div class="bar">
    <div class="bar-left">
      <label>
        ターゲットで絞込
        <select bind:value={filterTarget}>
          <option value="">すべて</option>
          {#each Object.entries(TARGET_LABELS) as [key, label] (key)}
            <option value={key}>{label}</option>
          {/each}
        </select>
      </label>
      <label class="grow">
        パターン/メモ検索
        <input type="text" bind:value={filterText} placeholder="検索" />
      </label>
      <label class="chip">
        <input type="checkbox" bind:checked={showDisabled} />
        無効も表示
      </label>
    </div>
    <div class="bar-right">
      <label class="file-button">
        インポート
        <input type="file" accept=".json,.csv" onchange={onImport} />
      </label>
      <button type="button" onclick={onExport} disabled={rules.length === 0}>
        JSON 書き出し
      </button>
      <button type="button" class="danger" onclick={onClearAll} disabled={rules.length === 0}>
        全削除
      </button>
    </div>
  </div>
  {#if importMessage}
    <div class="info">{importMessage}</div>
  {/if}

  <p class="muted small">
    {formatNumber(visible.length)} / {formatNumber(rules.length)} 件
  </p>

  {#if visible.length === 0}
    <p class="muted">該当するルールはありません。</p>
  {:else}
    <table class="rules">
      <thead>
        <tr>
          <th>有効</th>
          <th>ターゲット</th>
          <th>モード</th>
          <th>パターン</th>
          <th>適用</th>
          <th>ヒット</th>
          <th>追加日</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        {#each visible as r (r.id)}
          <tr class:disabled={!r.enabled}>
            <td>
              <input
                type="checkbox"
                checked={r.enabled}
                onchange={(e) =>
                  updateNgRule(r.id, { enabled: (e.currentTarget as HTMLInputElement).checked })}
              />
            </td>
            <td>{TARGET_LABELS[r.targetType]}</td>
            <td>{MODE_LABELS[r.matchMode]}</td>
            <td class="pattern">
              <code>{r.pattern}</code>
              {#if r.note}<div class="note muted small">{r.note}</div>{/if}
            </td>
            <td class="scopes-cell">
              {#if r.scopeSearch}<span class="tag-pill">検索</span>{/if}
              {#if r.scopeRanking}<span class="tag-pill">ランキング</span>{/if}
              {#if r.scopeComment}<span class="tag-pill">コメ</span>{/if}
            </td>
            <td class="num">{formatNumber(r.hitCount)}</td>
            <td class="muted small">{formatDate(new Date(r.createdAt).toISOString())}</td>
            <td
              ><button type="button" class="danger small" onclick={() => onDelete(r.id)}
                >削除</button
              ></td
            >
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</section>

<style>
  h2 {
    margin-top: 0;
    color: var(--text-heading);
  }
  h3 {
    margin: 0 0 8px;
    font-size: 14px;
    color: var(--text-2);
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: 12px;
  }
  .add {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 12px;
    margin: 16px 0;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: end;
    margin-bottom: 8px;
    flex-wrap: wrap;
  }
  .row label {
    display: flex;
    flex-direction: column;
    font-size: 12px;
    color: var(--text-3);
    gap: 4px;
  }
  .row label.grow {
    flex: 1;
    min-width: 220px;
  }
  input[type='text'],
  select {
    background: var(--input-bg);
    border: 1px solid var(--border-3);
    color: var(--text-heading);
    border-radius: 6px;
    padding: 6px 10px;
    font-size: 13px;
  }
  select {
    background: var(--select-option-bg);
    color: var(--select-option-text);
  }
  .scopes {
    align-items: center;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    border: 1px solid var(--border-3);
    border-radius: 999px;
    background: var(--surface-2);
    font-size: 12px;
    color: var(--text-2);
    cursor: pointer;
    user-select: none;
  }
  button {
    background: var(--surface-hover);
    border: 1px solid var(--menu-border);
    color: var(--text);
    border-radius: 6px;
    padding: 6px 14px;
    font-size: 13px;
    cursor: pointer;
  }
  button:hover {
    background: var(--surface-active);
  }
  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
    margin-left: auto;
  }
  button.primary:hover {
    background: var(--accent-hover);
  }
  button.danger {
    border-color: var(--error-border);
    color: var(--error-text);
  }
  button.danger:hover {
    background: var(--error-bg);
  }
  button.small {
    padding: 2px 8px;
    font-size: 11px;
  }
  .error {
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    color: var(--error-text);
    padding: 6px 10px;
    border-radius: 6px;
    font-size: 12px;
    margin-top: 6px;
  }
  .info {
    background: var(--success-icon-bg);
    border: 1px solid var(--success-icon-border);
    color: var(--success-icon-text);
    padding: 6px 10px;
    border-radius: 6px;
    font-size: 12px;
    margin: 8px 0;
  }
  .bar {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    align-items: end;
    margin: 16px 0 8px;
    flex-wrap: wrap;
  }
  .bar-left,
  .bar-right {
    display: flex;
    gap: 8px;
    align-items: end;
    flex-wrap: wrap;
  }
  .bar-left label {
    display: flex;
    flex-direction: column;
    font-size: 12px;
    color: var(--text-3);
    gap: 4px;
  }
  .bar-left label.grow {
    min-width: 200px;
  }
  .file-button {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 6px 14px;
    background: var(--surface-hover);
    border: 1px solid var(--menu-border);
    color: var(--text);
    border-radius: 6px;
    font-size: 13px;
    cursor: pointer;
  }
  .file-button:hover {
    background: var(--surface-active);
  }
  .file-button input {
    display: none;
  }
  .rules {
    width: 100%;
    border-collapse: collapse;
    margin-top: 8px;
    font-size: 13px;
  }
  .rules th,
  .rules td {
    text-align: left;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
    vertical-align: top;
  }
  .rules th {
    color: var(--text-muted);
    font-weight: 500;
    font-size: 11px;
    text-transform: uppercase;
    background: var(--surface);
    position: sticky;
    top: 0;
  }
  .rules tr.disabled {
    opacity: 0.45;
  }
  .pattern code {
    background: var(--surface-hover);
    padding: 1px 6px;
    border-radius: 4px;
    color: var(--warn-text);
    word-break: break-all;
  }
  .note {
    margin-top: 4px;
  }
  .scopes-cell {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .tag-pill {
    background: var(--tag-locked-bg);
    color: var(--tag-locked-text);
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 11px;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
