// コマンドパレットのオープン/クローズと組込みコマンドの管理。
//
// Ctrl/⌘+K で開き、組込みコマンド + プラグインコマンド (`ctx.commands.register`)
// を fuzzy 検索で実行する。組込みコマンドはアプリ本体のナビゲーションや
// テーマ切替など (キー = id で一意)。

import { goto } from '$app/navigation';
import { getStr, setSetting } from '$lib/stores/settings.svelte';
import type { PluginCommand } from '$lib/plugins/types';

export type BuiltinCommand = {
  id: string;
  title: string;
  hint?: string;
  keywords?: string[];
  handler: () => void | Promise<void>;
};

let open = $state(false);

export function isPaletteOpen(): boolean {
  return open;
}

export function openPalette(): void {
  open = true;
}

export function closePalette(): void {
  open = false;
}

export function togglePalette(): void {
  open = !open;
}

// ---- 組込みコマンド (アプリ本体機能) ----
//
// プラグイン非導入時でも palette を使えるよう、組込みだけで実用範囲を満たす。
// 内部 ID は `app.*` で名前空間化し、プラグインの `ctx.commands.register`
// と衝突しないようにする (プラグインの id は plugin が自由に決めるが、
// `app.*` を含むものは登録できないよう必要なら filter する; 現状は柔らか
// めにする)。

export const BUILTIN_COMMANDS: BuiltinCommand[] = [
  {
    id: 'app.nav.home',
    title: 'ホームへ移動',
    keywords: ['home', 'top'],
    handler: () => goto('/'),
  },
  {
    id: 'app.nav.library',
    title: 'ローカルライブラリへ移動',
    keywords: ['library', 'local'],
    handler: () => goto('/library'),
  },
  {
    id: 'app.nav.ranking',
    title: 'ランキングへ移動',
    keywords: ['ranking'],
    handler: () => goto('/ranking'),
  },
  {
    id: 'app.nav.search',
    title: '検索へ移動',
    keywords: ['search'],
    handler: () => goto('/search'),
  },
  {
    id: 'app.nav.playlists',
    title: 'プレイリストへ移動',
    keywords: ['playlist'],
    handler: () => goto('/playlists'),
  },
  {
    id: 'app.nav.downloads',
    title: 'ダウンロードへ移動',
    keywords: ['download', 'dl'],
    handler: () => goto('/downloads'),
  },
  {
    id: 'app.nav.history',
    title: '視聴履歴へ移動',
    keywords: ['history'],
    handler: () => goto('/history'),
  },
  {
    id: 'app.nav.ng',
    title: 'NG 管理へ移動',
    keywords: ['ng', 'mute'],
    handler: () => goto('/ng'),
  },
  {
    id: 'app.nav.settings',
    title: '設定へ移動',
    keywords: ['settings', 'config', 'preferences'],
    handler: () => goto('/settings'),
  },
  {
    id: 'app.theme.toggle',
    title: 'テーマを切替 (ダーク ⇔ ニコニコクラシック)',
    keywords: ['theme', 'dark', 'classic', 'light'],
    handler: async () => {
      const cur = getStr('appearance.theme');
      const next = cur === 'dark' ? 'niconico-classic' : 'dark';
      await setSetting('appearance.theme', next);
    },
  },
];

/** 大文字小文字を無視した部分一致スコアリング。
 *  - 完全一致 (タイトル/id) は高スコア
 *  - 単語境界一致 (空白後一致) を加点
 *  - 文字順を維持した subsequence にも軽い加点
 *  - 0 を返したらフィルタ対象外。
 *
 * 厳密な fuzzy matcher (= fzf 並) は不要。30〜100 件レンジを「いいかんじ」
 * に並び替える程度で実用上十分。 */
export function scoreMatch(haystack: string, needle: string): number {
  if (needle.length === 0) return 1; // 空クエリ = 全件マッチ (順序維持)
  const h = haystack.toLowerCase();
  const n = needle.toLowerCase();
  if (h === n) return 1000;
  const idx = h.indexOf(n);
  if (idx >= 0) {
    // 先頭一致を一番強く、単語境界一致を中程度に評価。
    let s = 200 - idx;
    if (idx === 0) s += 200;
    if (idx > 0 && /\s|-|_|:|\.|\//.test(h[idx - 1])) s += 100;
    return Math.max(1, s);
  }
  // subsequence (文字順を保った点線一致)
  let hi = 0;
  let ni = 0;
  while (hi < h.length && ni < n.length) {
    if (h[hi] === n[ni]) ni++;
    hi++;
  }
  if (ni === n.length) return 1;
  return 0;
}

/** 検索結果を「組込み優先 → プラグイン後」で並べたフラットなリストにする。
 *  プラグイン側の項目には `pluginId` が attach されている。 */
export type CommandEntry =
  | (BuiltinCommand & { source: 'builtin' })
  | (PluginCommand & { source: 'plugin'; pluginId: string });

export function rankCommands(
  query: string,
  builtins: BuiltinCommand[],
  pluginCmds: (PluginCommand & { pluginId: string })[],
): CommandEntry[] {
  const scored: { entry: CommandEntry; score: number }[] = [];
  for (const b of builtins) {
    const sTitle = scoreMatch(b.title, query);
    const sId = scoreMatch(b.id, query);
    const sKw = (b.keywords ?? []).reduce((a, k) => Math.max(a, scoreMatch(k, query)), 0);
    const score = Math.max(sTitle, sId, sKw);
    if (score > 0) scored.push({ entry: { ...b, source: 'builtin' }, score: score + 50 });
  }
  for (const p of pluginCmds) {
    const sTitle = scoreMatch(p.title, query);
    const sHint = p.hint ? scoreMatch(p.hint, query) : 0;
    const sKw = (p.keywords ?? []).reduce((a, k) => Math.max(a, scoreMatch(k, query)), 0);
    const score = Math.max(sTitle, sHint, sKw);
    if (score > 0) scored.push({ entry: { ...p, source: 'plugin' }, score });
  }
  scored.sort((a, b) => b.score - a.score);
  return scored.map((s) => s.entry);
}
