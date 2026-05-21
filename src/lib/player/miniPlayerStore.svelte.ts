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
  /** NG ルール適用済み (= 実際に Player に渡す表示用) のコメント。 */
  comments = $state<PlayerComment[]>([]);
  /** NG ルール適用前のソース・オブ・トゥルース。NG ルールが PiP 中に変更
   *  された時、ここから再フィルタする (codex r3283322745)。 */
  rawComments = $state<PlayerComment[]>([]);
  resumePosition = $state(0);
  expandHref = $state('/');
  loop = $state(false);
  /** mini 側の最新 currentTime (秒)。expand 時に resume へ反映する。 */
  currentTime = $state(0);
  /** ミニプレイヤー領域の位置/サイズ */
  geometry = $state<MiniGeometry>({ x: 0, y: 0, width: DEFAULT_WIDTH });
  /** mini が実際に音声出力を担当しているか。
   *  PiP 起動時はまずページ側の Player が鳴り続け、mini は無音でロードする。
   *  mini が再生開始 (playing) して引き継ぎ可能になったら true にする。
   *  ページ側はこのフラグを見てプレースホルダ ↔ Player を切り替える。 */
  audioOwned = $state(false);
  /** PiP 起動時点でページ側の Player が再生中だったか。
   *  true の時のみ mini は無音ロード → 引き継ぎフローを走らせる。
   *  false (= 一時停止中の PiP 化) は引き継ぎ不要なので audioOwned を即 true にする。 */
  wasPlaying = $state(false);
  /** 引き継ぎ用のソースページ側 currentTime。ページが再生中の間継続的に更新し、
   *  mini が引き継ぐ瞬間にこの値へシークすれば「ロード時間ぶんの巻き戻し」音声を防げる。 */
  handoffTime = $state(0);
  /** 引き継ぎ中のソースページ側 Player の最新 paused 状態。
   *  ユーザが PiP 起動後、mini の引き継ぎ完了前にソース側で停止した場合、mini も
   *  停止状態で引き継いで「停止したい」意図を尊重する。引き継ぎ完了 (audioOwned)
   *  後は更新を停止する。 */
  sourcePaused = $state(false);
  /** 直前の source 設定が `replaceSource` (= キュー advance) 由来かを示す。
   *  Player 側で `playback.autoplay=false` 設定でも連続再生を行うフラグ。
   *  `open()` でリセットされる (初回 PiP は autoplay 設定に従う)。 */
  replacedFromQueue = $state(false);
  /** 初期化済みか (geometry を 1 度 localStorage からロードしたか) */
  private hydrated = false;
  /** close() 時に退避した復帰先情報。ページ側が consume して PiP 前の位置に復元する。 */
  private _returnVideoId = '';
  private _returnPosition = 0;

  /** ブラウザ側でのみ呼ぶ — 初回 open 時などに lazy 初期化 */
  hydrate() {
    if (this.hydrated) return;
    this.hydrated = true;
    this.geometry = loadGeometry();
  }

  open(args: {
    source: MiniSource;
    title: string;
    /** Player に渡す表示用 (NG 適用済み)。 */
    comments: PlayerComment[];
    /** NG 適用前。省略時は `comments` をそのまま使う。 */
    rawComments?: PlayerComment[];
    resumePosition: number;
    expandHref: string;
    loop?: boolean;
    /** ページ側 Player が再生中なら true。無音ロード → 音声引き継ぎを行う。 */
    wasPlaying?: boolean;
  }) {
    this.hydrate();
    this.source = args.source;
    this.title = args.title;
    this.comments = args.comments;
    this.rawComments = args.rawComments ?? args.comments;
    this.resumePosition = Math.max(0, args.resumePosition || 0);
    this.currentTime = this.resumePosition;
    this.expandHref = args.expandHref;
    this.loop = args.loop ?? false;
    this.wasPlaying = !!args.wasPlaying;
    this.handoffTime = this.resumePosition;
    this.sourcePaused = false;
    // 初回 open は autoplay 設定を尊重するため queue フラグはクリア。
    this.replacedFromQueue = false;
    // 再生中だった場合のみ「mini ロード完了まで音声引き継ぎ保留」。
    // 一時停止中なら音声が無いので保留する意味が無く、即時にプレースホルダへ。
    this.audioOwned = !args.wasPlaying;
    this.active = true;
  }

  /** comments のみ後追いで差し込む (取得が非同期な動画ページから).
   *  ローディング中の一過性 [] で mini を潰さないよう、呼び出し側 ($effect)
   *  が commentsSettled を true にした後でのみここを呼ぶ前提。NG ルールで
   *  全件除外された結果の [] のような「正当な空」は普通に反映する。
   *
   *  `rawComments` を省略した場合は `comments` をそのまま raw として保存
   *  する (= 既存呼び出し互換)。明示的に渡すと NG 再フィルタの元データを
   *  更新できる (codex r3283322745)。 */
  updateComments(videoId: string, comments: PlayerComment[], rawComments?: PlayerComment[]) {
    if (this.source?.videoId !== videoId) return;
    this.comments = comments;
    if (rawComments !== undefined) {
      this.rawComments = rawComments;
    }
  }

  /** PiP 中に連続再生キューが次へ進む時に呼ぶ。`open()` と違って引き継ぎ
   *  (`wasPlaying` → `audioOwned` フロー) は走らせない: 既に audio を持って
   *  いる mini が、自分の中で動画だけ差し替える操作のため。
   *
   *  `loop` は item ごとに変わる (キュー末尾は always_loop を再尊重) ので
   *  毎回更新する。`open()` 時の値だけだと「キュー末尾でループ復帰しない」
   *  バグになる。 */
  replaceSource(args: {
    source: MiniSource;
    title: string;
    expandHref: string;
    resumePosition?: number;
    /** NG ルール適用済みの表示用コメント。 */
    comments?: PlayerComment[];
    /** NG 適用前。省略時は `comments` をそのまま raw として使う。 */
    rawComments?: PlayerComment[];
    loop?: boolean;
  }) {
    this.source = args.source;
    this.title = args.title;
    this.expandHref = args.expandHref;
    this.resumePosition = Math.max(0, args.resumePosition || 0);
    this.currentTime = this.resumePosition;
    this.handoffTime = this.resumePosition;
    this.comments = args.comments ?? [];
    this.rawComments = args.rawComments ?? args.comments ?? [];
    if (args.loop != null) this.loop = args.loop;
    // mini は既に audio を持っている。Player を {#key videoId} で remount
    // するので新しい動画は initialMuted=false の通常パスで自動再生開始する。
    this.audioOwned = true;
    this.wasPlaying = false;
    this.sourcePaused = false;
    // 直前の遷移がキュー advance だったかのフラグ。MiniPlayer は
    // forceAutoplay にこれを使って `playback.autoplay=false` でも次の動画を
    // 自動再生する (ユーザの明示的な連続再生意図を優先)。
    this.replacedFromQueue = true;
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

  /** mini が音声を引き継いだことを宣言。ページ側はこれを受けて Player を破棄し
   *  プレースホルダへ切り替える。引き継ぎ完了後は handoffTime の更新を停止する
   *  ため `audioOwned` 中のセットは無視 (`setHandoffTime`) する設計。 */
  acquireAudio() {
    this.audioOwned = true;
  }

  /** ソースページ側 Player の最新 currentTime を書き込む。引き継ぎ前のみ有効。 */
  setHandoffTime(t: number) {
    if (this.audioOwned) return;
    if (!Number.isFinite(t) || t < 0) return;
    this.handoffTime = t;
  }

  /** ソースページ側 Player の最新 paused 状態を書き込む。引き継ぎ前のみ有効。 */
  setSourcePaused(paused: boolean) {
    if (this.audioOwned) return;
    this.sourcePaused = !!paused;
  }

  /** ページ側が PiP からの復帰位置を取得する。呼び出しで消費される。 */
  consumeReturnPosition(videoId: string): number {
    if (this._returnVideoId === videoId && this._returnPosition > 0) {
      const pos = this._returnPosition;
      this._returnVideoId = '';
      this._returnPosition = 0;
      return pos;
    }
    return 0;
  }

  close() {
    if (this.source && this.currentTime > 0) {
      this._returnVideoId = this.source.videoId;
      this._returnPosition = this.currentTime;
    } else {
      this._returnVideoId = '';
      this._returnPosition = 0;
    }
    this.active = false;
    this.source = null;
    this.comments = [];
    this.rawComments = [];
    this.title = '';
    this.resumePosition = 0;
    this.currentTime = 0;
    this.audioOwned = false;
    this.wasPlaying = false;
    this.handoffTime = 0;
    this.sourcePaused = false;
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
