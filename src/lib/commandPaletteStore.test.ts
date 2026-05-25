// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';

// goto / settings は ranking/scoring に直接関係しないのでスタブ。
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$lib/stores/settings.svelte', () => ({
  getStr: vi.fn(() => 'dark'),
  setSetting: vi.fn(async () => undefined),
}));

import { BUILTIN_COMMANDS, rankCommands, scoreMatch } from './commandPaletteStore.svelte';
import type { PluginCommand } from '$lib/plugins/types';

describe('scoreMatch', () => {
  it('empty query matches everything (returns 1)', () => {
    expect(scoreMatch('Hello World', '')).toBe(1);
  });

  it('exact match returns 1000 (highest)', () => {
    expect(scoreMatch('Hello', 'Hello')).toBe(1000);
    expect(scoreMatch('foo', 'FOO')).toBe(1000); // case insensitive
  });

  it('prefix match scores higher than middle match', () => {
    // 自然な英単語の prefix を haystack/needle に置くとスペルチェッカが
    // 「短い prefix → typo」と誤検出するため、語彙的に中立な文字列を使う。
    expect(scoreMatch('foobar baz', 'foo')).toBeGreaterThan(scoreMatch('xyz foo', 'foo'));
  });

  it('word boundary boosts (after space/hyphen/_/etc.)', () => {
    // どちらも substring match だが、後者は単語境界 (空白後) で hit する
    const after = scoreMatch('xyz foobar', 'foobar');
    const inside = scoreMatch('zfoobarz', 'foobar');
    expect(after).toBeGreaterThan(inside);
  });

  it('returns 0 for non-substring non-subsequence', () => {
    expect(scoreMatch('abc', 'xyz')).toBe(0);
  });

  it('returns positive (1) for subsequence-only match', () => {
    // a..c..e は abcde の subsequence
    expect(scoreMatch('abcdefg', 'ace')).toBeGreaterThan(0);
  });
});

describe('rankCommands', () => {
  it('builtin commands appear with builtin source tag', () => {
    const result = rankCommands('home', BUILTIN_COMMANDS, []);
    expect(result.length).toBeGreaterThan(0);
    expect(result.every((r) => r.source === 'builtin')).toBe(true);
  });

  it('plugin commands are tagged plugin and carry pluginId', () => {
    const plugins: (PluginCommand & { pluginId: string })[] = [
      {
        pluginId: 'p.test',
        id: 'p.test.greet',
        title: 'Greet user',
        handler: () => undefined,
      },
    ];
    const result = rankCommands('greet', BUILTIN_COMMANDS, plugins);
    const pluginHit = result.find((r) => r.source === 'plugin');
    expect(pluginHit).toBeDefined();
    if (pluginHit && pluginHit.source === 'plugin') {
      expect(pluginHit.pluginId).toBe('p.test');
    }
  });

  it('builtin gets +50 boost over plugin with equal title score', () => {
    // Built-in "ホームへ移動" vs plugin command の同じ語
    const plugins: (PluginCommand & { pluginId: string })[] = [
      { pluginId: 'p.x', id: 'p.x.home', title: 'ホームへ移動', handler: () => undefined },
    ];
    const result = rankCommands('ホーム', BUILTIN_COMMANDS, plugins);
    // 最初の hit は組込みであるべき (同点なら +50 で組込み優先)
    expect(result[0].source).toBe('builtin');
  });

  it('zero-score entries are filtered out', () => {
    const result = rankCommands('zzz-no-such-command', BUILTIN_COMMANDS, []);
    expect(result).toEqual([]);
  });

  it('keyword matches even when title does not', () => {
    // 組込み app.nav.search の keywords に `search` が入っている
    const result = rankCommands('search', BUILTIN_COMMANDS, []);
    expect(result.some((r) => r.id === 'app.nav.search')).toBe(true);
  });
});
