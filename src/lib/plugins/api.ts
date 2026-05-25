// Tauri command への薄いラッパ (src/lib/api.ts と同じスタイル)。
//
// 個別の plugin_* command をフロント側で呼ぶための関数を提供する。
// ここから直接呼ぶのは host / 設定画面 / loader のみで、プラグイン本体
// は `ctx.invoke()` を経由する (ctx は loader が組み立てる)。

import { invoke } from '@tauri-apps/api/core';
import type { PluginInfo, PluginManifest } from './types';

export function pluginListInstalled(): Promise<PluginInfo[]> {
  return invoke<PluginInfo[]>('plugin_list_installed');
}

export function pluginGetManifest(id: string): Promise<PluginManifest | null> {
  return invoke<PluginManifest | null>('plugin_get_manifest', { id });
}

export function pluginInstallFromZip(path: string, replace: boolean): Promise<PluginInfo> {
  return invoke<PluginInfo>('plugin_install_from_zip', { path, replace });
}

export function pluginUninstall(id: string): Promise<void> {
  return invoke<void>('plugin_uninstall', { id });
}

export function pluginSetEnabled(id: string, enabled: boolean): Promise<void> {
  return invoke<void>('plugin_set_enabled', { id, enabled });
}

export function pluginInvoke(
  pluginId: string,
  action: string,
  payload: unknown,
): Promise<unknown> {
  // Rust 側コマンド引数名は `plugin_id` (snake_case) → Tauri が JS から
  // camelCase で受け取って snake_case に変換するが、明示しておく。
  return invoke<unknown>('plugin_invoke', { pluginId, action, payload });
}
