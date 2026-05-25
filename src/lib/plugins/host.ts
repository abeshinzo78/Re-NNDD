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
let bootstrapping = false;
let bridgeUnlisten: UnlistenFn | null = null;

type RustEventEnvelope = {
  name: string;
  payload: unknown;
};

/** Rust → JS プラグインイベント橋渡しを idempotent に attach する。
 *  bootstrap 経路だけでなく enable/disable 経路からも呼ばれる: 初回 bootstrap
 *  で listen() が拒否された場合でも、後続の操作タイミングで再試行できる
 *  (Codex #10)。失敗しても throw しない (catch して console.error にとどめる)。 */
async function ensureEventBridge(): Promise<void> {
  if (bridgeUnlisten) return;
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
}

/** 起動時に呼ぶ。多重呼び出しは安全 (idempotent)。
 *  初期リスト取得が失敗した場合は bootstrap 済みフラグを立てず、後続の
 *  retry を許可する (Codex review #4: 一過性エラーでセッション中ずっと
 *  プラグインがロードされなくなる問題の回避)。 */
export async function bootstrapPluginHost(): Promise<void> {
  if (bootstrapped || bootstrapping) return;
  bootstrapping = true;
  try {
    // キルスイッチ。OFF ならばここから先に **絶対に副作用を持たせない**。
    if (!getBool('plugins.enabled')) {
      bootstrapped = true; // キルスイッチ OFF はリトライ不要 (再起動で反映)
      return;
    }

    await ensureEventBridge();

    let installed: Awaited<ReturnType<typeof pluginListInstalled>>;
    try {
      installed = await pluginListInstalled();
    } catch (e) {
      console.error('[plugin] failed to list installed:', e);
      return; // bootstrapped を立てずに return → 次回呼出でリトライ可能
    }

    // 列挙成功 → ここで bootstrap 完了扱い。各プラグイン load は独立 try/catch
    // (失敗しても他プラグインと bootstrap 全体の成功扱いを巻き込まない)。
    bootstrapped = true;
    for (const info of installed) {
      if (!info.enabled) continue;
      try {
        await loader.loadPlugin(info);
      } catch (e) {
        console.error(`[plugin] load threw for ${info.pluginId}:`, e);
      }
    }
  } finally {
    bootstrapping = false;
  }
}

/** プラグインを有効化 (DB + ロード)。
 *  loader.loadPlugin は内部で全例外を catch して "failed" 状態として記録する
 *  だけなので、呼出側の UI が「有効化成功」と誤認しないよう、ここで失敗を
 *  検出して DB enable を rollback したうえで throw する
 *  (Codex review r3297741213)。 */
export async function enablePlugin(info: import('./types').PluginInfo): Promise<void> {
  // キルスイッチが OFF のときは plugin code を 1 byte も実行させない
  // (Codex #2: bootstrap 経路でしかチェックしていなかった抜け穴)。
  if (!getBool('plugins.enabled')) {
    throw new Error(
      'プラグイン機構が無効化されています (設定 → 高度な設定 → プラグイン機構を有効にする)。',
    );
  }
  // 既存セッションでイベント橋が attach できなかった可能性をリトライ。
  await ensureEventBridge();
  await pluginSetEnabled(info.pluginId, true);
  await loader.loadPlugin({ ...info, enabled: true });
  const state = loader.getLoadState(info.pluginId);
  if (state?.state === 'failed') {
    // DB を元に戻す。次回起動時に再度 auto-load されてしまうのを防ぐ。
    try {
      await pluginSetEnabled(info.pluginId, false);
    } catch (e) {
      console.error(`[plugin] enable rollback failed for ${info.pluginId}:`, e);
    }
    throw new Error(`プラグインの読み込みに失敗しました: ${state.error ?? 'unknown error'}`);
  }
}

/** プラグインを無効化 (DB 永続化 → アンロード)。
 *  DB 書き込みが失敗した場合に in-memory だけアンロードされて DB と乖離する
 *  のを防ぐため、DB を先に成功させる (Codex review r3297535052)。
 *  pluginSetEnabled が throw すると unloadPlugin は呼ばれず例外が再送される
 *  ので、呼出側 (settings UI) は plugin の状態が変わっていないことを前提に
 *  リトライできる。 */
export async function disablePlugin(pluginId: string): Promise<void> {
  // 橋が attach できていない可能性をリトライ (Codex #10)。
  await ensureEventBridge();
  await pluginSetEnabled(pluginId, false);
  await loader.unloadPlugin(pluginId);
}

/** テスト用: bootstrap フラグを reset。 */
export function _resetForTests(): void {
  bootstrapped = false;
  bootstrapping = false;
  if (bridgeUnlisten) {
    try {
      bridgeUnlisten();
    } catch {
      /* noop */
    }
    bridgeUnlisten = null;
  }
}
