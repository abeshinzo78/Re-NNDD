// プラグインが寄与した UI/設定アイテムのレジストリ。
//
// Svelte 5 の `$state` で reactive に保持し、寄与は plugin id でキー
// 付きに登録する。disable / uninstall 時に該当 plugin の寄与を atomic に
// 取り除けるよう、Map<pluginId, items[]> 形式で保持する。
//
// `*Entries()` 系の getter は **空配列** を初期値とするので、プラグイン
// 寄与が 0 件の状態は「プラグイン機構導入前」と完全一致する。

import { SvelteMap } from 'svelte/reactivity';
import type {
  PluginItemAction,
  PluginNavEntry,
  PluginPlayerAction,
  PluginSettingDef,
} from './types';

// SvelteMap は set/delete/clear をリアクティブに追跡する。プレーン Map に
// $state を当てても操作は追跡されないため、ここでは SvelteMap を使う
// (svelte/reactivity)。
type Bucket<T> = SvelteMap<string, T[]>;

const navByPlugin: Bucket<PluginNavEntry> = new SvelteMap();
const settingsByPlugin: Bucket<PluginSettingDef> = new SvelteMap();
const itemActionsByPlugin: Bucket<PluginItemAction> = new SvelteMap();
const playerActionsByPlugin: Bucket<PluginPlayerAction> = new SvelteMap();

function flatten<T>(b: Bucket<T>): T[] {
  const out: T[] = [];
  for (const arr of b.values()) out.push(...arr);
  return out;
}

function addTo<T>(b: Bucket<T>, pluginId: string, item: T): void {
  // SvelteMap の値変更を確実に通知するため、配列をコピーして set し直す。
  const prev = b.get(pluginId) ?? [];
  b.set(pluginId, [...prev, item]);
}

export function addNav(pluginId: string, entry: PluginNavEntry): void {
  addTo(navByPlugin, pluginId, entry);
}
export function addSetting(pluginId: string, def: PluginSettingDef): void {
  addTo(settingsByPlugin, pluginId, def);
}
export function addItemAction(pluginId: string, action: PluginItemAction): void {
  addTo(itemActionsByPlugin, pluginId, action);
}
export function addPlayerAction(pluginId: string, action: PluginPlayerAction): void {
  addTo(playerActionsByPlugin, pluginId, action);
}

/** 1 プラグインの寄与をまるごと取り除く (disable / uninstall 時)。 */
export function removeAllByPlugin(pluginId: string): void {
  navByPlugin.delete(pluginId);
  settingsByPlugin.delete(pluginId);
  itemActionsByPlugin.delete(pluginId);
  playerActionsByPlugin.delete(pluginId);
}

/** 全寄与をクリア (テスト用 / kill switch OFF 時)。 */
export function clearAll(): void {
  navByPlugin.clear();
  settingsByPlugin.clear();
  itemActionsByPlugin.clear();
  playerActionsByPlugin.clear();
}

// ---- 一覧 getter (UI 側はこれらを呼んでマージ表示する) ----

export function pluginNavEntries(): PluginNavEntry[] {
  return flatten(navByPlugin);
}
export function pluginSettingDefs(): PluginSettingDef[] {
  return flatten(settingsByPlugin);
}
export function pluginItemActions(): PluginItemAction[] {
  return flatten(itemActionsByPlugin);
}
export function pluginPlayerActions(): PluginPlayerAction[] {
  return flatten(playerActionsByPlugin);
}

/** テスト用: 寄与件数。 */
export function _counts(): {
  nav: number;
  settings: number;
  items: number;
  player: number;
} {
  return {
    nav: pluginNavEntries().length,
    settings: pluginSettingDefs().length,
    items: pluginItemActions().length,
    player: pluginPlayerActions().length,
  };
}
