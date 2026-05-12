// グローバルなミニプレイヤー (PiP) ステート。
//
// 設計:
//   - ミニプレイヤーは `+layout.svelte` に常駐し、ルート遷移を跨いで
//     再生を継続する。
//   - 元の再生ページ (video/[id] や library/[id]) が PiP ボタンを押した時、
//     `openMiniPlayer()` で状態を流し込み、ページ側は同じ動画 ID の場合
//     プレースホルダ表示に切り替える (二重再生防止)。
//   - 復帰時は `closeMiniPlayer()` → ページに goto。`resume:${id}` は
//     ミニ側でも継続的に書き込んでいるので、ページ再 mount で継ぎ目なく
//     再開できる。
//   - 位置/サイズは localStorage に保存する。

import type { PlayerComment } from './types';

export type MiniSource =
  | {
      kind: 'online';
      videoId: string;
      hlsUrl: string;
      refreshHlsUrl?: () => Promise<string>;
    }
  | {
      kind: 'local';
      videoId: string;
      localSrc: string;
      localAudioSrc?: string;
    };

export type MiniGeometry = {
  /** 画面左上からの x (px) */
  x: number;
  /** 画面左上からの y (px) */
  y: number;
  /** プレイヤー本体の幅 (px)。高さは 16:9 から自動 */
  width: number;
};

const GEOM_STORAGE_KEY = 'miniPlayer.geometry.v1';
const DEFAULT_WIDTH = 360;
const MIN_WIDTH = 240;
const MAX_WIDTH = 720;
const MARGIN = 20;
const ASPECT_RATIO = 16 / 9;

function loadGeometry(): MiniGeometry {
  if (typeof window === 'undefined') {
    return { x: 0, y: 0, width: DEFAULT_WIDTH };
  }
  try {
    const raw = localStorage.getItem(GEOM_STORAGE_KEY);
    if (raw) {
      const v = JSON.parse(raw) as Partial<MiniGeometry>;
      const w = clampWidth(Number(v.width) || DEFAULT_WIDTH);
      const h = w / ASPECT_RATIO;
      const fallbackX = Math.max(MARGIN, window.innerWidth - w - MARGIN);
      const fallbackY = Math.max(MARGIN, window.innerHeight - h - MARGIN);
      const rx = typeof v.x === 'number' && Number.isFinite(v.x) ? v.x : fallbackX;
      const ry = typeof v.y === 'number' && Number.isFinite(v.y) ? v.y : fallbackY;
      return {
        width: w,
        x: clamp(rx, MARGIN, fallbackX),
        y: clamp(ry, MARGIN, fallbackY),
      };
    }
  } catch {
    /* ignore */
  }
  const width = DEFAULT_WIDTH;
  const height = width / ASPECT_RATIO;
  return {
    width,
    x: Math.max(MARGIN, window.innerWidth - width - MARGIN),
    y: Math.max(MARGIN, window.innerHeight - height - MARGIN),
  };
}

export function clampWidth(w: number): number {
  return Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, Math.round(w)));
}

export function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

export function snapGeometry(g: MiniGeometry, vw: number, vh: number): MiniGeometry {
  const height = g.width / ASPECT_RATIO;
  // 画面の四隅のうち、最も近い角へスナップする。
  const cx = g.x + g.width / 2;
  const cy = g.y + height / 2;
  const leftSide = cx < vw / 2;
  const topSide = cy < vh / 2;
  return {
    width: g.width,
    x: leftSide ? MARGIN : Math.max(MARGIN, vw - g.width - MARGIN),
    y: topSide ? MARGIN : Math.max(MARGIN, vh - height - MARGIN),
  };
}

export function saveGeometry(g: MiniGeometry) {
  try {
    localStorage.setItem(GEOM_STORAGE_KEY, JSON.stringify(g));
  } catch {
    /* ignore */
  }
}

class MiniPlayerStore {
  active = $state(false);
  source = $state<MiniSource | null>(null);
  title = $state('');
  comments = $state<PlayerComment[]>([]);
  resumePosition = $state(0);
  expandHref = $state('/');
  loop = $state(false);
  /** mini 側の最新 currentTime (秒)。expand 時に resume へ反映する。 */
  currentTime = $state(0);
  /** ミニプレイヤー領域の位置/サイズ */
  geometry = $state<MiniGeometry>({ x: 0, y: 0, width: DEFAULT_WIDTH });
  /** 初期化済みか (geometry を 1 度 localStorage からロードしたか) */
  private hydrated = false;

  /** ブラウザ側でのみ呼ぶ — 初回 open 時などに lazy 初期化 */
  hydrate() {
    if (this.hydrated) return;
    this.hydrated = true;
    this.geometry = loadGeometry();
  }

  open(args: {
    source: MiniSource;
    title: string;
    comments: PlayerComment[];
    resumePosition: number;
    expandHref: string;
    loop?: boolean;
  }) {
    this.hydrate();
    this.source = args.source;
    this.title = args.title;
    this.comments = args.comments;
    this.resumePosition = Math.max(0, args.resumePosition || 0);
    this.currentTime = this.resumePosition;
    this.expandHref = args.expandHref;
    this.loop = args.loop ?? false;
    this.active = true;
  }

  /** comments のみ後追いで差し込む (取得が非同期な動画ページから) */
  updateComments(videoId: string, comments: PlayerComment[]) {
    if (this.source?.videoId === videoId) {
      this.comments = comments;
    }
  }

  setGeometry(g: MiniGeometry) {
    this.geometry = g;
    saveGeometry(g);
  }

  setCurrentTime(t: number) {
    if (Number.isFinite(t) && t >= 0) {
      this.currentTime = t;
    }
  }

  close() {
    this.active = false;
    this.source = null;
    this.comments = [];
    this.title = '';
    this.resumePosition = 0;
    this.currentTime = 0;
  }
}

export const miniPlayer = new MiniPlayerStore();

export const MINI_CONSTANTS = {
  MIN_WIDTH,
  MAX_WIDTH,
  MARGIN,
  ASPECT_RATIO,
  DEFAULT_WIDTH,
};
