/**
 * シークバーのシーンサムネイル (ストーリーボード) の位置計算。
 *
 * ストーリーボードは `columns × rows` 枚のコマを `intervalMs` おきに並べた
 * スプライト画像の列で、各画像の `timestampMs` がその先頭コマの時刻。ある時刻の
 * コマは「その時刻以前に始まる最後の画像」の中で
 * `floor((t - timestamp) / interval)` 番目 (左上から行優先) になる。
 */

import type { Storyboard } from '$lib/api';

export type StoryboardFrame = {
  /** スプライト画像の URL */
  url: string;
  /** スプライト内でのコマの左上 (元の寸法の px) */
  x: number;
  y: number;
  width: number;
  height: number;
  /** スプライト全体の寸法 (元の寸法の px) */
  sheetWidth: number;
  sheetHeight: number;
};

/**
 * `timeSec` に当たるコマ。データが無い・壊れている・時刻が数値でなければ `null`。
 * `durationSec` を渡すと、最後の (コマが埋まりきっていない) 画像で動画の長さを
 * 越えた空白のコマを指さないようにする。
 */
export function storyboardFrameAt(
  sb: Storyboard | null | undefined,
  timeSec: number,
  durationSec?: number,
): StoryboardFrame | null {
  if (!sb || sb.images.length === 0 || !Number.isFinite(timeSec)) return null;
  const { columns, rows, intervalMs, thumbnailWidth: w, thumbnailHeight: h } = sb;
  if (!(columns > 0 && rows > 0 && intervalMs > 0 && w > 0 && h > 0)) return null;

  let ms = Math.max(0, timeSec * 1000);
  if (durationSec != null && Number.isFinite(durationSec) && durationSec > 0) {
    // 最後のコマは動画の終端より前に始まる。
    ms = Math.min(ms, Math.max(0, durationSec * 1000 - 1));
  }

  let sheet = sb.images[0];
  for (const img of sb.images) {
    if (img.timestampMs <= ms) sheet = img;
    else break;
  }

  const perSheet = columns * rows;
  const index = Math.min(
    perSheet - 1,
    Math.max(0, Math.floor((ms - sheet.timestampMs) / intervalMs)),
  );
  return {
    url: sheet.url,
    x: (index % columns) * w,
    y: Math.floor(index / columns) * h,
    width: w,
    height: h,
    sheetWidth: columns * w,
    sheetHeight: rows * h,
  };
}

/** 期限のどれだけ前に取り直すか。 */
const REFRESH_MARGIN_MS = 5 * 60 * 1000;
/** 期限が近すぎる・過ぎている時も、取り直しを連打しないよう最低これだけ待つ。 */
const MIN_REFRESH_DELAY_MS = 60 * 1000;

/**
 * 画像の署名 (`expireTime`、発行から 3 時間) が切れる少し前に取り直すまでの待ち時間
 * (ミリ秒)。期限が分からなければ `null` (取り直さない)。
 */
export function storyboardRefreshDelay(
  expireTime: string | undefined,
  nowMs: number,
): number | null {
  if (!expireTime) return null;
  const at = Date.parse(expireTime);
  if (!Number.isFinite(at)) return null;
  return Math.max(MIN_REFRESH_DELAY_MS, at - nowMs - REFRESH_MARGIN_MS);
}

/** CSS の `url(...)` に入れられる形にする (二重引用符だけ逃がせば十分)。 */
export function cssUrl(url: string): string {
  return `url("${url.replace(/"/g, '%22')}")`;
}
