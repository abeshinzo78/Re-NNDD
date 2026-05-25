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
});
