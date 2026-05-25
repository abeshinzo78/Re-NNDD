// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as bus from './eventBus';
import * as registry from './registry';

vi.mock('@tauri-apps/api/core', () => ({
  // path をそのまま返す (テストでは asset:// 変換不要)
  convertFileSrc: (p: string) => p,
}));

vi.mock('./api', () => ({
  pluginListInstalled: vi.fn(),
  pluginInstallFromZip: vi.fn(),
  pluginUninstall: vi.fn(),
  pluginSetEnabled: vi.fn(),
  pluginGetManifest: vi.fn(),
  pluginInvoke: vi.fn(async () => null),
}));

import * as loader from './loader';
import type { PluginInfo } from './types';

const baseInfo = (id: string): PluginInfo => ({
  pluginId: id,
  name: id,
  version: '0.1.0',
  enabled: true,
  entry: 'index.js',
  entryAbsPath: `/fake/${id}/index.js`,
  permissions: [],
  installedAt: 0,
  updatedAt: 0,
});

beforeEach(() => {
  bus._resetForTests();
  registry.clearAll();
  loader._resetForTests();
  vi.clearAllMocks();
});

describe('RESERVED_HOST_EVENT_NAMES', () => {
  it('contains the player.control event used by Player.svelte', () => {
    // この event を block しないと permission バイパスが成立する
    // (Codex r3299045281 回帰防止)。
    expect(loader.RESERVED_HOST_EVENT_NAMES.has('plugin:player:control')).toBe(true);
  });

  it('contains all standard host-emitted events to prevent plugin spoofing', () => {
    for (const name of [
      'notify:toast',
      'download:start',
      'download:complete',
      'download:error',
      'player:play',
      'player:pause',
      'player:time',
      'player:ended',
    ]) {
      expect(loader.RESERVED_HOST_EVENT_NAMES.has(name)).toBe(true);
    }
  });

  it('does not include arbitrary plugin namespaces', () => {
    // プラグイン同士の通信 (`custom:foo` 等) は引き続き emit 可能。
    expect(loader.RESERVED_HOST_EVENT_NAMES.has('custom:my-event')).toBe(false);
    expect(loader.RESERVED_HOST_EVENT_NAMES.has('plugin:my-data')).toBe(false);
  });
});

describe('loader.loadPlugin', () => {
  it('records failed state when dynamic import rejects', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const info = baseInfo('p.failed');
    // Vite が解析できない動的 import は実行時に 404 で失敗するはず
    // (テスト環境では fetch されないが、import() 自体が失敗する)
    await loader.loadPlugin(info);
    const state = loader.getLoadState(info.pluginId);
    expect(state?.state).toBe('failed');
    expect(errorSpy).toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('unloadPlugin clears registry contributions and bus listeners for that plugin', async () => {
    registry.addNav('p.x', { href: '/p.x', label: 'X' });
    bus.on('p.x', 'evt', () => undefined);
    expect(registry.pluginNavEntries()).toHaveLength(1);
    expect(bus._handlerCount()).toBe(1);
    await loader.unloadPlugin('p.x');
    expect(registry.pluginNavEntries()).toHaveLength(0);
    expect(bus._handlerCount()).toBe(0);
  });

  it('failed load rolls back partial contributions and listeners', async () => {
    // 失敗パスを手動で再現: loadPlugin が dynamic import に失敗する前に、
    // 失敗より前のフェーズで他のプラグインが登録した寄与は残ってはいけない
    // (Codex review r3297535055)。ここでは register → 失敗 のシナリオを
    // 直接シミュレートする: 寄与を入れておいてから loadPlugin (確実に
    // dynamic import で fail) を呼ぶ。失敗時に同じ pluginId の寄与だけが
    // 取り除かれることを assert する。
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const info = baseInfo('p.rollback');
    // 別プラグインの寄与は残る
    registry.addNav('p.other', { href: '/other', label: 'O' });
    // 同じプラグインの "幽霊" 寄与は失敗で消える
    registry.addNav('p.rollback', { href: '/r', label: 'R' });
    bus.on('p.rollback', 'evt', () => undefined);
    expect(registry.pluginNavEntries()).toHaveLength(2);
    expect(bus._handlerCount()).toBe(1);
    await loader.loadPlugin(info);
    expect(loader.getLoadState('p.rollback')?.state).toBe('failed');
    expect(registry.pluginNavEntries()).toEqual([{ href: '/other', label: 'O' }]);
    expect(bus._handlerCount()).toBe(0);
    errSpy.mockRestore();
  });
});
