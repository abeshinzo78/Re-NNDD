import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { readJson, writeJson, writeJsonSafe } from './localStorageJson';

const KEY = 'nndd:test-key';

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('readJson', () => {
  test('returns the parsed value when present and valid', () => {
    localStorage.setItem(KEY, JSON.stringify([1, 2, 3]));
    expect(
      readJson<number[]>(
        KEY,
        () => [],
        (p) => p as number[],
      ),
    ).toEqual([1, 2, 3]);
  });

  test('returns fallback when the key is absent', () => {
    expect(readJson(KEY, () => 'fallback')).toBe('fallback');
  });

  test('returns fallback when the stored value is empty string', () => {
    localStorage.setItem(KEY, '');
    expect(readJson(KEY, () => 'fallback')).toBe('fallback');
  });

  test('returns fallback (does not throw) on corrupt JSON', () => {
    localStorage.setItem(KEY, '{not valid json');
    expect(readJson(KEY, () => 'fallback')).toBe('fallback');
  });

  test('validate can reject a parsed-but-invalid value (e.g. non-array)', () => {
    localStorage.setItem(KEY, JSON.stringify({ notAnArray: true }));
    const out = readJson<number[]>(
      KEY,
      () => [],
      (p) => (Array.isArray(p) ? (p as number[]) : []),
    );
    expect(out).toEqual([]);
  });

  test('returns fallback (does not throw) when validate itself throws', () => {
    localStorage.setItem(KEY, JSON.stringify({ a: 1 }));
    const out = readJson(
      KEY,
      () => 'fallback',
      () => {
        throw new Error('boom');
      },
    );
    expect(out).toBe('fallback');
  });

  test('fallback is lazy — not evaluated when the value parses successfully', () => {
    localStorage.setItem(KEY, JSON.stringify(42));
    const fallback = vi.fn(() => 0);
    readJson<number>(KEY, fallback, (p) => p as number);
    expect(fallback).not.toHaveBeenCalled();
  });

  test('returns fallback when localStorage is unavailable (SSR guard)', () => {
    vi.stubGlobal('localStorage', undefined);
    expect(readJson(KEY, () => 'ssr')).toBe('ssr');
  });
});

describe('writeJson', () => {
  test('serializes and stores the value, returning true', () => {
    expect(writeJson(KEY, { a: 1 })).toBe(true);
    expect(JSON.parse(localStorage.getItem(KEY)!)).toEqual({ a: 1 });
  });

  test('propagates a setItem exception (quota) — does NOT swallow', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('quota', 'QuotaExceededError');
    });
    expect(() => writeJson(KEY, { a: 1 })).toThrow();
  });

  test('returns false without writing when localStorage is unavailable', () => {
    vi.stubGlobal('localStorage', undefined);
    expect(writeJson(KEY, { a: 1 })).toBe(false);
  });
});

describe('writeJsonSafe', () => {
  test('serializes and stores the value, returning true', () => {
    expect(writeJsonSafe(KEY, { a: 1 })).toBe(true);
    expect(JSON.parse(localStorage.getItem(KEY)!)).toEqual({ a: 1 });
  });

  test('removes the key when value is null', () => {
    localStorage.setItem(KEY, JSON.stringify({ a: 1 }));
    expect(writeJsonSafe(KEY, null)).toBe(true);
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  test('swallows a setItem exception (quota) and still returns true', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('quota', 'QuotaExceededError');
    });
    expect(writeJsonSafe(KEY, { a: 1 })).toBe(true);
  });

  test('returns false without writing when localStorage is unavailable', () => {
    vi.stubGlobal('localStorage', undefined);
    expect(writeJsonSafe(KEY, { a: 1 })).toBe(false);
  });
});
