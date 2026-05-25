// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import {
  _setForTests,
  clearPlayerState,
  getPlayerState,
  updatePlayerState,
} from './playerState.svelte';

beforeEach(() => {
  clearPlayerState();
});

describe('playerState', () => {
  it('initial state has videoId=null and zero values', () => {
    const s = getPlayerState();
    expect(s.videoId).toBe(null);
    expect(s.currentTime).toBe(0);
    expect(s.duration).toBe(0);
    expect(s.paused).toBe(true);
    expect(s.volume).toBe(1);
    expect(s.muted).toBe(false);
    expect(s.playbackRate).toBe(1);
  });

  it('updatePlayerState merges partial values', () => {
    updatePlayerState({ videoId: 'sm123', currentTime: 42 });
    const s = getPlayerState();
    expect(s.videoId).toBe('sm123');
    expect(s.currentTime).toBe(42);
    // 他のフィールドはデフォルトのまま
    expect(s.paused).toBe(true);
    expect(s.volume).toBe(1);
  });

  it('returns a defensive copy (plugin cannot mutate the store)', () => {
    updatePlayerState({ videoId: 'sm1', currentTime: 5 });
    const s = getPlayerState();
    s.currentTime = 9999;
    s.videoId = 'EVIL';
    const s2 = getPlayerState();
    expect(s2.currentTime).toBe(5);
    expect(s2.videoId).toBe('sm1');
  });

  it('clearPlayerState resets everything', () => {
    updatePlayerState({ videoId: 'sm1', currentTime: 10, paused: false });
    clearPlayerState();
    const s = getPlayerState();
    expect(s.videoId).toBe(null);
    expect(s.currentTime).toBe(0);
    expect(s.paused).toBe(true);
  });

  it('_setForTests replaces entire state', () => {
    _setForTests({
      videoId: 'sm999',
      currentTime: 100,
      duration: 200,
      paused: false,
      volume: 0.5,
      muted: true,
      playbackRate: 1.5,
    });
    const s = getPlayerState();
    expect(s.videoId).toBe('sm999');
    expect(s.duration).toBe(200);
    expect(s.muted).toBe(true);
    expect(s.playbackRate).toBe(1.5);
  });
});
