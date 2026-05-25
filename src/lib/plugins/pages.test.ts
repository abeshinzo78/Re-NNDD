// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import * as reg from './registry';

beforeEach(() => {
  reg.clearAll();
});

describe('plugin pages registry', () => {
  it('pluginPage returns null when nothing is registered', () => {
    expect(reg.pluginPage('p.x', 'dashboard')).toBe(null);
  });

  it('addPage + pluginPage round-trip', () => {
    const render = () => undefined;
    reg.addPage('p.a', '/dashboard', render);
    expect(reg.pluginPage('p.a', 'dashboard')).toBe(render);
    expect(reg.pluginPage('p.a', '/dashboard')).toBe(render); // leading slash 許容
  });

  it('different plugins have isolated namespaces', () => {
    const rA = () => undefined;
    const rB = () => undefined;
    reg.addPage('p.a', 'main', rA);
    reg.addPage('p.b', 'main', rB);
    expect(reg.pluginPage('p.a', 'main')).toBe(rA);
    expect(reg.pluginPage('p.b', 'main')).toBe(rB);
  });

  it('falls back to root renderer when subpath has no exact match', () => {
    const rootRender = () => undefined;
    reg.addPage('p.a', '', rootRender);
    expect(reg.pluginPage('p.a', 'unknown')).toBe(rootRender);
  });

  it('removeAllByPlugin clears that plugin pages', () => {
    reg.addPage('p.a', 'main', () => undefined);
    reg.addPage('p.b', 'main', () => undefined);
    reg.removeAllByPlugin('p.a');
    expect(reg.pluginPage('p.a', 'main')).toBe(null);
    expect(reg.pluginPage('p.b', 'main')).not.toBe(null);
  });

  it('counts include pages', () => {
    reg.addPage('p.a', 'main', () => undefined);
    reg.addPage('p.a', 'sub', () => undefined);
    reg.addPage('p.b', 'main', () => undefined);
    expect(reg._counts().pages).toBe(3);
  });
});

describe('plugin commands registry', () => {
  it('pluginCommands starts empty', () => {
    expect(reg.pluginCommands()).toEqual([]);
  });

  it('addCommand attaches pluginId on listing', () => {
    reg.addCommand('p.a', { id: 'c1', title: 'C1', handler: () => undefined });
    const list = reg.pluginCommands();
    expect(list).toHaveLength(1);
    expect(list[0]).toMatchObject({ pluginId: 'p.a', id: 'c1', title: 'C1' });
  });

  it('addCommand de-duplicates by id within the same plugin (last-wins)', () => {
    reg.addCommand('p.a', { id: 'c1', title: 'C1 first', handler: () => undefined });
    reg.addCommand('p.a', { id: 'c1', title: 'C1 second', handler: () => undefined });
    const list = reg.pluginCommands().filter((c) => c.pluginId === 'p.a' && c.id === 'c1');
    expect(list).toHaveLength(1);
    expect(list[0].title).toBe('C1 second');
  });

  it('same id from different plugins are kept separate', () => {
    reg.addCommand('p.a', { id: 'shared', title: 'A', handler: () => undefined });
    reg.addCommand('p.b', { id: 'shared', title: 'B', handler: () => undefined });
    const list = reg.pluginCommands();
    expect(list).toHaveLength(2);
  });

  it('removeAllByPlugin clears that plugin commands', () => {
    reg.addCommand('p.a', { id: 'c1', title: 'C1', handler: () => undefined });
    reg.addCommand('p.b', { id: 'c1', title: 'C1', handler: () => undefined });
    reg.removeAllByPlugin('p.a');
    expect(reg.pluginCommands()).toHaveLength(1);
    expect(reg.pluginCommands()[0].pluginId).toBe('p.b');
  });
});
