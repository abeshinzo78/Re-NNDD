<script lang="ts">
  import { onMount } from 'svelte';
  import {
    clearSessionCookie,
    getAppInfo,
    loginMfa,
    loginPassword,
    saveSessionCookie,
    sessionCookieStatus,
    type AppInfo,
    type LoginResult,
  } from '$lib/api';
  import {
    SETTING_DEFS,
    get,
    getRawSetting,
    isLoaded,
    loadSettings,
    resetRawSetting,
    resetSetting,
    setRawSetting,
    setSetting,
    type SettingDef,
    type SettingKey,
  } from '$lib/stores/settings.svelte';
  import { pluginInstallFromZip, pluginListInstalled, pluginUninstall } from '$lib/plugins/api';
  import { disablePlugin, enablePlugin } from '$lib/plugins/host';
  import { pluginSettingDefs } from '$lib/plugins/registry';
  import type { PluginInfo } from '$lib/plugins/types';
  import { open as openDialog } from '@tauri-apps/plugin-dialog';

  // ========= アカウント =========
  let loggedIn = $state(false);
  let email = $state('');
  let password = $state('');
  let cookie = $state('');
  let pending = $state(false);
  let message = $state<{ kind: 'ok' | 'warn' | 'error'; text: string } | null>(null);

  let mfaRequired = $state(false);
  let mfaSession = $state<string | null>(null);
  let otpCode = $state('');

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
    void refreshPlugins();
  });

  function summarizeLogin(result: LoginResult): { kind: 'ok' | 'warn' | 'error'; text: string } {
    switch (result.kind) {
      case 'success':
        return { kind: 'ok', text: 'ログインしました。' };
      case 'mfa':
        return {
          kind: 'warn',
          text: '二段階認証が有効なアカウントです。下の「Cookie を直接入力」で user_session を貼り付けてください。',
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
    mfaRequired = false;
    try {
      const result = await loginPassword(email, password);
      if (result.kind === 'mfa') {
        mfaRequired = true;
        mfaSession = result.mfaSession ?? null;
        if (result.mfaSession) {
          message = { kind: 'warn', text: '二段階認証コードを入力してください。' };
        } else {
          message = {
            kind: 'warn',
            text: '二段階認証が必要です。ブラウザでログインして user_session を貼り付けてください。',
          };
        }
      } else {
        message = summarizeLogin(result);
        if (result.kind === 'success') password = '';
        await refresh();
      }
    } catch (e) {
      message = { kind: 'error', text: String(e) };
    } finally {
      pending = false;
    }
  }

  async function handleMfaSubmit(event: Event) {
    event.preventDefault();
    if (!otpCode.trim() || !mfaSession) return;
    pending = true;
    message = null;
    try {
      const result = await loginMfa(mfaSession, otpCode.trim());
      if (result.kind === 'mfa') {
        message = { kind: 'error', text: '認証コードが正しくありません。再試行してください。' };
      } else {
        message = summarizeLogin(result);
        if (result.kind === 'success') {
          password = '';
          mfaRequired = false;
          mfaSession = null;
          otpCode = '';
        }
        await refresh();
      }
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
    { id: 'plugins', label: 'プラグイン', description: 'インストール済みプラグインの管理' },
  ];

  // ========= プラグイン管理 =========
  let plugins = $state<PluginInfo[]>([]);
  let pluginsLoaded = $state(false);
  let pluginBusyId = $state<string | null>(null);
  let pluginMessage = $state<{ kind: 'ok' | 'warn' | 'error'; text: string } | null>(null);

  async function refreshPlugins() {
    try {
      plugins = await pluginListInstalled();
      pluginsLoaded = true;
    } catch (e) {
      pluginMessage = { kind: 'error', text: `プラグイン一覧の取得失敗: ${e}` };
    }
  }

  async function handlePluginToggle(info: PluginInfo, next: boolean) {
    pluginBusyId = info.pluginId;
    pluginMessage = null;
    try {
      if (next) {
        await enablePlugin(info);
      } else {
        await disablePlugin(info.pluginId);
      }
      await refreshPlugins();
      pluginMessage = {
        kind: 'ok',
        text: next
          ? `${info.name} を有効化しました (リロードで反映されることがあります)`
          : `${info.name} を無効化しました`,
      };
    } catch (e) {
      pluginMessage = { kind: 'error', text: `操作失敗: ${e}` };
    } finally {
      pluginBusyId = null;
    }
  }

  async function handlePluginUninstall(info: PluginInfo) {
    if (!confirm(`プラグイン「${info.name}」をアンインストールします。よろしいですか?`)) return;
    pluginBusyId = info.pluginId;
    pluginMessage = null;
    try {
      // 無効化してからアンインストール (loaded module を確実に解除)。
      // disable が失敗した場合はここで中止する — 続行すると DB/ファイルだけ
      // 消えて in-memory に loaded module/寄与だけが残る "幽霊" 状態に
      // なる (Codex review r3297638380)。
      if (info.enabled) {
        try {
          await disablePlugin(info.pluginId);
        } catch (e) {
          pluginMessage = {
            kind: 'error',
            text: `無効化に失敗したためアンインストールを中止しました: ${e}`,
          };
          return;
        }
      }
      await pluginUninstall(info.pluginId);
      await refreshPlugins();
      pluginMessage = { kind: 'ok', text: `${info.name} をアンインストールしました` };
    } catch (e) {
      pluginMessage = { kind: 'error', text: `アンインストール失敗: ${e}` };
    } finally {
      pluginBusyId = null;
    }
  }

  async function handlePluginInstallZip() {
    pluginMessage = null;
    try {
      const selected = await openDialog({
        multiple: false,
        directory: false,
        filters: [{ name: 'Plugin ZIP', extensions: ['zip'] }],
      });
      if (!selected || Array.isArray(selected)) return;
      const installed = await tryInstall(selected, false);
      pluginMessage = {
        kind: 'ok',
        text: `${installed.name} (${installed.version}) をインストールしました。設定の「有効化」をオンにすると次回ロードされます。`,
      };
      await refreshPlugins();
    } catch (e) {
      const msg = String(e);
      if (msg.includes('already installed')) {
        // 同 ID が既に存在 → 上書き確認
        if (confirm('同じ ID のプラグインが既にインストールされています。上書きしますか?')) {
          try {
            const selected = await openDialog({
              multiple: false,
              directory: false,
              filters: [{ name: 'Plugin ZIP', extensions: ['zip'] }],
            });
            if (!selected || Array.isArray(selected)) return;
            const installed = await tryInstall(selected, true);
            pluginMessage = {
              kind: 'ok',
              text: `${installed.name} を上書きインストールしました`,
            };
            await refreshPlugins();
          } catch (e2) {
            pluginMessage = { kind: 'error', text: `上書き失敗: ${e2}` };
          }
        }
      } else {
        pluginMessage = { kind: 'error', text: `インストール失敗: ${e}` };
      }
    }
  }

  async function tryInstall(path: string, replace: boolean): Promise<PluginInfo> {
    return await pluginInstallFromZip(path, replace);
  }

  // ========= プラグイン設定 (raw key 経由) =========
  // 値はプラグイン側 ctx.settings.get/set と同じ `settings` テーブルに保存される。
  // ホスト UI からの編集は permission チェック不要 (ユーザ操作起点) のため、
  // dispatcher 経由ではなく raw API で直接保存する。
  async function handlePluginSettingChange(key: string, value: string) {
    try {
      await setRawSetting(key, value);
    } catch (e) {
      pluginMessage = { kind: 'error', text: `プラグイン設定の保存失敗: ${e}` };
    }
  }
  async function handlePluginSettingReset(key: string) {
    try {
      await resetRawSetting(key);
    } catch (e) {
      pluginMessage = { kind: 'error', text: `プラグイン設定のリセット失敗: ${e}` };
    }
  }

  function defsForSection(id: string) {
    return [...SETTING_DEFS].filter((d) => d.section === id).sort((a, b) => a.order - b.order);
  }

  function sourceLabel(s: string): string {
    switch (s) {
      case 'bundled':
        return 'バンドル済';
      case 'sidecar':
        return 'サイドカー';
      case 'system_path':
        return 'システム PATH';
      case 'not_found':
        return '未検出';
      default:
        return s;
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
        {#if section.id === 'plugins'}
          <!-- ===== プラグイン管理 (custom render) ===== -->
          {#if pluginMessage}
            <div class="msg {pluginMessage.kind}" style="margin-bottom:12px">
              {pluginMessage.text}
            </div>
          {/if}
          <div class="plugin-toolbar">
            <button type="button" class="primary" onclick={handlePluginInstallZip}>
              ZIP からインストール
            </button>
            <button type="button" class="link" onclick={refreshPlugins}>再読み込み</button>
          </div>

          {#if !pluginsLoaded}
            <p class="muted">プラグイン一覧を読み込み中…</p>
          {:else if plugins.length === 0}
            <p class="muted">インストール済みプラグインはありません。</p>
          {:else}
            <ul class="plugin-list">
              {#each plugins as p (p.pluginId)}
                <li class="plugin-row" class:enabled={p.enabled}>
                  <div class="plugin-main">
                    <div class="plugin-name">
                      {p.name}
                      <span class="plugin-version">v{p.version}</span>
                    </div>
                    <div class="plugin-id"><code>{p.pluginId}</code></div>
                    {#if p.description}<div class="plugin-desc hint">{p.description}</div>{/if}
                    {#if p.permissions.length > 0}
                      <div class="plugin-perms">
                        {#each p.permissions as perm (perm)}
                          <span class="perm-chip">{perm}</span>
                        {/each}
                      </div>
                    {/if}
                    {#if p.author || p.homepage}
                      <div class="plugin-meta hint">
                        {#if p.author}<span>{p.author}</span>{/if}
                        {#if p.homepage}
                          <a href={p.homepage} target="_blank" rel="noreferrer noopener"
                            >{p.homepage}</a
                          >
                        {/if}
                      </div>
                    {/if}
                  </div>
                  <div class="plugin-actions">
                    <label class="switch" title={p.enabled ? '無効化' : '有効化'}>
                      <input
                        type="checkbox"
                        checked={p.enabled}
                        disabled={pluginBusyId === p.pluginId}
                        onchange={(e) =>
                          handlePluginToggle(p, (e.currentTarget as HTMLInputElement).checked)}
                      />
                      <span class="switch-thumb"></span>
                    </label>
                    <button
                      type="button"
                      class="link danger"
                      disabled={pluginBusyId === p.pluginId}
                      onclick={() => handlePluginUninstall(p)}
                    >
                      アンインストール
                    </button>
                  </div>
                </li>
              {/each}
            </ul>
          {/if}

          <!-- プラグインが register した設定項目 (有効プラグインのみ表示される)。
               bool/number/select/text 各 kind に対応する編集 UI を提供し、
               `plugin:<id>:` 名前空間の settings テーブルに直接永続化する。
               プラグイン側で `ctx.settings.get(key)` で読むと同じ値が読める。 -->
          {#if pluginSettingDefs().length > 0}
            <div class="plugin-settings" style="margin-top:16px">
              <h4 class="hint" style="margin:0 0 8px">プラグインが追加した設定</h4>
              <div class="settings-list">
                {#each pluginSettingDefs() as pdef (pdef.key)}
                  {@const raw = getRawSetting(pdef.key)}
                  {@const hasValue = raw !== undefined}
                  <div class="setting-row" class:overridden={hasValue}>
                    <div class="setting-label">
                      <label for={`pset-${pdef.key}`}>{pdef.label}</label>
                      {#if pdef.description}<div class="hint">{pdef.description}</div>{/if}
                      <div class="hint"><code>{pdef.key}</code></div>
                    </div>
                    <div class="setting-control">
                      {#if pdef.kind === 'bool'}
                        {@const cur = hasValue ? raw === 'true' : !!pdef.default}
                        <label class="switch">
                          <input
                            id={`pset-${pdef.key}`}
                            type="checkbox"
                            checked={cur}
                            onchange={(e) =>
                              handlePluginSettingChange(
                                pdef.key,
                                (e.currentTarget as HTMLInputElement).checked ? 'true' : 'false',
                              )}
                          />
                          <span class="switch-thumb"></span>
                        </label>
                      {:else if pdef.kind === 'number'}
                        {@const curRaw = hasValue ? raw : String(pdef.default ?? 0)}
                        <input
                          id={`pset-${pdef.key}`}
                          type="number"
                          min={pdef.min}
                          max={pdef.max}
                          step={pdef.step}
                          value={curRaw}
                          onchange={(e) => {
                            const v = Number((e.currentTarget as HTMLInputElement).value);
                            if (Number.isFinite(v)) handlePluginSettingChange(pdef.key, String(v));
                          }}
                        />
                      {:else if pdef.kind === 'select' && pdef.options}
                        {@const curStr = hasValue ? raw! : String(pdef.default ?? '')}
                        <select
                          id={`pset-${pdef.key}`}
                          value={curStr}
                          onchange={(e) =>
                            handlePluginSettingChange(
                              pdef.key,
                              (e.currentTarget as HTMLSelectElement).value,
                            )}
                        >
                          {#each pdef.options as opt (opt.value)}
                            <option value={opt.value}>{opt.label}</option>
                          {/each}
                        </select>
                      {:else}
                        {@const curStr = hasValue ? raw! : String(pdef.default ?? '')}
                        <input
                          id={`pset-${pdef.key}`}
                          type="text"
                          value={curStr}
                          onchange={(e) =>
                            handlePluginSettingChange(
                              pdef.key,
                              (e.currentTarget as HTMLInputElement).value,
                            )}
                        />
                      {/if}
                      {#if hasValue}
                        <button
                          type="button"
                          class="reset-btn"
                          title="既定値に戻す"
                          onclick={() => handlePluginSettingReset(pdef.key)}>↺</button
                        >
                      {/if}
                    </div>
                  </div>
                {/each}
              </div>
            </div>
          {/if}

          <p class="hint" style="margin-top:16px">
            <strong>注意:</strong> プラグインはアプリと同じ権限でレンダラ realm 内で動作します。 信頼できる提供元のプラグインのみインストールしてください。
            プラグイン機構を完全に停止したい場合は「高度な設定」→「プラグイン機構を有効にする」を OFF
            にしてください (再起動で反映)。
          </p>
        {:else}
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
                      onclick={() => onSettingReset(k)}>↺</button
                    >
                  {/if}
                </div>
              </div>
            {/each}
          </div>
        {/if}
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
        <input type="password" bind:value={password} autocomplete="current-password" />
      </label>
      <div class="actions">
        <button type="submit" class="primary" disabled={pending || !email || !password}>
          {pending ? 'ログイン中…' : 'ログイン'}
        </button>
      </div>
    </form>

    {#if mfaRequired}
      {#if mfaSession}
        <form onsubmit={handleMfaSubmit} class="mfa-form">
          <label>
            二段階認証コード
            <input
              type="text"
              inputmode="numeric"
              maxlength="6"
              bind:value={otpCode}
              autocomplete="one-time-code"
              placeholder="000000"
            />
          </label>
          <div class="actions">
            <button type="submit" class="primary" disabled={pending || otpCode.length < 6}>
              {pending ? '確認中…' : '認証'}
            </button>
          </div>
        </form>
      {:else}
        <p
          class="hint"
          style="padding:12px;border:1px solid var(--theme-border-strong);border-radius:8px"
        >
          MFA セッションを取得できませんでした。下の「Cookie 直入れ」から <code>user_session</code> を貼り付けてください。
        </p>
      {/if}
    {/if}

    <details>
      <summary>2FA / SSO の人は user_session Cookie 直入れ</summary>
      <p class="hint">
        ブラウザでログイン → DevTools → Cookies → <code>user_session</code> の値をコピペ
      </p>
      <form onsubmit={handleCookieSubmit}>
        <input type="password" bind:value={cookie} placeholder="xxxxxx..." autocomplete="off" />
        <div class="actions">
          <button type="submit" class="primary" disabled={pending || !cookie.trim()}> 保存 </button>
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
        <dt>バージョン</dt>
        <dd>{appInfo.version}</dd>
        <dt>識別子</dt>
        <dd><code>{appInfo.identifier}</code></dd>
        <dt>データ保存場所</dt>
        <dd><code>{appInfo.dataDir}</code></dd>
        <dt>動画保存場所</dt>
        <dd><code>{appInfo.videosDir}</code></dd>
        <dt>DB 場所</dt>
        <dd><code>{appInfo.dbPath}</code></dd>
        <dt>ローカルサーバ</dt>
        <dd><code>http://127.0.0.1:{appInfo.localServerPort}/v/</code></dd>
        <dt>ライブラリ動画数</dt>
        <dd>{appInfo.libraryVideoCount} 本 ({formatBytes(appInfo.libraryVideosSizeBytes)})</dd>
        <dt>yt-dlp</dt>
        <dd>
          {#if appInfo.ytdlpAvailable}
            <span class="ok">✓ {appInfo.ytdlpVersion ?? '検出'}</span>
            <span class="src-badge src-{appInfo.ytdlpSource}"
              >{sourceLabel(appInfo.ytdlpSource)}</span
            >
            <code class="path-tiny">{appInfo.ytdlpPath}</code>
          {:else}
            <span class="error-text">× 未検出 — DL に必要</span>
          {/if}
        </dd>
        <dt>ffmpeg</dt>
        <dd>
          {#if appInfo.ffmpegAvailable}
            <span class="ok">✓ {appInfo.ffmpegVersion ?? '検出'}</span>
            <span class="src-badge src-{appInfo.ffmpegSource}"
              >{sourceLabel(appInfo.ffmpegSource)}</span
            >
            <code class="path-tiny">{appInfo.ffmpegPath}</code>
          {:else}
            <span class="error-text">× 未検出 — yt-dlp の merge に必要</span>
          {/if}
        </dd>
      </dl>
      <p class="hint">
        <strong>「アプリ単体で完結」を目指す場合:</strong>
        プロジェクト ルートで <code>bash scripts/fetch-binaries.sh</code> を 1 回実行すれば、 yt-dlp
        / ffmpeg の単体バイナリが <code>src-tauri/binaries/</code> に展開されて バンドルされます。<code
          >npm run tauri build</code
        >
        で生成される .deb / .app / .msi にはこのバイナリも入るので、ユーザは別途インストール不要になります。
        <br />開発中で system PATH のものを使いたい場合は
        <code>bash scripts/fetch-binaries.sh --system</code> でシステムバイナリへの symlink を張れます。
      </p>
    {:else}
      <p class="muted">取得中…</p>
    {/if}
  </div>
</section>

<style>
  .page {
    max-width: 900px;
  }
  h2 {
    margin-top: 0;
  }
  h3 {
    margin: 0 0 4px;
    font-size: 15px;
  }
  .muted {
    color: var(--theme-text-muted);
  }
  .hint {
    color: var(--theme-text-muted);
    font-size: 12px;
    margin: 0;
    line-height: 1.5;
  }
  .ok {
    color: var(--theme-success-strong);
  }
  .error-text {
    color: var(--theme-danger-text);
  }
  .card {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
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
    border-bottom: 1px solid var(--theme-border);
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
    border-bottom: 1px solid var(--theme-surface-3);
  }
  .setting-row:last-child {
    border-bottom: none;
  }
  .setting-row.overridden {
    background: linear-gradient(90deg, rgba(37, 99, 235, 0.05), transparent);
  }
  .setting-label label {
    color: var(--theme-text);
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
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-accent-soft);
    width: 28px;
    height: 28px;
    border-radius: 50%;
    cursor: pointer;
    font-size: 12px;
    line-height: 1;
  }
  .reset-btn:hover {
    background: var(--theme-surface-3);
  }
  /* number/text inputs */
  input[type='number'],
  input[type='text'],
  input[type='email'],
  input[type='password'],
  select {
    background: var(--theme-input-bg);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    border-radius: 6px;
    padding: 6px 10px;
    font-size: 13px;
    min-width: 120px;
  }
  input:focus,
  select:focus {
    outline: none;
    border-color: var(--theme-border-focus);
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
    background: var(--theme-border-strong);
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
    background: var(--theme-text-soft);
    border-radius: 50%;
    transition:
      transform 0.15s,
      background 0.15s;
  }
  .switch input:checked + .switch-thumb {
    background: var(--theme-accent);
  }
  .switch input:checked + .switch-thumb::before {
    transform: translateX(22px);
    background: var(--theme-surface-2);
  }

  /* status / login / cookie */
  .status {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .dot {
    width: 10px;
    height: 10px;
    background: var(--theme-text-faint);
    border-radius: 999px;
  }
  .dot.on {
    background: var(--theme-success-strong);
  }
  .login-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .mfa-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border: 1px solid var(--theme-border-strong);
    border-radius: 8px;
    background: var(--theme-surface-2);
    margin-bottom: 8px;
  }
  label {
    display: flex;
    flex-direction: column;
    font-size: 12px;
    color: var(--theme-text-soft);
    gap: 4px;
  }
  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .primary {
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    border: none;
    border-radius: 6px;
    padding: 8px 18px;
    font-size: 14px;
    cursor: pointer;
  }
  .primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .link {
    background: transparent;
    border: none;
    color: var(--theme-accent-soft);
    cursor: pointer;
    text-decoration: underline;
    font-size: 13px;
    padding: 0;
    margin-left: auto;
  }
  .link.danger {
    color: var(--theme-danger-text);
  }
  details > summary {
    cursor: pointer;
    color: var(--theme-text-soft);
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
  .msg.ok {
    background: var(--theme-success-bg-2);
    border: 1px solid var(--theme-success-border);
    color: var(--theme-success-text);
  }
  .msg.warn {
    background: var(--theme-warning-bg);
    border: 1px solid var(--theme-warning-border);
    color: var(--theme-warning-text);
  }
  .msg.error {
    background: var(--theme-danger-bg);
    border: 1px solid var(--theme-danger-border);
    color: var(--theme-danger-text);
  }
  .info-grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px 16px;
    margin: 0;
    font-size: 13px;
  }
  .info-grid dt {
    color: var(--theme-text-muted);
  }
  .info-grid dd {
    margin: 0;
    color: var(--theme-text);
    word-break: break-all;
  }
  .src-badge {
    display: inline-block;
    margin-left: 6px;
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 500;
  }
  .src-bundled {
    background: var(--theme-success-bg);
    color: var(--theme-success-text);
    border: 1px solid var(--theme-success-border);
  }
  .src-sidecar {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
    border: 1px solid var(--theme-accent-border);
  }
  .src-system_path {
    background: var(--theme-warning-bg);
    color: var(--theme-warning-text);
    border: 1px solid var(--theme-warning-border);
  }
  .src-not_found {
    background: var(--theme-danger-bg);
    color: var(--theme-danger-text);
    border: 1px solid var(--theme-danger-border);
  }
  .path-tiny {
    display: block;
    font-size: 10px;
    margin-top: 4px;
    color: var(--theme-text-muted);
  }
  code {
    background: var(--theme-bg);
    border: 1px solid var(--theme-border);
    border-radius: 3px;
    padding: 0 4px;
    font-size: 12px;
  }
  /* ===== プラグイン管理 (section.id === 'plugins') ===== */
  .plugin-toolbar {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-bottom: 12px;
  }
  .plugin-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .plugin-row {
    display: flex;
    gap: 12px;
    padding: 12px;
    border: 1px solid var(--theme-border);
    border-radius: 8px;
    background: var(--theme-surface-3);
  }
  .plugin-row.enabled {
    border-color: var(--theme-success-border);
  }
  .plugin-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .plugin-name {
    font-weight: 600;
    color: var(--theme-text);
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .plugin-version {
    font-size: 12px;
    color: var(--theme-text-muted);
    font-weight: 400;
  }
  .plugin-id {
    font-size: 12px;
    color: var(--theme-text-muted);
  }
  .plugin-desc {
    font-size: 13px;
  }
  .plugin-perms {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
  }
  .perm-chip {
    background: var(--theme-chip-bg);
    color: var(--theme-chip-text);
    border-radius: 999px;
    font-size: 11px;
    padding: 2px 8px;
  }
  .plugin-meta {
    display: flex;
    gap: 8px;
    font-size: 12px;
  }
  .plugin-actions {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
    flex-shrink: 0;
  }
</style>
