// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { SETTING_DEFS } from './settings.svelte';

describe('SETTING_DEFS (regression guard for plugin system)', () => {
  // プラグイン機構追加で 1 件 (plugins.enabled) が増える。これが unexpected に
  // 増減したら気付けるよう、厳密な数で固定する。新しい設定追加時はここも更新する。
  // (built-in 15 件 + plugins.enabled 1 件 = 16)
  it('has exactly the expected number of built-in settings', () => {
    expect(SETTING_DEFS.length).toBe(16);
  });

  it('contains the plugins.enabled kill switch', () => {
    const def = SETTING_DEFS.find((d) => d.key === 'plugins.enabled');
    expect(def).toBeTruthy();
    expect(def?.default).toBe(true);
    expect(def?.kind).toBe('bool');
    expect(def?.section).toBe('advanced');
  });

  it('all setting keys are unique', () => {
    const keys = SETTING_DEFS.map((d) => d.key);
    expect(new Set(keys).size).toBe(keys.length);
  });
});
