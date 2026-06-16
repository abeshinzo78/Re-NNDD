// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { thumbFallback } from './thumbnail';

/** 全マイクロタスク + 0ms タイマを流す。 */
const flush = () => new Promise((r) => setTimeout(r, 0));
const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

function makeImg(src: string): HTMLImageElement {
  const img = document.createElement('img');
  img.src = src;
  document.body.appendChild(img);
  return img;
}

describe('thumbFallback', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    document.body.innerHTML = '';
  });
  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('読み込み失敗時に getthumbinfo から現行 URL を引き直して貼り替える', async () => {
    invokeMock.mockResolvedValue('https://cdn.example/thumbnails/1/1.abcdef');
    const img = makeImg('https://cdn.example/thumbnails/1/1');
    const action = thumbFallback(img, { videoId: 'sm1' });

    img.dispatchEvent(new Event('error'));
    await flush();

    expect(invokeMock).toHaveBeenCalledWith('resolve_thumbnail_url', { videoId: 'sm1' });
    expect(img.src).toContain('1.abcdef');
    expect(img.dataset.thumbBroken).toBeUndefined();
    action.destroy();
  });

  it('再解決でも回復しなければ最終的にプレースホルダ化する', async () => {
    // 現行 URL も同じ(=差し替えても無駄) → リトライ → プレースホルダ。
    invokeMock.mockResolvedValue('https://cdn.example/thumbnails/2/2');
    const img = makeImg('https://cdn.example/thumbnails/2/2');
    const action = thumbFallback(img, { videoId: 'sm2' });

    img.dispatchEvent(new Event('error')); // ① 再解決(同 URL) → ② リトライ予約
    await flush();
    await delay(400); // リトライの貼り直しが走る
    img.dispatchEvent(new Event('error')); // ③ 万策尽きる
    await flush();

    expect(img.dataset.thumbBroken).toBe('true');
    expect(img.src).toContain('data:image/gif');
    action.destroy();
  });

  it('videoId が無ければ再解決せず、リトライのみで最終的にプレースホルダ化する', async () => {
    const img = makeImg('https://cdn.example/thumbnails/3/3');
    const action = thumbFallback(img, {});

    img.dispatchEvent(new Event('error')); // ② リトライ予約 (再解決はスキップ)
    await delay(400);
    img.dispatchEvent(new Event('error')); // ③ プレースホルダ

    expect(invokeMock).not.toHaveBeenCalled();
    expect(img.dataset.thumbBroken).toBe('true');
    action.destroy();
  });

  it('destroy 後は error を無視する', async () => {
    const img = makeImg('https://cdn.example/thumbnails/4/4');
    const action = thumbFallback(img, { videoId: 'sm4' });
    action.destroy();

    img.dispatchEvent(new Event('error'));
    await flush();

    expect(invokeMock).not.toHaveBeenCalled();
    expect(img.dataset.thumbBroken).toBeUndefined();
  });

  it('update で videoId が変わるとフォールバック状態をリセットする', async () => {
    invokeMock.mockResolvedValue('https://cdn.example/thumbnails/5/5.new');
    const img = makeImg('https://cdn.example/thumbnails/5/5');
    const action = thumbFallback(img, { videoId: 'sm5' });

    img.dispatchEvent(new Event('error'));
    await flush();
    expect(invokeMock).toHaveBeenCalledTimes(1);

    // 別動画にバインドし直す → 再び再解決が効くようになる。
    invokeMock.mockResolvedValue('https://cdn.example/thumbnails/6/6.new');
    action.update?.({ videoId: 'sm6' });
    img.src = 'https://cdn.example/thumbnails/6/6';
    img.dispatchEvent(new Event('error'));
    await flush();

    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(invokeMock).toHaveBeenLastCalledWith('resolve_thumbnail_url', { videoId: 'sm6' });
    action.destroy();
  });
});
