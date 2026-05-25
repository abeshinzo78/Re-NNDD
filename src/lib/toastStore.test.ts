// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { _count, _resetForTests, dismissToast, listToasts, showToast } from './toastStore.svelte';

beforeEach(() => {
  _resetForTests();
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('toastStore', () => {
  it('starts empty', () => {
    expect(_count()).toBe(0);
    expect(listToasts()).toEqual([]);
  });

  it('showToast adds an entry with default kind=info', () => {
    const id = showToast('hello');
    expect(_count()).toBe(1);
    const t = listToasts()[0];
    expect(t.id).toBe(id);
    expect(t.message).toBe('hello');
    expect(t.kind).toBe('info');
    expect(t.pluginId).toBe(null);
  });

  it('showToast respects kind and pluginId options', () => {
    showToast('boom', 'error', { pluginId: 'p.a' });
    const t = listToasts()[0];
    expect(t.kind).toBe('error');
    expect(t.pluginId).toBe('p.a');
  });

  it('auto-dismisses after default duration (3500ms)', () => {
    showToast('disappearing');
    expect(_count()).toBe(1);
    vi.advanceTimersByTime(3499);
    expect(_count()).toBe(1);
    vi.advanceTimersByTime(2);
    expect(_count()).toBe(0);
  });

  it('does NOT auto-dismiss when durationMs <= 0', () => {
    showToast('sticky', 'info', { durationMs: 0 });
    vi.advanceTimersByTime(10_000);
    expect(_count()).toBe(1);
  });

  it('dismissToast removes an entry immediately', () => {
    const id = showToast('x');
    dismissToast(id);
    expect(_count()).toBe(0);
  });

  it('caps at 5 toasts: oldest dropped when 6th arrives', () => {
    for (let i = 0; i < 5; i++) showToast(`t${i}`);
    expect(_count()).toBe(5);
    const oldestId = listToasts()[0].id;
    showToast('t5');
    expect(_count()).toBe(5);
    // 最古が消えて、新規が末尾に入っている
    expect(listToasts().some((t) => t.id === oldestId)).toBe(false);
    expect(listToasts()[listToasts().length - 1].message).toBe('t5');
  });
});
