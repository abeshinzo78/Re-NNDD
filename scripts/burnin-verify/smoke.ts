import { writeFileSync } from 'node:fs';

import { createCanvas, GlobalFonts, Canvas, Path2D, Image } from '@napi-rs/canvas';
import NiconiComments from '@xpadev-net/niconicomments';

import { buildNiconiOptions } from '../../src/lib/burnin/core';
import { toV1Threads } from '../../src/lib/burnin/comments';
import type { PlayerComment } from '../../src/lib/player/types';

GlobalFonts.loadSystemFonts();
console.log('fonts loaded:', GlobalFonts.families.length);

// niconicomments は描画対象が HTMLCanvasElement か (instanceof) を見るため、
// napi-canvas の Canvas をグローバルへ差し込む。document も最小スタブを置く。
const g = globalThis as Record<string, unknown>;
g.HTMLCanvasElement = Canvas;
g.Path2D = Path2D;
g.Image = Image;
g.window = {
  setTimeout: (fn: (...a: unknown[]) => void, ms?: number) => setTimeout(fn, ms),
  clearTimeout: (id: unknown) => clearTimeout(id as ReturnType<typeof setTimeout>),
  devicePixelRatio: 1,
};
g.document = {
  createElement: (tag: string) => (tag === 'canvas' ? createCanvas(1, 1) : {}),
  fonts: { ready: Promise.resolve() },
};
g.OffscreenCanvas = class {
  width: number;
  height: number;
  private c: Canvas;
  constructor(w: number, h: number) {
    this.width = w;
    this.height = h;
    this.c = createCanvas(w, h);
  }
  getContext(type: string) {
    return this.c.getContext(type as '2d');
  }
};

const W = 1920;
const H = 1080;
const canvas = createCanvas(W, H);

const comments: PlayerComment[] = [
  {
    id: '1',
    no: 1,
    vposMs: 200,
    content: '流れるコメントのテスト',
    mail: '',
    commands: [],
    fork: 'main',
    isOwner: false,
  },
  {
    id: '2',
    no: 2,
    vposMs: 300,
    content: '赤い大きい弾幕ｗｗｗ',
    mail: 'red big',
    commands: ['red', 'big'],
    fork: 'main',
    isOwner: false,
  },
  {
    id: '3',
    no: 3,
    vposMs: 400,
    content: '上固定コメント',
    mail: 'ue',
    commands: ['ue'],
    fork: 'main',
    isOwner: false,
  },
  {
    id: '4',
    no: 4,
    vposMs: 400,
    content: '下固定の青コメント',
    mail: 'shita blue',
    commands: ['shita', 'blue'],
    fork: 'main',
    isOwner: false,
  },
  {
    id: '5',
    no: 5,
    vposMs: 500,
    content: 'small white naka',
    mail: 'small',
    commands: ['small'],
    fork: 'main',
    isOwner: false,
  },
];

const opts = buildNiconiOptions({ format: 'v1', mode: 'default', scale: 1 });
console.log(
  'options:',
  JSON.stringify({ format: opts.format, mode: opts.mode, scale: opts.scale, lazy: opts.lazy }),
);

const nico = new NiconiComments(canvas as never, toV1Threads(comments) as never, opts as never);
const timelineKeys = Object.keys(
  (nico as unknown as { timeline: Record<number, unknown[]> }).timeline,
);
console.log('timeline key count:', timelineKeys.length, 'sample:', timelineKeys.slice(0, 10));

// vpos = 0.9s * 100 = 90 (全コメントが出ているはず)
nico.drawCanvas(90);
writeFileSync('/tmp/burnin-verify/smoke.png', canvas.toBuffer('image/png'));
console.log('wrote smoke.png');
