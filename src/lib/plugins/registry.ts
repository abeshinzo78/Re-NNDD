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
  PluginCommand,
  PluginItemAction,
  PluginNavEntry,
  PluginPageRenderer,
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
const commandsByPlugin: Bucket<PluginCommand> = new SvelteMap();

/** ページレンダラ。subpath → render の入れ子マップ。SvelteKit ルートの
 *  `/plugin/[id]/[...path]/+page.svelte` がこの map から renderer を引く。
 *  プラグインが同 subpath を 2 回 register したら後勝ち (Codex review で
 *  指摘が来たら配列にしてフィルタ表示するなど検討)。 */
const pagesByPlugin: SvelteMap<string, SvelteMap<string, PluginPageRenderer>> = new SvelteMap();

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
export function addCommand(pluginId: string, cmd: PluginCommand): void {
  // 同 id の重複は後勝ちで置換 (プラグイン側で hot-reload するときの再登録対応)。
  const prev = commandsByPlugin.get(pluginId) ?? [];
  const filtered = prev.filter((c) => c.id !== cmd.id);
  commandsByPlugin.set(pluginId, [...filtered, cmd]);
}
export function addPage(pluginId: string, subpath: string, render: PluginPageRenderer): void {
  // subpath は normalize: 先頭スラッシュを剥がす (`/dashboard` → `dashboard`)。
  // SvelteKit の `[...path]` rest セグメントが渡してくる値とキーを揃えるため。
  const key = subpath.replace(/^\/+/, '');
  let inner = pagesByPlugin.get(pluginId);
  if (!inner) {
    inner = new SvelteMap();
    pagesByPlugin.set(pluginId, inner);
  }
  inner.set(key, render);
}

/** 1 プラグインの寄与をまるごと取り除く (disable / uninstall 時)。 */
export function removeAllByPlugin(pluginId: string): void {
  navByPlugin.delete(pluginId);
  settingsByPlugin.delete(pluginId);
  itemActionsByPlugin.delete(pluginId);
  playerActionsByPlugin.delete(pluginId);
  commandsByPlugin.delete(pluginId);
  pagesByPlugin.delete(pluginId);
}

/** 全寄与をクリア (テスト用 / kill switch OFF 時)。 */
export function clearAll(): void {
  navByPlugin.clear();
  settingsByPlugin.clear();
  itemActionsByPlugin.clear();
  playerActionsByPlugin.clear();
  commandsByPlugin.clear();
  pagesByPlugin.clear();
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
export function pluginCommands(): (PluginCommand & { pluginId: string })[] {
  // pluginId を attach して返す (重複 ID 解消やテレメトリ用)。
  const out: (PluginCommand & { pluginId: string })[] = [];
  for (const [pid, arr] of commandsByPlugin.entries()) {
    for (const c of arr) out.push({ ...c, pluginId: pid });
  }
  return out;
}
/** プラグイン専用ページの renderer を引く。見つからなければ null。
 *  SvelteKit ルート `/plugin/[id]/[...path]` から呼ばれる。 */
export function pluginPage(pluginId: string, subpath: string): PluginPageRenderer | null {
  const inner = pagesByPlugin.get(pluginId);
  if (!inner) return null;
  const key = subpath.replace(/^\/+/, '');
  // 完全一致を優先。ヒットしなければ `''` (= ルート) も試す。
  return inner.get(key) ?? inner.get('') ?? null;
}

/** テスト用: 寄与件数。 */
export function _counts(): {
  nav: number;
  settings: number;
  items: number;
  player: number;
  commands: number;
  pages: number;
} {
  let pageCount = 0;
  for (const m of pagesByPlugin.values()) pageCount += m.size;
  return {
    nav: pluginNavEntries().length,
    settings: pluginSettingDefs().length,
    items: pluginItemActions().length,
    player: pluginPlayerActions().length,
    commands: pluginCommands().length,
    pages: pageCount,
  };
}
