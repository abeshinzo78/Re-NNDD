// 単一プラグインのロード/アンロード。
//
// プラグインは `$APPDATA/plugins/<id>/<entry>` に置かれた ES module。
// Tauri 2 の `convertFileSrc` で asset:// URL に変換し、`import(/* @vite-ignore */ url)`
// で動的読み込みする (アセットプロトコルは tauri.conf.json で $APPDATA/**
// にスコープ済み)。
//
// ロード失敗 / activate 中の throw は **全部 catch して console.error にとどめる**
// — 1 プラグインの失敗で他プラグインやホストが死なないことを最優先する。

import { convertFileSrc } from '@tauri-apps/api/core';
import * as bus from './eventBus';
import * as registry from './registry';
import { pluginInvoke } from './api';
import type {
  PluginContext,
  PluginInfo,
  PluginItemAction,
  PluginManifest,
  PluginModule,
  PluginNavEntry,
  PluginPlayerAction,
  PluginSettingDef,
} from './types';

/** 一度ロードしたプラグインの deactivate を呼び出すためのフック保持。 */
const loadedModules = new Map<string, PluginModule>();

/** plugin id ごとのロード状態 (UI の "失敗" 表示用に保持)。 */
export type LoadState = 'loaded' | 'failed';
const loadStates = new Map<string, { state: LoadState; error?: string }>();

export function getLoadState(pluginId: string): { state: LoadState; error?: string } | undefined {
  return loadStates.get(pluginId);
}

function buildContext(info: PluginInfo): PluginContext {
  const manifest: PluginManifest = {
    id: info.pluginId,
    name: info.name,
    version: info.version,
    entry: info.entry,
    description: info.description ?? null,
    author: info.author ?? null,
    homepage: info.homepage ?? null,
    permissions: info.permissions,
  };
  const pid = info.pluginId;
  const logTag = `[plugin:${pid}]`;
  return {
    manifest,
    events: {
      on(name: string, handler: (payload: never) => void) {
        return bus.on(pid, name, handler as (p: unknown) => void);
      },
      emit(name: string, payload: unknown) {
        bus.emit(name, payload);
      },
    },
    settings: {
      register(def: PluginSettingDef) {
        // key prefix の防御 (Rust 側でも enforce されるが UX のため事前に弾く)
        const prefix = `plugin.${pid}.`;
        if (!def.key.startsWith(prefix)) {
          console.warn(logTag, 'settings.register rejected: key must start with', prefix);
          return;
        }
        registry.addSetting(pid, def);
      },
      get(key: string) {
        return pluginInvoke(pid, 'settings.get', { key });
      },
      set(key: string, value: string) {
        return pluginInvoke(pid, 'settings.set', { key, value }) as Promise<void>;
      },
    },
    nav: {
      addPage(entry: PluginNavEntry) {
        registry.addNav(pid, entry);
      },
    },
    items: {
      addAction(action: PluginItemAction) {
        registry.addItemAction(pid, action);
      },
    },
    player: {
      addAction(action: PluginPlayerAction) {
        registry.addPlayerAction(pid, action);
      },
    },
    invoke(action: string, payload?: unknown) {
      return pluginInvoke(pid, action, payload ?? null);
    },
    log: {
      info: (...args: unknown[]) => console.info(logTag, ...args),
      warn: (...args: unknown[]) => console.warn(logTag, ...args),
      error: (...args: unknown[]) => console.error(logTag, ...args),
    },
  };
}

/** プラグインを動的 import して `activate()` を呼ぶ。失敗しても throw しない。 */
export async function loadPlugin(info: PluginInfo): Promise<void> {
  const pid = info.pluginId;
  try {
    const assetUrl = convertFileSrc(info.entryAbsPath);
    // Vite の解析を回避 (実行時に決まる URL を import するため)
    const mod = (await import(/* @vite-ignore */ assetUrl)) as PluginModule;
    loadedModules.set(pid, mod);
    if (typeof mod.activate === 'function') {
      await mod.activate(buildContext(info));
    }
    loadStates.set(pid, { state: 'loaded' });
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    loadStates.set(pid, { state: 'failed', error: msg });
    console.error(`[plugin] failed to load ${pid}:`, e);
  }
}

/** プラグインを停止する。registry/bus からの解除 + (あれば) deactivate 呼び出し。 */
export async function unloadPlugin(pluginId: string): Promise<void> {
  const mod = loadedModules.get(pluginId);
  loadedModules.delete(pluginId);
  loadStates.delete(pluginId);
  registry.removeAllByPlugin(pluginId);
  bus.offAllByOwner(pluginId);
  if (mod && typeof mod.deactivate === 'function') {
    try {
      await mod.deactivate();
    } catch (e) {
      console.error(`[plugin] deactivate of ${pluginId} threw:`, e);
    }
  }
}

/** テスト用: 内部状態を全クリア。 */
export function _resetForTests(): void {
  loadedModules.clear();
  loadStates.clear();
}
