import { describe, expect, test } from 'vitest';

import { buildNiconiOptions, computeTotalFrames, runFrameLoop, type FrameSink } from './core';

/** timeline と drawCanvas を持つ最小の nico スタブ。 */
function fakeNico(timeline: Record<number, unknown[]>) {
  const drawn: number[] = [];
  const nico = {
    timeline,
    drawCanvas: (vpos: number) => {
      drawn.push(vpos);
      return true;
    },
  };
  return { nico, drawn };
}

/**
 * 記録用 sink。`closeAt` を渡すと、その通算フレーム index で `false` を返して
 * 「ffmpeg が stdin を閉じた (= 必要分を読み終えた)」状況を再現する。
 */
const recordingSink = (closeAt = Infinity) => {
  const calls: Array<{ kind: 'frame' | 'empty'; index: number }> = [];
  let n = 0;
  const sink: FrameSink = {
    async frame(index) {
      if (n >= closeAt) return false;
      n++;
      calls.push({ kind: 'frame', index });
      return true;
    },
    async empty(index) {
      if (n >= closeAt) return false;
      n++;
      calls.push({ kind: 'empty', index });
      return true;
    },
  };
  return { sink, calls };
};

describe('computeTotalFrames', () => {
  test('convert 式 ceil(to - ss) * fps', () => {
    expect(computeTotalFrames(30, 10)).toBe(300);
    expect(computeTotalFrames(30, 10.2)).toBe(330); // ceil(10.2)=11
    expect(computeTotalFrames(30, 10, 2)).toBe(240); // ceil(8)=8
    expect(computeTotalFrames(30, 100, 0, 5)).toBe(150);
  });

  test('fps 0 は 30 にフォールバック', () => {
    expect(computeTotalFrames(0, 10)).toBe(300);
  });
});

describe('runFrameLoop', () => {
  test('vpos = ceil(i * 100 / fps) + offset (convert と一致), 空フレームは描画しない', async () => {
    const { nico, drawn } = fakeNico({}); // timeline 空 → 全フレーム empty
    const { sink, calls } = recordingSink();
    let pngCount = 0;
    const total = await runFrameLoop({
      nico: nico as never,
      toPng: async () => {
        pngCount++;
        return new Uint8Array();
      },
      fps: 10,
      durationSec: 1,
      sink,
    });
    expect(total).toBe(10);
    expect(calls.map((c) => c.kind)).toEqual(Array(10).fill('empty'));
    expect(calls.map((c) => c.index)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    expect(drawn).toEqual([]);
    expect(pngCount).toBe(0); // 空フレームでは toPng を呼ばない
  });

  test('コメントが存在する vpos のみ drawCanvas + frame、それ以外は empty', async () => {
    // fps=10 → 第0秒の vpos は i*10。コメントを vpos 20 と 50 に置く。
    const { nico, drawn } = fakeNico({ 20: [{}], 50: [{}] });
    const { sink, calls } = recordingSink();
    let pngCount = 0;
    await runFrameLoop({
      nico: nico as never,
      toPng: async () => {
        pngCount++;
        return new Uint8Array([pngCount]);
      },
      fps: 10,
      durationSec: 1,
      sink,
    });
    expect(drawn).toEqual([20, 50]);
    const frameIdx = calls.filter((c) => c.kind === 'frame').map((c) => c.index);
    expect(frameIdx).toEqual([2, 5]);
    expect(pngCount).toBe(2); // 描画フレームだけ PNG 化
  });

  test('offset は 1 秒ごとに 100 進む', async () => {
    // fps=2, duration=2s → 4 フレーム。vpos = 0,50,100,150。
    const { nico, drawn } = fakeNico({ 0: [{}], 50: [{}], 100: [{}], 150: [{}] });
    const { sink } = recordingSink();
    await runFrameLoop({
      nico: nico as never,
      toPng: async () => new Uint8Array(),
      fps: 2,
      durationSec: 2,
      sink,
    });
    expect(drawn).toEqual([0, 50, 100, 150]);
  });

  test('ssSec / toSec が初期 offset と総フレーム数を決める', async () => {
    // ss=1, to=2, fps=2 → 2 フレーム、offset=100 → vpos 100,150。
    const { nico, drawn } = fakeNico({ 100: [{}], 150: [{}] });
    const { sink } = recordingSink();
    const total = await runFrameLoop({
      nico: nico as never,
      toPng: async () => new Uint8Array(),
      fps: 2,
      durationSec: 5,
      ssSec: 1,
      toSec: 2,
      sink,
    });
    expect(total).toBe(2);
    expect(drawn).toEqual([100, 150]);
  });

  test('shouldAbort でループを中断する', async () => {
    const { nico } = fakeNico({});
    let count = 0;
    const sink: FrameSink = {
      async frame() {
        return true;
      },
      async empty() {
        count++;
        return true;
      },
    };
    const total = await runFrameLoop({
      nico: nico as never,
      toPng: async () => new Uint8Array(),
      fps: 30,
      durationSec: 10,
      sink,
      shouldAbort: () => count >= 3,
    });
    expect(count).toBe(3);
    expect(total).toBe(3);
  });

  test('sink が false を返したら (ffmpeg が stdin を閉じた) 送出を止める', async () => {
    // 末尾の余剰フレーム再現: 10 フレーム要求のうち 6 枚目で sink が閉じる。
    // 余剰分は送らずに止まり、生成数は閉じる直前まで (= 5)。
    const { nico } = fakeNico({}); // 全 empty
    const { sink, calls } = recordingSink(5);
    const total = await runFrameLoop({
      nico: nico as never,
      toPng: async () => new Uint8Array(),
      fps: 10,
      durationSec: 1, // 要求 10 フレーム
      sink,
    });
    // 5 枚受理 → 6 枚目で false → 即停止。残り 4 枚は送らない。
    expect(calls.length).toBe(5);
    expect(total).toBe(5);
  });
});

describe('buildNiconiOptions', () => {
  test('convert の既定 (v1 / default / scale 1 / lazy false) とフォント config', () => {
    const o = buildNiconiOptions();
    expect(o.format).toBe('v1');
    expect(o.mode).toBe('default');
    expect(o.scale).toBe(1);
    expect(o.lazy).toBe(false);
    expect(o.config).toBeDefined();
  });

  test('scale / mode を上書きできる', () => {
    const o = buildNiconiOptions({ scale: 1.5, mode: 'html5' });
    expect(o.scale).toBe(1.5);
    expect(o.mode).toBe('html5');
  });
});
