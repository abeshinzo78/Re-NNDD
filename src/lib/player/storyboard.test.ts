import { describe, expect, it } from 'vitest';
import type { Storyboard } from '$lib/api';
import { cssUrl, storyboardFrameAt, storyboardRefreshDelay } from './storyboard';

// 2026-11 に sm9 (320 秒) で取れた形: 120×90 のコマを 10×10、2 秒おき、2 枚。
const SB: Storyboard = {
  thumbnailWidth: 120,
  thumbnailHeight: 90,
  columns: 10,
  rows: 10,
  intervalMs: 2000,
  images: [
    { timestampMs: 0, url: 'https://asset.domand.nicovideo.jp/x/0001.jpg?s=1' },
    { timestampMs: 200000, url: 'https://asset.domand.nicovideo.jp/x/0002.jpg?s=1' },
  ],
};

describe('storyboardFrameAt', () => {
  it('picks the frame in row-major order within a sheet', () => {
    expect(storyboardFrameAt(SB, 0)).toMatchObject({ x: 0, y: 0, url: SB.images[0].url });
    expect(storyboardFrameAt(SB, 1.99)).toMatchObject({ x: 0, y: 0 });
    expect(storyboardFrameAt(SB, 2)).toMatchObject({ x: 120, y: 0 });
    expect(storyboardFrameAt(SB, 19)).toMatchObject({ x: 1080, y: 0 });
    expect(storyboardFrameAt(SB, 20)).toMatchObject({ x: 0, y: 90 });
    expect(storyboardFrameAt(SB, 199.9)).toMatchObject({ x: 1080, y: 810 });
  });

  it('moves to the next sheet at its timestamp', () => {
    const f = storyboardFrameAt(SB, 200);
    expect(f).toMatchObject({ x: 0, y: 0, url: SB.images[1].url });
    expect(f).toMatchObject({ width: 120, height: 90, sheetWidth: 1200, sheetHeight: 900 });
  });

  it('clamps to the last frame of the video when the duration is known', () => {
    // 320 秒の動画の最後のコマは 318 秒 (2 枚目の 59 番目)。
    expect(storyboardFrameAt(SB, 320, 320)).toMatchObject({ x: 1080, y: 450 });
    expect(storyboardFrameAt(SB, 9999, 320)).toMatchObject({ x: 1080, y: 450 });
    // 長さが分からなければ最後の画像の最後のコマで止める。
    expect(storyboardFrameAt(SB, 9999)).toMatchObject({ x: 1080, y: 810 });
  });

  it('treats negative times as the start', () => {
    expect(storyboardFrameAt(SB, -5)).toMatchObject({ x: 0, y: 0, url: SB.images[0].url });
  });

  it('returns null for missing or broken data', () => {
    expect(storyboardFrameAt(null, 10)).toBeNull();
    expect(storyboardFrameAt(undefined, 10)).toBeNull();
    expect(storyboardFrameAt({ ...SB, images: [] }, 10)).toBeNull();
    expect(storyboardFrameAt({ ...SB, columns: 0 }, 10)).toBeNull();
    expect(storyboardFrameAt({ ...SB, intervalMs: 0 }, 10)).toBeNull();
    expect(storyboardFrameAt(SB, Number.NaN)).toBeNull();
  });
});

describe('storyboardRefreshDelay', () => {
  const now = Date.parse('2026-11-12T20:25:36+09:00');

  it('refreshes five minutes before the signature expires', () => {
    expect(storyboardRefreshDelay('2026-11-12T23:25:36+09:00', now)).toBe(
      3 * 60 * 60 * 1000 - 5 * 60 * 1000,
    );
  });

  it('waits at least a minute when the expiry is near or past', () => {
    expect(storyboardRefreshDelay('2026-11-12T20:27:36+09:00', now)).toBe(60 * 1000);
    expect(storyboardRefreshDelay('2026-11-12T19:00:00+09:00', now)).toBe(60 * 1000);
  });

  it('does not schedule anything without a readable expiry', () => {
    expect(storyboardRefreshDelay(undefined, now)).toBeNull();
    expect(storyboardRefreshDelay('not a date', now)).toBeNull();
  });
});

describe('cssUrl', () => {
  it('quotes the URL for CSS', () => {
    expect(cssUrl('https://a.test/x.jpg?a=1&b=~_')).toBe('url("https://a.test/x.jpg?a=1&b=~_")');
    expect(cssUrl('https://a.test/"x".jpg')).toBe('url("https://a.test/%22x%22.jpg")');
  });
});
