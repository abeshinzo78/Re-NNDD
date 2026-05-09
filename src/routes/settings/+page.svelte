<script lang="ts">
  import { onMount } from 'svelte';
  import {
    clearSessionCookie,
    getAppInfo,
    loginPassword,
    saveSessionCookie,
    sessionCookieStatus,
    type AppInfo,
    type LoginResult,
  } from '$lib/api';
  import {
    SETTING_DEFS,
    get,
    isLoaded,
    loadSettings,
    resetSetting,
    setSetting,
    type SettingDef,
    type SettingKey,
  } from '$lib/stores/settings.svelte';

  // ========= アカウント =========
  let loggedIn = $state(false);
  let email = $state('');
  let password = $state('');
  let cookie = $state('');
  let pending = $state(false);
  let message = $state<{ kind: 'ok' | 'warn' | 'error'; text: string } | null>(null);

  // ========= アプリ情報 =========
  let appInfo = $state<AppInfo | null>(null);

  async function refresh() {
    try {
      loggedIn = await sessionCookieStatus();
    } catch (e) {
      message = { kind: 'error', text: String(e) };
    }
  }

  async function refreshAppInfo() {
    try {
      appInfo = await getAppInfo();
    } catch (e) {
      // non-fatal
      console.warn('app info', e);
    }
  }

  onMount(async () => {
    await loadSettings();
    void refresh();
    void refreshAppInfo();
  });

  function summarizeLogin(result: LoginResult): { kind: 'ok' | 'warn' | 'error'; text: string } {
    switch (result.kind) {
      case 'success':
        return { kind: 'ok', text: 'ログインしました。' };
      case 'mfa':
        return {
          kind: 'warn',
          text:
            '二段階認証が有効なアカウントです。下の「Cookie を直接入力」で user_session を貼り付けてください。',
        };
      case 'invalid_credentials':
        return { kind: 'error', text: 'メールアドレスかパスワードが正しくありません。' };
    }
  }

  async function handleLogin(event: Event) {
    event.preventDefault();
    if (!email || !password) return;
    pending = true;
    message = null;
    try {
      const result = await loginPassword(email, password);
      message = summarizeLogin(result);
      if (result.kind === 'success') password = '';
      await refresh();
    } catch (e) {
      message = { kind: 'error', text: String(e) };
    } finally {
      pending = false;
    }
  }

  async function handleLogout() {
    pending = true;
    try {
      await clearSessionCookie();
      message = { kind: 'ok', text: 'ログアウトしました。' };
      email = '';
      password = '';
      cookie = '';
      await refresh();
    } catch (e) {
      message = { kind: 'error', text: String(e) };
    } finally {
      pending = false;
    }
  }

  async function handleCookieSubmit(event: Event) {
    event.preventDefault();
    if (!cookie.trim()) return;
    pending = true;
    message = null;
    try {
      await saveSessionCookie(cookie.trim());
      message = { kind: 'ok', text: 'Cookie を保存しました。' };
      cookie = '';
      await refresh();
    } catch (e) {
      message = { kind: 'error', text: String(e) };
    } finally {
      pending = false;
    }
  }

  // ========= 設定変更 =========
  async function onSettingChange(key: SettingKey, value: unknown) {
    try {
      await setSetting(key, value);
    } catch (e) {
      message = { kind: 'error', text: `保存失敗: ${e}` };
    }
  }

  async function onSettingReset(key: SettingKey) {
    try {
      await resetSetting(key);
    } catch (e) {
      message = { kind: 'error', text: `リセット失敗: ${e}` };
    }
  }

  function isOverridden(def: SettingDef<unknown>): boolean {
    return get(def.key as SettingKey) !== def.default;
  }

  // セクション分類 + 並び順
  const SECTIONS: { id: string; label: string; description?: string }[] = [
    { id: 'playback', label: '再生', description: '動画プレイヤーの動作' },
    { id: 'comment', label: 'コメント', description: 'コメ表示の初期値' },
    { id: 'download', label: 'ダウンロード', description: 'yt-dlp 経由 DL の挙動' },
    { id: 'library', label: 'ライブラリ', description: 'DL 済み一覧の表示' },
    { id: 'appearance', label: '外観' },
    { id: 'advanced', label: '高度な設定' },
  ];

  function defsForSection(id: string) {
    return [...SETTING_DEFS]
      .filter((d) => d.section === id)
      .sort((a, b) => a.order - b.order);
  }

  function sourceLabel(s: string): string {
    switch (s) {
      case 'bundled': return 'バンドル済';
      case 'sidecar': return 'サイドカー';
      case 'system_path': return 'システム PATH';
      case 'not_found': return '未検出';
      default: return s;
    }
  }
  function formatBytes(b: number): string {
    if (b < 1024) return `${b} B`;
    if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
    if (b < 1024 * 1024 * 1024) return `${(b / 1024 / 1024).toFixed(1)} MB`;
    return `${(b / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }
</script>

<section class="page">
  <h2>設定</h2>

  {#if message}
    <div class="msg {message.kind}">{message.text}</div>
  {/if}

  {#if !isLoaded()}
    <p class="muted">設定を読み込み中…</p>
  {:else}
    {#each SECTIONS as section (section.id)}
      <div class="card">
        <header>
          <h3>{section.label}</h3>
          {#if section.description}<p class="hint">{section.description}</p>{/if}
        </header>
        <div class="settings-list">
          {#each defsForSection(section.id) as def_raw (def_raw.key)}
            {@const def = def_raw as SettingDef<unknown>}
            {@const k = def.key as SettingKey}
            {@const cur = get(k)}
            <div class="setting-row" class:overridden={isOverridden(def)}>
              <div class="setting-label">
                <label for={`set-${def.key}`}>{def.label}</label>
                {#if def.description}<div class="hint">{def.description}</div>{/if}
              </div>
              <div class="setting-control">
                {#if def.kind === 'bool'}
                  <label class="switch">
                    <input
                      id={`set-${def.key}`}
                      type="checkbox"
                      checked={cur as boolean}
                      onchange={(e) =>
                        onSettingChange(k, (e.currentTarget as HTMLInputElement).checked)}
                    />
                    <span class="switch-thumb"></span>
                  </label>
                {:else if def.kind === 'number'}
                  <input
                    id={`set-${def.key}`}
                    type="number"
                    min={def.min}
                    max={def.max}
                    step={def.step}
                    value={cur as number}
                    onchange={(e) => {
                      const v = Number((e.currentTarget as HTMLInputElement).value);
                      if (Number.isFinite(v)) onSettingChange(k, v);
                    }}
                  />
                {:else if def.kind === 'select' && def.options}
                  <select
                    id={`set-${def.key}`}
                    value={String(cur)}
                    onchange={(e) =>
                      onSettingChange(k, (e.currentTarget as HTMLSelectElement).value)}
                  >
                    {#each def.options as opt (opt.value)}
                      <option value={opt.value}>{opt.label}</option>
                    {/each}
                  </select>
                {:else}
                  <input
                    id={`set-${def.key}`}
                    type="text"
                    value={String(cur)}
                    onchange={(e) =>
                      onSettingChange(k, (e.currentTarget as HTMLInputElement).value)}
                  />
                {/if}
                {#if isOverridden(def)}
                  <button
                    type="button"
                    class="reset-btn"
                    title="既定値に戻す"
                    onclick={() => onSettingReset(k)}
                  >↺</button>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </div>
    {/each}
  {/if}

  <!-- アカウント -->
  <div class="card">
    <header>
      <h3>アカウント</h3>
      <p class="hint">ログインしないと公開動画のみ再生可能（プレミアム限定など不可）。</p>
    </header>
    <div class="status">
      <span class="dot" class:on={loggedIn}></span>
      <span class={loggedIn ? 'ok' : 'muted'}>
        {loggedIn ? 'ログイン済み（メモリ内）' : '未ログイン'}
      </span>
      {#if loggedIn}
        <button class="link danger" type="button" onclick={handleLogout} disabled={pending}>
          ログアウト
        </button>
      {/if}
    </div>

    <form onsubmit={handleLogin} class="login-form">
      <label>
        メールアドレス / 電話番号
        <input
          type="email"
          bind:value={email}
          autocomplete="username"
          placeholder="example@example.com"
        />
      </label>
      <label>
        パスワード
        <input
          type="password"
          bind:value={password}
          autocomplete="current-password"
        />
      </label>
      <div class="actions">
        <button type="submit" class="primary" disabled={pending || !email || !password}>
          {pending ? 'ログイン中…' : 'ログイン'}
        </button>
      </div>
    </form>

    <details>
      <summary>2FA / SSO の人は user_session Cookie 直入れ</summary>
      <p class="hint">
        ブラウザでログイン → DevTools → Cookies → <code>user_session</code> の値をコピペ
      </p>
      <form onsubmit={handleCookieSubmit}>
        <input type="password" bind:value={cookie} placeholder="xxxxxx..." autocomplete="off" />
        <div class="actions">
          <button type="submit" class="primary" disabled={pending || !cookie.trim()}>
            保存
          </button>
        </div>
      </form>
    </details>
  </div>

  <!-- アプリ情報 -->
  <div class="card">
    <header>
      <h3>アプリ情報</h3>
      <p class="hint">アプリ化（パッケージ化）に必要な情報、依存ツールの状態など</p>
    </header>
    {#if appInfo}
      <dl class="info-grid">
        <dt>バージョン</dt><dd>{appInfo.version}</dd>
        <dt>識別子</dt><dd><code>{appInfo.identifier}</code></dd>
        <dt>データ保存場所</dt><dd><code>{appInfo.dataDir}</code></dd>
        <dt>動画保存場所</dt><dd><code>{appInfo.videosDir}</code></dd>
        <dt>DB 場所</dt><dd><code>{appInfo.dbPath}</code></dd>
        <dt>ローカルサーバ</dt><dd><code>http://127.0.0.1:{appInfo.localServerPort}/v/</code></dd>
        <dt>ライブラリ動画数</dt><dd>{appInfo.libraryVideoCount} 本 ({formatBytes(appInfo.libraryVideosSizeBytes)})</dd>
        <dt>yt-dlp</dt>
        <dd>
          {#if appInfo.ytdlpAvailable}
            <span class="ok">✓ {appInfo.ytdlpVersion ?? '検出'}</span>
            <span class="src-badge src-{appInfo.ytdlpSource}">{sourceLabel(appInfo.ytdlpSource)}</span>
            <code class="path-tiny">{appInfo.ytdlpPath}</code>
          {:else}
            <span class="error-text">× 未検出 — DL に必要</span>
          {/if}
        </dd>
        <dt>ffmpeg</dt>
        <dd>
          {#if appInfo.ffmpegAvailable}
            <span class="ok">✓ {appInfo.ffmpegVersion ?? '検出'}</span>
            <span class="src-badge src-{appInfo.ffmpegSource}">{sourceLabel(appInfo.ffmpegSource)}</span>
            <code class="path-tiny">{appInfo.ffmpegPath}</code>
          {:else}
            <span class="error-text">× 未検出 — yt-dlp の merge に必要</span>
          {/if}
        </dd>
      </dl>
      <p class="hint">
        <strong>「アプリ単体で完結」を目指す場合:</strong>
        プロジェクト ルートで <code>bash scripts/fetch-binaries.sh</code> を 1 回実行すれば、
        yt-dlp / ffmpeg の単体バイナリが <code>src-tauri/binaries/</code> に展開されて
        バンドルされます。<code>npm run tauri build</code> で生成される .deb / .app / .msi
        にはこのバイナリも入るので、ユーザは別途インストール不要になります。
        <br />開発中で system PATH のものを使いたい場合は
        <code>bash scripts/fetch-binaries.sh --system</code> でシステムバイナリへの
        symlink を張れます。
      </p>
    {:else}
      <p class="muted">取得中…</p>
    {/if}
  </div>
</section>

<style>
  .page { max-width: 900px; }
  h2 { margin-top: 0; }
  h3 { margin: 0 0 4px; font-size: 15px; }
  .muted { color: #9a9a9a; }
  .hint {
    color: #9a9a9a;
    font-size: 12px;
    margin: 0;
    line-height: 1.5;
  }
  .ok { color: #4ade80; }
  .error-text { color: #f87171; }
  .card {
    background: #161616;
    border: 1px solid #1f1f1f;
    border-radius: 8px;
    padding: 14px 16px;
    margin-bottom: 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .card header {
    display: flex;
    flex-direction: column;
    gap: 4px;
    border-bottom: 1px solid #1f1f1f;
    padding-bottom: 10px;
  }
  .settings-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .setting-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 16px;
    align-items: center;
    padding: 8px 0;
    border-bottom: 1px solid #1a1a1a;
  }
  .setting-row:last-child { border-bottom: none; }
  .setting-row.overridden {
    background: linear-gradient(90deg, rgba(37, 99, 235, 0.05), transparent);
  }
  .setting-label label {
    color: #eaeaea;
    font-size: 13px;
    cursor: pointer;
  }
  .setting-control {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .reset-btn {
    background: transparent;
    border: 1px solid #2a2a2a;
    color: #93c5fd;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    cursor: pointer;
    font-size: 12px;
    line-height: 1;
  }
  .reset-btn:hover { background: #1a1a1a; }
  /* number/text inputs */
  input[type='number'],
  input[type='text'],
  input[type='email'],
  input[type='password'],
  select {
    background: #0f0f0f;
    border: 1px solid #2f2f2f;
    color: #f5f5f5;
    border-radius: 6px;
    padding: 6px 10px;
    font-size: 13px;
    min-width: 120px;
  }
  input:focus, select:focus {
    outline: none;
    border-color: #5a5a5a;
  }
  /* toggle switch */
  .switch {
    position: relative;
    display: inline-block;
    width: 44px;
    height: 22px;
  }
  .switch input {
    opacity: 0;
    width: 0;
    height: 0;
  }
  .switch-thumb {
    position: absolute;
    inset: 0;
    background: #2a2a2a;
    border-radius: 22px;
    transition: background 0.15s;
    cursor: pointer;
  }
  .switch-thumb::before {
    content: '';
    position: absolute;
    height: 16px;
    width: 16px;
    left: 3px;
    top: 3px;
    background: #b0b0b0;
    border-radius: 50%;
    transition: transform 0.15s, background 0.15s;
  }
  .switch input:checked + .switch-thumb {
    background: #2563eb;
  }
  .switch input:checked + .switch-thumb::before {
    transform: translateX(22px);
    background: #fff;
  }

  /* status / login / cookie */
  .status {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .dot {
    width: 10px; height: 10px;
    background: #555; border-radius: 999px;
  }
  .dot.on { background: #4ade80; }
  .login-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  label {
    display: flex;
    flex-direction: column;
    font-size: 12px;
    color: #b0b0b0;
    gap: 4px;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .primary {
    background: #2563eb;
    color: white;
    border: none;
    border-radius: 6px;
    padding: 8px 18px;
    font-size: 14px;
    cursor: pointer;
  }
  .primary:disabled { opacity: 0.5; cursor: not-allowed; }
  .link {
    background: transparent;
    border: none;
    color: #6ea8fe;
    cursor: pointer;
    text-decoration: underline;
    font-size: 13px;
    padding: 0;
    margin-left: auto;
  }
  .link.danger { color: #f87171; }
  details > summary {
    cursor: pointer;
    color: #cfcfcf;
    font-size: 13px;
    user-select: none;
    padding: 4px 0;
  }
  .msg {
    border-radius: 6px;
    padding: 10px 12px;
    font-size: 13px;
    margin-bottom: 12px;
  }
  .msg.ok { background: #102d20; border: 1px solid #1e6b48; color: #bbf7d0; }
  .msg.warn { background: #2a2410; border: 1px solid #5a4a1a; color: #fde68a; }
  .msg.error { background: #2a1212; border: 1px solid #5a2222; color: #f5b3b3; }
  .info-grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px 16px;
    margin: 0;
    font-size: 13px;
  }
  .info-grid dt { color: #9a9a9a; }
  .info-grid dd { margin: 0; color: #eaeaea; word-break: break-all; }
  .src-badge {
    display: inline-block;
    margin-left: 6px;
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 500;
  }
  .src-bundled { background: #1a3a26; color: #b3f5b3; border: 1px solid #2a5a3a; }
  .src-sidecar { background: #1a2a44; color: #93c5fd; border: 1px solid #2a3f5a; }
  .src-system_path { background: #2a2418; color: #fde68a; border: 1px solid #5a4a1a; }
  .src-not_found { background: #2a1212; color: #f5b3b3; border: 1px solid #5a2222; }
  .path-tiny {
    display: block;
    font-size: 10px;
    margin-top: 4px;
    color: #9a9a9a;
  }
  code {
    background: #0a0a0a;
    border: 1px solid #1f1f1f;
    border-radius: 3px;
    padding: 0 4px;
    font-size: 12px;
  }
</style>
