import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as bus from './eventBus';

beforeEach(() => {
  bus._resetForTests();
});

describe('plugin eventBus', () => {
  it('emit with no listeners is a no-op', () => {
    expect(() => bus.emit('player:play', { videoId: 'x', currentTime: 0 })).not.toThrow();
    expect(bus._handlerCount()).toBe(0);
  });

  it('on registers a handler and returns an unsubscribe', () => {
    const handler = vi.fn();
    const off = bus.on('plug.a', 'player:play', handler);
    bus.emit('player:play', { videoId: 'sm1', currentTime: 1 });
    expect(handler).toHaveBeenCalledOnce();
    off();
    bus.emit('player:play', { videoId: 'sm1', currentTime: 2 });
    expect(handler).toHaveBeenCalledOnce();
    expect(bus._handlerCount()).toBe(0);
  });

  it('multiple handlers on same event all fire in registration order', () => {
    const order: number[] = [];
    bus.on('a', 'evt', () => order.push(1));
    bus.on('b', 'evt', () => order.push(2));
    bus.on('c', 'evt', () => order.push(3));
    bus.emit('evt', null);
    expect(order).toEqual([1, 2, 3]);
  });

  it('a throwing handler does NOT prevent sibling handlers from running', () => {
    const sibling = vi.fn();
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    bus.on('bad', 'evt', () => {
      throw new Error('boom');
    });
    bus.on('good', 'evt', sibling);
    bus.emit('evt', { hello: 'world' });
    expect(sibling).toHaveBeenCalledWith({ hello: 'world' });
    expect(errorSpy).toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('offAllByOwner removes all handlers of that owner across event names', () => {
    const h1 = vi.fn();
    const h2 = vi.fn();
    const h3 = vi.fn();
    bus.on('plug.a', 'evt1', h1);
    bus.on('plug.a', 'evt2', h2);
    bus.on('plug.b', 'evt1', h3);
    bus.offAllByOwner('plug.a');
    bus.emit('evt1', null);
    bus.emit('evt2', null);
    expect(h1).not.toHaveBeenCalled();
    expect(h2).not.toHaveBeenCalled();
    expect(h3).toHaveBeenCalledOnce();
  });

  it('handler subscribing inside emit takes effect from NEXT emit only', () => {
    const seen: string[] = [];
    bus.on('a', 'evt', () => {
      seen.push('first');
      bus.on('a', 'evt', () => seen.push('second-from-first'));
    });
    bus.emit('evt', null);
    // Snapshot semantics: second handler is NOT invoked during the first emit
    expect(seen).toEqual(['first']);
    bus.emit('evt', null);
    expect(seen).toEqual(['first', 'first', 'second-from-first']);
  });

  it('re-on with same (owner, handler) is deduplicated (one invocation per emit)', () => {
    const h = vi.fn();
    const off1 = bus.on('plug.a', 'evt', h);
    const off2 = bus.on('plug.a', 'evt', h);
    expect(bus._handlerCount()).toBe(1);
    bus.emit('evt', null);
    expect(h).toHaveBeenCalledTimes(1);
    // Either off function should remove the single underlying entry.
    off1();
    expect(bus._handlerCount()).toBe(0);
    // off2 is a no-op now (the entry is already gone).
    off2();
    bus.emit('evt', null);
    expect(h).toHaveBeenCalledTimes(1);
  });

  it('different owners with same handler are NOT deduplicated', () => {
    const h = vi.fn();
    bus.on('plug.a', 'evt', h);
    bus.on('plug.b', 'evt', h);
    expect(bus._handlerCount()).toBe(2);
    bus.emit('evt', null);
    expect(h).toHaveBeenCalledTimes(2);
  });

  it('stale off() does NOT wipe a replacement bucket at the same name', () => {
    // Codex #12 回帰防止: 旧 bucket への参照を握った off クロージャが、
    // bucket 削除 → 別 owner が新 bucket 作成 のあとで誤って新 bucket を
    // 削除する race を防ぐ。
    const hA = vi.fn();
    const hB = vi.fn();
    const offA = bus.on('plug.a', 'evt', hA);
    // plug.a の全 listener を解除 → bucket は空になり 'evt' から削除される
    bus.offAllByOwner('plug.a');
    expect(bus._handlerCount()).toBe(0);
    // 別 owner が新しく登録 → 'evt' に新 bucket 生成
    bus.on('plug.b', 'evt', hB);
    expect(bus._handlerCount()).toBe(1);
    // ここで stale な offA が走っても、新 bucket (= plug.b の登録) を消しては
    // いけない。
    offA();
    expect(bus._handlerCount()).toBe(1);
    bus.emit('evt', null);
    expect(hB).toHaveBeenCalledOnce();
    expect(hA).not.toHaveBeenCalled();
  });
});
