import { beforeEach, describe, expect, test } from 'vitest';
import { readSavedMuted, readSavedVolume, saveMuted, saveVolume } from './volumePersistence';

beforeEach(() => {
  localStorage.clear();
});

describe('readSavedVolume / saveVolume', () => {
  test('returns null when nothing saved', () => {
    expect(readSavedVolume()).toBeNull();
  });

  test('round-trips a normal value', () => {
    saveVolume(0.42);
    expect(readSavedVolume()).toBeCloseTo(0.42);
  });

  test('clamps out-of-range values when saving', () => {
    saveVolume(1.5);
    expect(readSavedVolume()).toBe(1);
    saveVolume(-0.5);
    expect(readSavedVolume()).toBe(0);
  });

  test('ignores non-finite values when saving', () => {
    saveVolume(0.3);
    saveVolume(NaN);
    expect(readSavedVolume()).toBeCloseTo(0.3);
    saveVolume(Infinity);
    expect(readSavedVolume()).toBeCloseTo(0.3);
  });

  test('treats corrupt stored values as absent', () => {
    localStorage.setItem('player.lastVolume.v1', 'nope');
    expect(readSavedVolume()).toBeNull();
  });
});

describe('readSavedMuted / saveMuted', () => {
  test('defaults to false', () => {
    expect(readSavedMuted()).toBe(false);
  });

  test('round-trips true and false', () => {
    saveMuted(true);
    expect(readSavedMuted()).toBe(true);
    saveMuted(false);
    expect(readSavedMuted()).toBe(false);
  });
});
