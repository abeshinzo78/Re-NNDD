//! ブラウザ (Tauri WebView) 側の焼き込みオーケストレーション。
//!
//! niconicomments-convert の renderer プロセスに相当する。オフスクリーン Canvas に
//! `@xpadev-net/niconicomments` で 1 フレームずつ描画し、PNG を Rust へ raw IPC で
//! 流し込む。Rust 側 (`burnin_feed`) がそれを ffmpeg の stdin に渡し、元動画へ
//! オーバーレイした MP4 を書き出す。

import { invoke } from '@tauri-apps/api/core';
import NiconiComments from '@xpadev-net/niconicomments';

import type { PlayerComment } from '../player/types';
import { toV1Threads } from './comments';
import { buildNiconiOptions, runFrameLoop, type CommentMode, type FrameSink } from './core';

// Rust の burnin::FLAG_* と一致させること。
const FLAG_FRAME = 0;
const FLAG_EMPTY = 1;
const FLAG_SET_EMPTY = 2;

export type BurnInStart = {
  sessionId: string;
  width: number;
  height: number;
  fps: number;
  durationSec: number;
  totalFrames: number;
};

export type BurnInFinish = {
  outputPath: string;
  width: number;
  height: number;
};

export type BurnInResult = {
  outputPath: string;
  commentCount: number;
  width: number;
  height: number;
};

export type BurnInExportOptions = {
  videoId: string;
  comments: PlayerComment[];
  /** フォント倍率 = niconicomments の scale (既定 1.0)。 */
  fontScale?: number;
  /** 不透明度 0..1 (既定 1.0)。ffmpeg 合成時に適用。 */
  opacity?: number;
  /** 出力幅 (px)。省略時は元動画幅。高さは 16:9。 */
  width?: number;
  /** フレームレート (既定 30)。 */
  fps?: number;
  /** 出力先フォルダ (省略時は app_data の exports/)。 */
  outputDir?: string;
  /** 描画モード (既定 'default' = flash/html5 自動判定)。 */
  mode?: CommentMode;
  /** 進捗通知。phase は 'render' (フレーム生成) / 'encode' (ffmpeg 仕上げ)。 */
  onProgress?: (rendered: number, total: number, phase: 'render' | 'encode') => void;
  /** 中断シグナル。 */
  signal?: AbortSignal;
};

/**
 * `[u8 flag][u32 LE sidLen][sid][payload]` のバイナリフレームを作る。
 *
 * Tauri 2 の raw IPC は `ArrayBuffer` をそのまま raw body として Rust へ渡す。
 * 過不足ない専用バッファを返すので、`invoke(cmd, frame)` で送れる。
 */
function buildFrame(flag: number, sessionId: string, payload?: Uint8Array): ArrayBuffer {
  const sid = new TextEncoder().encode(sessionId);
  const payloadLen = payload ? payload.byteLength : 0;
  const buf = new Uint8Array(1 + 4 + sid.length + payloadLen);
  const dv = new DataView(buf.buffer);
  buf[0] = flag;
  dv.setUint32(1, sid.length, true); // little-endian
  buf.set(sid, 5);
  if (payload) buf.set(payload, 5 + sid.length);
  return buf.buffer;
}

/** Canvas を PNG の Uint8Array へ変換する。 */
function canvasToPng(canvas: HTMLCanvasElement): Promise<Uint8Array> {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (!blob) {
        reject(new Error('canvas.toBlob returned null'));
        return;
      }
      blob
        .arrayBuffer()
        .then((ab) => resolve(new Uint8Array(ab)))
        .catch(reject);
    }, 'image/png');
  });
}

/** イベントループへ譲って UI を更新させる。 */
function yieldToEventLoop(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

/**
 * 焼き込みエクスポートを実行する。完了すると出力 MP4 のパスを返す。
 */
export async function runBurnInExport(opts: BurnInExportOptions): Promise<BurnInResult> {
  const start = await invoke<BurnInStart>('burnin_start', {
    videoId: opts.videoId,
    options: {
      width: opts.width,
      fps: opts.fps,
      opacity: opts.opacity,
      outputDir: opts.outputDir,
    },
  });

  const sessionId = start.sessionId;
  const canvas = document.createElement('canvas');
  canvas.width = start.width;
  canvas.height = start.height;
  // WebGL2 を避けて Canvas2D を強制する (WebKitGTK では WebGL2 が不安定)。
  canvas.getContext('2d');

  let nico: NiconiComments | null = null;
  try {
    // 透明フレームを 1 度だけ Rust へ送り、空フレームで使い回す。
    const emptyPng = await canvasToPng(canvas);
    await invoke('burnin_feed', buildFrame(FLAG_SET_EMPTY, sessionId, emptyPng));

    nico = new NiconiComments(
      canvas,
      toV1Threads(opts.comments) as never,
      buildNiconiOptions({
        format: 'v1',
        mode: opts.mode ?? 'default',
        scale: opts.fontScale ?? 1,
      }) as never,
    );

    const sink: FrameSink = {
      async frame(_index, png) {
        await invoke('burnin_feed', buildFrame(FLAG_FRAME, sessionId, png));
      },
      async empty(_index) {
        await invoke('burnin_feed', buildFrame(FLAG_EMPTY, sessionId));
      },
    };

    await runFrameLoop({
      nico,
      toPng: () => canvasToPng(canvas),
      fps: start.fps,
      durationSec: start.durationSec,
      sink,
      onProgress: (rendered, total) => opts.onProgress?.(rendered, total, 'render'),
      shouldAbort: () => opts.signal?.aborted ?? false,
      yieldFn: yieldToEventLoop,
    });

    if (opts.signal?.aborted) {
      throw new Error('aborted');
    }

    opts.onProgress?.(start.totalFrames, start.totalFrames, 'encode');
    const fin = await invoke<BurnInFinish>('burnin_finish', { sessionId });
    return {
      outputPath: fin.outputPath,
      commentCount: opts.comments.length,
      width: fin.width,
      height: fin.height,
    };
  } catch (e) {
    try {
      await invoke('burnin_cancel', { sessionId });
    } catch {
      /* ignore */
    }
    throw e;
  } finally {
    try {
      (nico as unknown as { destroy?: () => void } | null)?.destroy?.();
    } catch {
      /* ignore */
    }
  }
}
