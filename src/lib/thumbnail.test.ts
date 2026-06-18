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

  it('失敗(null)した再解決はキャッシュせず、次回はバックエンドへ再問い合わせする', async () => {
    invokeMock.mockResolvedValue(null);
    const img1 = makeImg('https://cdn.example/thumbnails/9/9');
    const a1 = thumbFallback(img1, { videoId: 'sm9' });
    img1.dispatchEvent(new Event('error'));
    await flush();
    expect(invokeMock).toHaveBeenCalledTimes(1);

    // 同じ動画 ID でも、前回 null だったらキャッシュされていないので再度引く。
    const img2 = makeImg('https://cdn.example/thumbnails/9/9');
    const a2 = thumbFallback(img2, { videoId: 'sm9' });
    img2.dispatchEvent(new Event('error'));
    await flush();
    expect(invokeMock).toHaveBeenCalledTimes(2);
    a1.destroy();
    a2.destroy();
  });

  it('成功した再解決はキャッシュして重複問い合わせを避ける', async () => {
    invokeMock.mockResolvedValue('https://cdn.example/thumbnails/10/10.x');
    const img1 = makeImg('https://cdn.example/thumbnails/10/10');
    const a1 = thumbFallback(img1, { videoId: 'sm10' });
    img1.dispatchEvent(new Event('error'));
    await flush();

    const img2 = makeImg('https://cdn.example/thumbnails/10/10');
    const a2 = thumbFallback(img2, { videoId: 'sm10' });
    img2.dispatchEvent(new Event('error'));
    await flush();

    expect(invokeMock).toHaveBeenCalledTimes(1); // 2 回目はキャッシュ命中
    a1.destroy();
    a2.destroy();
  });

  it('再解決の解決前に別動画へ rebind されたら旧 URL を書き込まない', async () => {
    let settle: (v: string | null) => void = () => {};
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<string | null>((res) => {
          settle = res;
        }),
    );
    const img = makeImg('https://cdn.example/thumbnails/11/11');
    const action = thumbFallback(img, { videoId: 'sm11' });

    img.dispatchEvent(new Event('error')); // sm11 の再解決を開始(未解決のまま保留)
    await flush();

    // 解決前に別動画へ貼り替え(リスト行の使い回しを模す)。
    action.update?.({ videoId: 'sm12' });
    img.src = 'https://cdn.example/thumbnails/12/12';

    // 旧 sm11 の再解決がいま解決しても、世代が違うので src は触らない。
    settle('https://cdn.example/thumbnails/11/11.OLD');
    await flush();

    expect(img.src).toContain('/12/12');
    expect(img.src).not.toContain('OLD');
    action.destroy();
  });

  it('videoId 無しでも、リトライ保留中に別サムネへ使い回されたら新 src を壊さない', async () => {
    // シリーズ/マイリスト表紙のような use:thumbFallback 単独呼び出し
    // (videoId 無し) で、同じ <img> が別サムネに使い回されるケース。
    const img = makeImg('https://cdn.example/thumbnails/A/A');
    const action = thumbFallback(img, {});

    img.dispatchEvent(new Event('error')); // urlA のリトライを予約
    await flush();

    // リトライ発火前に Svelte が同じノードを別サムネ B に使い回す。
    img.src = 'https://cdn.example/thumbnails/B/B';
    await delay(400); // urlA のリトライタイマが発火するが…

    // B を空にしたり A を復元したりしていない。
    expect(img.src).toContain('/B/B');
    expect(img.src).not.toContain('/A/A');
    action.destroy();
  });

  it('使い回し後に新サムネがエラーしたら、前の broken 状態を引き継がず新規処理する', async () => {
    const img = makeImg('https://cdn.example/thumbnails/E/E');
    const action = thumbFallback(img, {});

    // E を最後まで失敗させてプレースホルダ化。
    img.dispatchEvent(new Event('error'));
    await delay(400);
    img.dispatchEvent(new Event('error'));
    await flush();
    expect(img.dataset.thumbBroken).toBe('true');

    // 同じ <img> を F に使い回し、F もエラー → 新エピソードとして broken を解除。
    img.src = 'https://cdn.example/thumbnails/F/F';
    img.dispatchEvent(new Event('error'));
    await flush();
    expect(img.dataset.thumbBroken).toBeUndefined();
    action.destroy();
  });
});
