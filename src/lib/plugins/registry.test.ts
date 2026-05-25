// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import * as reg from './registry';

beforeEach(() => {
  reg.clearAll();
});

describe('plugin registry', () => {
  it('starts empty', () => {
    expect(reg._counts()).toEqual({ nav: 0, settings: 0, items: 0, player: 0 });
    expect(reg.pluginNavEntries()).toEqual([]);
    expect(reg.pluginSettingDefs()).toEqual([]);
    expect(reg.pluginItemActions()).toEqual([]);
    expect(reg.pluginPlayerActions()).toEqual([]);
  });

  it('addNav adds an entry keyed by plugin id', () => {
    reg.addNav('plug.a', { href: '/plugin/plug.a/main', label: 'A' });
    expect(reg.pluginNavEntries()).toEqual([{ href: '/plugin/plug.a/main', label: 'A' }]);
  });

  it('removeAllByPlugin removes only that plugin contributions', () => {
    reg.addNav('plug.a', { href: '/a', label: 'A' });
    reg.addNav('plug.b', { href: '/b', label: 'B' });
    reg.addSetting('plug.a', { key: 'plugin.plug.a.k', label: 'k', kind: 'bool', default: false });
    reg.addItemAction('plug.a', { label: 'do', handler: () => undefined });
    reg.addPlayerAction('plug.a', { label: 'btn', handler: () => undefined });

    reg.removeAllByPlugin('plug.a');

    expect(reg.pluginNavEntries()).toEqual([{ href: '/b', label: 'B' }]);
    expect(reg.pluginSettingDefs()).toEqual([]);
    expect(reg.pluginItemActions()).toEqual([]);
    expect(reg.pluginPlayerActions()).toEqual([]);
  });

  it('multiple contributions from same plugin accumulate', () => {
    reg.addNav('plug.a', { href: '/a/1', label: '1' });
    reg.addNav('plug.a', { href: '/a/2', label: '2' });
    expect(reg.pluginNavEntries()).toHaveLength(2);
  });

  it('clearAll wipes everything', () => {
    reg.addNav('plug.a', { href: '/a', label: 'A' });
    reg.addNav('plug.b', { href: '/b', label: 'B' });
    reg.clearAll();
    expect(reg.pluginNavEntries()).toEqual([]);
  });
});
