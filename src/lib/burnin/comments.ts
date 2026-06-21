//! niconicomments へ渡すコメントデータ / フォント設定を組み立てる共有モジュール。
//!
//! プレイヤー (`CommentLayer.svelte`) と焼き込みエクスポートの両方がここを使う。
//! 「プレイヤーで見えているもの」と「焼き込んだ動画」を寸分違わず一致させるには、
//! niconicomments に渡す **データ形** と **config (フォント等)** を完全に共有する
//! 必要がある。独自実装を持たず、niconicomments 本体にすべての描画判断を委ねる。

import type { PlayerComment } from '../player/types';

// niconicomments のプラットフォーム判定は Linux/X11 を "other" に落として
// generic な sans-serif/serif しか指定しない。WebKitGTK の Canvas2D は
// CSS の per-glyph フォールバックを完全には行わないことがあるため、
// 和文 + 罫線（┃ ━ ┌ 等のコメ職人 AA で多用される文字）を両方持つ
// フォントを上位に置く。VL Gothic / Noto Sans CJK JP / IPA Gothic は
// 和文 + 罫線を両方持つ。末尾の DejaVu Sans / sans-serif は罫線フォールバック用の保険。
export const JP_GOTHIC =
  '"Noto Sans CJK JP", "Noto Sans JP", "Source Han Sans JP", ' +
  '"VL Gothic", "VL PGothic", "VL ゴシック", "VL Pゴシック", ' +
  '"IPAexGothic", "IPAPGothic", "IPAGothic", "IPA Pゴシック", "IPAゴシック", ' +
  '"Takao P Gothic", "Takao Gothic", ' +
  '"Hiragino Kaku Gothic ProN", "Hiragino Sans", ' +
  '"Yu Gothic UI", "Yu Gothic", YuGothic, ' +
  '"BIZ UDPGothic", "Meiryo", ' +
  '"MS PGothic", MS-PGothic, ' +
  '"DejaVu Sans", "FreeSans", ' +
  '"Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", sans-serif';
export const JP_MINCHO =
  '"Noto Serif CJK JP", "Noto Serif JP", ' +
  '"IPAexMincho", "IPAPMincho", "IPAMincho", "IPA明朝", ' +
  '"Hiragino Mincho ProN", "Yu Mincho", YuMincho, ' +
  '"MS PMincho", MS-PMincho, "DejaVu Serif", "FreeSerif", serif';

/**
 * niconicomments の `fonts` フィールドだけを上書きする config を返す。
 * 他のキー (commentDrawRange, fontSize, スケール係数 等) は niconicomments の
 * defaultConfig が浅いマージで残るので **絶対に触らない**。座標・サイズの算出は
 * すべて niconicomments 本体に任せる。
 */
export function buildConfigOverride() {
  const html5 = {
    defont: { font: JP_GOTHIC, offset: 0, weight: 600 },
    gothic: { font: JP_GOTHIC, offset: -0.04, weight: 400 },
    mincho: { font: JP_MINCHO, offset: -0.01, weight: 400 },
  };
  return {
    fonts: {
      html5,
      flash: {
        gulim: `normal 600 [size]px gulim, ${html5.gothic.font}`,
        simsun: `normal 400 [size]px simsun, batang, "PMingLiU", MingLiU-ExtB, ${html5.mincho.font}`,
      },
    },
  } as const;
}

// 制御文字や正規化前の結合文字で niconicomments の文字計測が崩れる/
// 豆腐化することがあるので、Canvas に渡す直前で軽く整形する。
// ・NFC 正規化（結合文字をプリコンポーズ）
// ・C0/C1/DEL 制御文字（改行とタブ以外）を除去
// ・行区切り(U+2028)/段落区切り(U+2029) を改行に統一
// ・孤立サロゲートを除去
// eslint-disable-next-line no-control-regex
const RE_CONTROL = /[\x00-\x08\x0B\x0C\x0E-\x1F\x7F-\x9F]/g;
const RE_LINESEP = new RegExp('[' + String.fromCharCode(0x2028, 0x2029) + ']', 'g');
const RE_LONE_HIGH = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])/g;
const RE_LONE_LOW = /(^|[^\uD800-\uDBFF])[\uDC00-\uDFFF]/g;

export function sanitizeContent(raw: string): string {
  if (!raw) return '';
  let s: string;
  try {
    s = raw.normalize('NFC');
  } catch {
    s = raw;
  }
  s = s.replace(RE_CONTROL, '');
  s = s.replace(RE_LINESEP, '\n');
  s = s.replace(RE_LONE_HIGH, '').replace(RE_LONE_LOW, '$1');
  return s;
}

/** niconicomments v1 入力の 1 コメント。 */
export type V1Comment = {
  id: string;
  no: number;
  vposMs: number;
  body: string;
  commands: string[];
  userId: string;
  isPremium: boolean;
  score: number;
  postedAt: string;
  nicoruCount: number;
  nicoruId: string | null;
  source: string;
  isMyPost: boolean;
};

/** niconicomments v1 入力の 1 スレッド (fork)。 */
export type V1Thread = {
  id: string;
  fork: string;
  commentCount: number;
  comments: V1Comment[];
};

/**
 * `postedAt` を ISO 8601 へ正規化する。
 *
 * niconicomments の v1 パーサは `postedAt` を `Date.parse()` で解釈し、ISO 8601 を
 * 前提にしている (flash/html5 のモード判定に投稿日時を使う)。ところがローカル
 * スナップショットのコメントは `posted_at` を **Unix 秒の文字列** (`"1170000000"`)
 * として返すため、そのまま渡すと `Date.parse` が NaN になり、古いコメントの
 * モード判定が壊れる (0.2.x は html5 に誤判定、0.3.x はコメントごと破棄)。
 * 全数字なら Unix 秒とみなして ISO へ変換する。既に ISO ならそのまま通す。
 */
export function toIsoPostedAt(raw: string | undefined): string {
  if (!raw) return '';
  if (/^\d+$/.test(raw)) {
    const d = new Date(Number(raw) * 1000);
    return Number.isNaN(d.getTime()) ? '' : d.toISOString();
  }
  return raw;
}

function toV1Comment(c: PlayerComment): V1Comment {
  return {
    id: c.id,
    no: c.no,
    vposMs: c.vposMs,
    body: sanitizeContent(c.content),
    commands: c.commands,
    userId: c.userId ?? '',
    isPremium: false,
    score: c.score ?? 0,
    postedAt: toIsoPostedAt(c.postedAt),
    nicoruCount: c.nicoruCount ?? 0,
    nicoruId: null,
    source: 'leaf',
    isMyPost: false,
  };
}

/**
 * `PlayerComment[]` を niconicomments の v1 (`V1Thread[]`) 形式へ変換する。
 * fork ごとにスレッドへまとめる (owner / main / easy)。プレイヤーと完全に
 * 同じ変換を使うことで、表示と焼き込みの差異をなくす。
 */
export function toV1Threads(comments: PlayerComment[]): V1Thread[] {
  const byFork = new Map<string, V1Comment[]>();
  for (const c of comments) {
    const fork = c.fork || 'main';
    const arr = byFork.get(fork) ?? [];
    arr.push(toV1Comment(c));
    byFork.set(fork, arr);
  }
  return Array.from(byFork.entries()).map(([fork, arr]) => ({
    id: fork,
    fork,
    commentCount: arr.length,
    comments: arr,
  }));
}
