//! 焼き込みエクスポートのフレーム生成コア。環境非依存。
//!
//! niconicomments-convert の `src/renderer/renderer.ts` のフレームループを
//! 忠実に移植したもの。**vpos の算出・空フレーム判定・描画タイミングを
//! convert と 1:1 で一致させる** ことが目的。描画そのものは niconicomments
//! 本体 (`drawCanvas`) に完全に委ねる。独自のレイアウト計算は一切しない。
//!
//! 本番 (ブラウザ WebView) と検証ハーネス (Node) の双方がこのループを共有し、
//! canvas 実装と PNG 化と「フレームの送り先 (sink)」だけを環境ごとに差し替える。

import type NiconiComments from '@xpadev-net/niconicomments';

import { buildConfigOverride } from './comments';

/** niconicomments の描画モード。`default` は flash/html5 をコメントごとに自動判定。 */
export type CommentMode = 'default' | 'html5' | 'flash';

export type NiconiOptionParams = {
  /** 入力フォーマット。Re:NNDD は v1 で統一。 */
  format?: 'v1' | 'default' | 'legacy' | 'owner' | 'formatted' | 'legacyOwner';
  /** 描画モード。convert 既定は `default`。 */
  mode?: CommentMode;
  /** コメント全体のスケール (= UI のフォント倍率)。niconicomments の scale。 */
  scale?: number;
};

/**
 * `new NiconiComments(canvas, data, options)` に渡すオプションを組み立てる。
 * convert と同じ既定 (mode=default, scale=1, format=v1) に、プレイヤーと共有の
 * フォント config を重ねる。
 */
export function buildNiconiOptions(p: NiconiOptionParams = {}) {
  return {
    format: p.format ?? 'v1',
    mode: p.mode ?? 'default',
    scale: p.scale ?? 1,
    config: buildConfigOverride(),
    // lazy=false: コンストラクタで timeline を完全に確定させる。これにより
    // 後段の「timeline[vpos] が空か」の判定が確実になり、遅延解決による
    // コメント取りこぼしを防ぐ (convert は lazy=true だが、正確性を優先する)。
    lazy: false,
  };
}

/**
 * 生成フレームの送り先。順序どおりに await される前提 (= バックプレッシャ)。
 * - `frame`: コメントが存在し描画されたフレームの PNG。
 * - `empty`: コメントが無い透明フレーム。実バイトは sink 側が持つ透明 PNG を
 *   使い回す (本番は Rust 側キャッシュ、Node は手元の透明 PNG)。同一の透明
 *   PNG を毎フレーム転送しないための分割。
 */
export interface FrameSink {
  frame(frameIndex: number, png: Uint8Array): Promise<void>;
  empty(frameIndex: number): Promise<void>;
}

export type FrameLoopParams = {
  nico: NiconiComments;
  /** 現在の canvas を PNG バイト列へ変換する (環境依存)。 */
  toPng: () => Promise<Uint8Array>;
  /** フレームレート (既定 30)。 */
  fps: number;
  /** 動画長 (秒)。 */
  durationSec: number;
  /** 開始位置 (秒, 既定 0)。 */
  ssSec?: number;
  /** 終了位置 (秒, 既定 durationSec)。 */
  toSec?: number;
  /** 生成した各フレームの送り先。 */
  sink: FrameSink;
  /** 進捗通知 (rendered/total フレーム)。 */
  onProgress?: (rendered: number, total: number) => void;
  /** true を返すと中断する。 */
  shouldAbort?: () => boolean;
  /** 何フレームごとにイベントループへ譲るか (UI 応答性確保, 既定 fps)。 */
  yieldEvery?: number;
  /** イベントループへ譲る関数 (既定: マイクロタスク)。 */
  yieldFn?: () => Promise<void>;
};

type TimelineHost = { timeline: Record<number, unknown[] | undefined> };

/**
 * 総フレーム数を convert と同じ式で算出する。
 * `ceil((to ?? duration) - ss) * fps`。
 */
export function computeTotalFrames(
  fps: number,
  durationSec: number,
  ssSec = 0,
  toSec?: number,
): number {
  const rate = fps || 30;
  const end = toSec ?? durationSec;
  return Math.max(0, Math.ceil(end - ssSec) * rate);
}

/**
 * フレームループ本体。convert の `process()` を素直なループに展開したもの。
 *
 * 1 秒ぶん (= fps フレーム) ごとに offset を 100 (センチ秒) 進め、各フレームの
 * vpos を `ceil(i * (100 / fps)) + offset` で求める。timeline にコメントが無い
 * フレームは透明 PNG (emptyPng) を送り、ある場合のみ `drawCanvas(vpos)` して
 * canvas を PNG 化して送る。
 *
 * @returns 生成した総フレーム数。
 */
export async function runFrameLoop(p: FrameLoopParams): Promise<number> {
  const rate = p.fps || 30;
  const ss = p.ssSec ?? 0;
  const totalFrames = computeTotalFrames(rate, p.durationSec, ss, p.toSec);
  const timeline = (p.nico as unknown as TimelineHost).timeline;
  const yieldEvery = Math.max(1, p.yieldEvery ?? rate);
  const yieldFn = p.yieldFn ?? (() => Promise.resolve());

  let offset = Math.ceil(ss * 100);
  let generated = 0;

  while (generated < totalFrames) {
    for (let i = 0; i < rate; i++) {
      if (p.shouldAbort?.()) return generated;
      const frame = generated;
      const vpos = Math.ceil(i * (100 / rate)) + offset;
      const atVpos = timeline[vpos];
      if (!atVpos || atVpos.length === 0) {
        await p.sink.empty(frame);
      } else {
        p.nico.drawCanvas(vpos);
        const png = await p.toPng();
        await p.sink.frame(frame, png);
      }
      generated++;
      if (generated % yieldEvery === 0) {
        p.onProgress?.(generated, totalFrames);
        await yieldFn();
      }
      if (generated >= totalFrames) break;
    }
    offset += 100;
  }

  p.onProgress?.(generated, totalFrames);
  return generated;
}
