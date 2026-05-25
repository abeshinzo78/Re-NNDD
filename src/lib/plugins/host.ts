// プラグインホスト。+layout.svelte の onMount から `bootstrapPluginHost()`
// を 1 度だけ呼ぶ。
//
// 重要な不変条件:
// - キルスイッチ `plugins.enabled` が false なら **何もせず即 return**。
//   この場合の挙動はプラグイン機構導入前と完全に同一 (= console ログ 0、
//   registry 0 件、Tauri listen 0 件)。
// - 各プラグインのロードは独立した try/catch。1 つの失敗で他が止まらない。
// - Rust → JS のプラグインイベント (`nndd:plugin:event`) を 1 本だけ listen し、
//   ペイロードを内部 event bus に再 emit する。

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import * as bus from './eventBus';
import * as loader from './loader';
import { pluginListInstalled, pluginSetEnabled } from './api';
import { getBool } from '$lib/stores/settings.svelte';

let bootstrapped = false;
let bridgeUnlisten: UnlistenFn | null = null;

type RustEventEnvelope = {
  name: string;
  payload: unknown;
};

/** 起動時に呼ぶ。多重呼び出しは安全 (idempotent)。 */
export async function bootstrapPluginHost(): Promise<void> {
  if (bootstrapped) return;
  bootstrapped = true;

  // キルスイッチ。OFF ならばここから先に **絶対に副作用を持たせない**。
  if (!getBool('plugins.enabled')) {
    return;
  }

  // Rust → JS のプラグインイベント橋渡しを 1 本張る。
  try {
    bridgeUnlisten = await listen<RustEventEnvelope>('nndd:plugin:event', (ev) => {
      const env = ev.payload;
      if (env && typeof env === 'object' && typeof env.name === 'string') {
        bus.emit(env.name, env.payload);
      }
    });
  } catch (e) {
    console.error('[plugin] failed to attach event bridge:', e);
  }

  let installed: Awaited<ReturnType<typeof pluginListInstalled>>;
  try {
    installed = await pluginListInstalled();
  } catch (e) {
    console.error('[plugin] failed to list installed:', e);
    return;
  }
  for (const info of installed) {
    if (!info.enabled) continue;
    // 各プラグインは独立 try/catch (loader 内部でも catch しているが二重防御)
    try {
      await loader.loadPlugin(info);
    } catch (e) {
      console.error(`[plugin] load threw for ${info.pluginId}:`, e);
    }
  }
}

/** プラグインを有効化 (DB + ロード)。 */
export async function enablePlugin(
  info: import('./types').PluginInfo,
): Promise<void> {
  await pluginSetEnabled(info.pluginId, true);
  await loader.loadPlugin({ ...info, enabled: true });
}

/** プラグインを無効化 (アンロード + DB)。 */
export async function disablePlugin(pluginId: string): Promise<void> {
  await loader.unloadPlugin(pluginId);
  await pluginSetEnabled(pluginId, false);
}

/** テスト用: bootstrap フラグを reset。 */
export function _resetForTests(): void {
  bootstrapped = false;
  if (bridgeUnlisten) {
    try {
      bridgeUnlisten();
    } catch {
      /* noop */
    }
    bridgeUnlisten = null;
  }
}
