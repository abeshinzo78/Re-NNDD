<script lang="ts">
  import { onMount } from 'svelte';
  import { getAppVersion } from '$lib/api';

  let version = $state<string | null>(null);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      version = await getAppVersion();
    } catch (e) {
      error = String(e);
    }
  });
</script>

<section>
  <h2>Re:NNDD</h2>
  <p class="muted">ニコニコ動画専用クライアント。</p>

  <div class="cards">
    <a class="card" href="/library">
      <h3>ローカル</h3>
      <p class="muted">DL 済みの動画を一覧 / 再生する。</p>
    </a>
    <a class="card" href="/search">
      <h3>検索</h3>
      <p class="muted">スナップショット検索 API で動画を探す。</p>
    </a>
    <a class="card" href="/downloads">
      <h3>ダウンロード</h3>
      <p class="muted">動画 ID を入れて DL キューに追加する。</p>
    </a>
    <a class="card" href="/history">
      <h3>履歴</h3>
      <p class="muted">過去に再生した動画。</p>
    </a>
  </div>

  <dl class="env">
    <dt>アプリバージョン</dt>
    <dd>{version ?? (error ? `エラー: ${error}` : '取得中…')}</dd>
  </dl>
</section>

<style>
  h2 {
    margin-top: 0;
    color: var(--text-heading);
  }
  .muted {
    color: var(--text-muted);
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 12px;
    margin: 16px 0 24px;
  }
  .card {
    display: block;
    padding: 14px 16px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--text);
    text-decoration: none;
    transition:
      background 0.1s,
      border-color 0.1s;
  }
  .card:hover {
    background: var(--surface-hover);
    border-color: var(--border-strong);
  }
  .card h3 {
    margin: 0 0 6px;
    font-size: 15px;
    color: var(--text-heading);
  }
  .card p {
    margin: 0;
    font-size: 13px;
  }
  .env {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 16px;
    font-size: 13px;
    margin-top: 16px;
  }
  .env dt {
    color: var(--text-muted);
  }
</style>
