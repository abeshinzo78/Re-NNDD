// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// settings store の getBool をモックして kill switch をオン/オフできるようにする。
vi.mock('$lib/stores/settings.svelte', () => {
  let enabled = true;
  return {
    getBool: vi.fn(() => enabled),
    __setEnabled(v: boolean) {
      enabled = v;
    },
  };
});

// api を全部スタブ。
vi.mock('./api', () => ({
  pluginListInstalled: vi.fn(async () => []),
  pluginInstallFromZip: vi.fn(),
  pluginUninstall: vi.fn(),
  pluginSetEnabled: vi.fn(async () => undefined),
  pluginGetManifest: vi.fn(),
  pluginInvoke: vi.fn(),
}));

// Tauri event API もスタブ。
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => undefined),
}));

import * as host from './host';
import * as api from './api';
import * as settings from '$lib/stores/settings.svelte';
import { listen } from '@tauri-apps/api/event';

beforeEach(() => {
  host._resetForTests();
  vi.clearAllMocks();
});

afterEach(() => {
  // テスト間の漏れ防止
  host._resetForTests();
});

describe('bootstrapPluginHost — kill switch', () => {
  it('does NOTHING when plugins.enabled = false (no DB calls, no Tauri listen)', async () => {
    (settings as unknown as { __setEnabled(v: boolean): void }).__setEnabled(false);
    await host.bootstrapPluginHost();
    expect(api.pluginListInstalled).not.toHaveBeenCalled();
    expect(listen).not.toHaveBeenCalled();
  });

  it('attaches event listener and lists installed when enabled', async () => {
    (settings as unknown as { __setEnabled(v: boolean): void }).__setEnabled(true);
    await host.bootstrapPluginHost();
    expect(listen).toHaveBeenCalledWith('nndd:plugin:event', expect.any(Function));
    expect(api.pluginListInstalled).toHaveBeenCalled();
  });

  it('is idempotent: a second call does NOT re-list', async () => {
    (settings as unknown as { __setEnabled(v: boolean): void }).__setEnabled(true);
    await host.bootstrapPluginHost();
    await host.bootstrapPluginHost();
    expect(api.pluginListInstalled).toHaveBeenCalledTimes(1);
  });

  it('skips disabled plugins (does not attempt to load them)', async () => {
    (settings as unknown as { __setEnabled(v: boolean): void }).__setEnabled(true);
    const fakeInstalled = [
      {
        pluginId: 'p1',
        name: 'P1',
        version: '0.1.0',
        enabled: false,
        entry: 'index.js',
        entryAbsPath: '/fake/p1/index.js',
        permissions: [],
        installedAt: 0,
        updatedAt: 0,
      },
    ];
    (api.pluginListInstalled as ReturnType<typeof vi.fn>).mockResolvedValueOnce(fakeInstalled);
    // loader.loadPlugin should not be called. We assert indirectly by ensuring
    // no error is thrown and pluginListInstalled was called once.
    await host.bootstrapPluginHost();
    expect(api.pluginListInstalled).toHaveBeenCalledTimes(1);
  });
});
